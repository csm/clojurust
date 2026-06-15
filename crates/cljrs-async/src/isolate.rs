//! Per-isolate Tokio runtime — each isolate has its own GC heap, collector,
//! and `current_thread` + `LocalSet` executor that GCs independently.

use std::future::Future;

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
                let _mutator = cljrs_gc::register_mutator();
                cljrs_gc::HEAP.set_config_from_env();
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("isolate: failed to build Tokio runtime");
                let local = tokio::task::LocalSet::new();
                local.block_on(&rt, f());
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
    use std::sync::{Arc, Barrier};

    #[test]
    fn isolate_spawn_runs_to_completion() {
        let handle = Isolate::new("test-isolate").spawn(|| async {
            // Simple async work on the isolate
            tokio::task::yield_now().await;
        });
        handle.join().expect("isolate panicked");
    }

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
