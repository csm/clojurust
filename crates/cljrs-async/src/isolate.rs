//! Per-isolate Tokio runtime — each isolate has its own GC heap, collector,
//! and `current_thread` + `LocalSet` executor that GCs independently.

use std::future::Future;

/// A root evaluation failure that can cross the isolate's OS-thread boundary.
/// Managed Clojure values remain on their owning thread; ordinary errors carry
/// their diagnostic text, while resource errors keep their typed details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootError {
    GasExhausted,
    MemoryLimitExceeded(cljrs_gc::MemoryLimitExceeded),
    Evaluation(String),
}
impl std::fmt::Display for RootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GasExhausted => f.write_str("gas exhausted"),
            Self::MemoryLimitExceeded(error) => error.fmt(f),
            Self::Evaluation(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for RootError {}
impl From<cljrs_runtime::env::error::EvalError> for RootError {
    fn from(error: cljrs_runtime::env::error::EvalError) -> Self {
        use cljrs_runtime::env::error::EvalError;
        match error {
            EvalError::GasExhausted => Self::GasExhausted,
            EvalError::MemoryLimitExceeded(error) => Self::MemoryLimitExceeded(error),
            other => Self::Evaluation(other.to_string()),
        }
    }
}

/// An independent execution context: its own Tokio `current_thread` runtime,
/// `LocalSet`, and per-isolate GC heap (thread-local).
///
/// Spawn one isolate per CPU-core of Clojure work. Isolates share no heap
/// pointers; the `!Send` bound on `GcPtr` makes crossing the boundary a
/// compile error. Values cross via a copy/structured-clone (Phase B2).
pub struct Isolate {
    name: String,
}

impl Isolate {
    /// Create a new isolate with the given debug name.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// Spawn this isolate on a dedicated OS thread and run `f()` as its
    /// entry-point future.
    ///
    /// `f` must be `Send + 'static` (it is moved to the new thread), but the
    /// `Future` it produces does **not** need to be `Send` — it runs entirely
    /// on the isolate's thread, where `GcPtr` and other `!Send` values are safe.
    ///
    /// The isolate thread automatically:
    /// - registers with the process memory governor under the isolate's name
    /// - calls [`cljrs_gc::register_mutator`] to initialize per-isolate GC state
    /// - calls [`cljrs_gc::HEAP.set_config_from_env`] with env-configured limits
    /// - builds a `current_thread` Tokio runtime + `LocalSet`
    /// - runs the `LocalSet` until `f()` completes, then exits
    #[cfg(not(target_arch = "wasm32"))]
    pub fn spawn<F, Fut>(self, f: F) -> std::thread::JoinHandle<()>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        std::thread::Builder::new()
            .name(self.name.clone())
            .spawn(move || {
                // Register with the process memory governor before the first
                // `GcPtr` allocation on this thread.
                cljrs_gc::register_current_isolate(&self.name);
                let _mutator = cljrs_gc::register_mutator();
                cljrs_gc::HEAP.set_config_from_env();
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("isolate: failed to build Tokio runtime");
                let local = tokio::task::LocalSet::new();
                local.block_on(&rt, f());
                drop(local);
                crate::channel::report_unobserved_thread_failures();
            })
            .expect("isolate: failed to spawn OS thread")
    }

    /// Run a synchronous entry function in a root fiber on a new isolate.
    /// Blocking Clojure waits park the root and let local tasks make progress.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn spawn_root<F>(self, body: F) -> std::thread::JoinHandle<Result<(), RootError>>
    where
        F: FnOnce() -> cljrs_runtime::env::error::EvalResult + Send + 'static,
    {
        std::thread::Builder::new()
            .name(self.name.clone())
            .spawn(move || {
                cljrs_gc::register_current_isolate(&self.name);
                let _mutator = cljrs_gc::register_mutator();
                cljrs_gc::HEAP.set_config_from_env();
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("isolate runtime");
                let local = tokio::task::LocalSet::new();
                crate::fiber::run_root_owned(&rt, local, body)
                    .map(|_| ())
                    .map_err(RootError::from)
            })
            .expect("isolate: failed to spawn OS thread")
    }

    /// wasm32 stub — wasm has a single thread; return a dummy handle.
    #[cfg(target_arch = "wasm32")]
    pub fn spawn<F, Fut>(self, _f: F) -> std::thread::JoinHandle<()>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        panic!("Isolate::spawn is not supported on wasm32")
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    #[cfg(not(feature = "no-gc"))]
    use std::sync::{Arc, Barrier};

    #[test]
    #[allow(clippy::result_large_err)]
    fn root_errors_are_join_results_instead_of_panics() {
        use cljrs_runtime::env::error::EvalError;
        let gas = Isolate::new("root-gas")
            .spawn_root(|| Err(EvalError::GasExhausted))
            .join()
            .expect("resource failures must not panic");
        assert_eq!(gas, Err(RootError::GasExhausted));
        let fault = cljrs_gc::MemoryLimitExceeded {
            class: cljrs_gc::MemoryClass::GcHeap,
            requested: 1,
            committed: 2,
            hard_limit: 2,
        };
        let memory = Isolate::new("root-memory")
            .spawn_root(move || Err(EvalError::MemoryLimitExceeded(fault)))
            .join()
            .expect("memory-limit failures must not panic");
        assert_eq!(memory, Err(RootError::MemoryLimitExceeded(fault)));
        let thrown = Isolate::new("root-throw")
            .spawn_root(|| Err(EvalError::Thrown(cljrs_value::Value::Long(42))))
            .join()
            .expect("Clojure throws must not panic");
        assert!(matches!(thrown, Err(RootError::Evaluation(_))));
    }

    #[test]
    fn isolate_spawn_runs_to_completion() {
        let handle = Isolate::new("test-isolate").spawn(|| async {
            // Simple async work on the isolate
            tokio::task::yield_now().await;
        });
        handle.join().expect("isolate panicked");
    }

    // no-gc uses static arenas and does not maintain traced heap counts.
    #[cfg(not(feature = "no-gc"))]
    #[test]
    fn two_isolates_run_concurrently() {
        let barrier = Arc::new(Barrier::new(2));

        let b1 = barrier.clone();
        let h1 = Isolate::new("iso-a").spawn(move || async move {
            // Allocate some GC values
            let _vals: Vec<_> = (0_i64..50).map(cljrs_gc::GcPtr::new).collect();
            b1.wait();
            // Isolate A's heap has exactly 50 objects
            assert_eq!(cljrs_gc::HEAP.count(), 50);
        });

        let b2 = barrier.clone();
        let h2 = Isolate::new("iso-b").spawn(move || async move {
            let _vals: Vec<_> = (0_i64..75).map(cljrs_gc::GcPtr::new).collect();
            b2.wait();
            // Isolate B's heap has exactly 75 objects — unaffected by A
            assert_eq!(cljrs_gc::HEAP.count(), 75);
        });

        h1.join().expect("isolate A panicked");
        h2.join().expect("isolate B panicked");
    }
}
