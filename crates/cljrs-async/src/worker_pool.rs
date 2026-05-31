//! `WorkerPool` — a `Send`-only multi-thread worker pool for byte-level work.
//!
//! TCP/TLS I/O, hashing, compression, and similar CPU- or I/O-bound operations
//! that must not block the heap thread (which runs on a `LocalSet` +
//! `current_thread` runtime) are offloaded here. Results cross back to the heap
//! thread as `Send` data (`Vec<u8>`, `String`, oneshot/mpsc messages) rather
//! than as `GcPtr`/`Value`, which are `!Send`.
//!
//! # Key invariant
//!
//! Pool tasks **must never hold `GcPtr`, `Value`, or any other GC-heap data**.
//! The seam between the pool and the heap thread is:
//! - Pool → heap: `Vec<u8>`, `String`, or plain Rust primitives over oneshot/mpsc.
//! - Heap → pool: `Vec<u8>` bytes, opaque `Arc`-wrapped config, or plain values.
//!
//! All `GcPtr` construction happens on the heap thread (in LocalSet bridge tasks).
//!
//! # Usage
//!
//! ```rust,ignore
//! use cljrs_async::worker_pool::WorkerPool;
//!
//! // Offload a Send computation; await the result on the heap thread.
//! let result = WorkerPool::global().offload(async { expensive_hash() }).await;
//! ```

use std::future::Future;
use std::sync::OnceLock;

// ── Non-wasm implementation ───────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
mod inner {
    use std::future::Future;
    use tokio::runtime::{Handle, Runtime};

    /// Global `Send`-only worker pool backed by a Tokio multi-thread runtime.
    pub struct WorkerPool {
        pub(super) rt: Runtime,
    }

    impl WorkerPool {
        pub(super) fn new() -> Self {
            let parallelism = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(2);
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(parallelism)
                .thread_name("cljrs-worker")
                .enable_all()
                .build()
                .expect("cljrs-async: failed to build worker pool runtime");
            Self { rt }
        }

        /// Offload a `Send` future to the pool and return a local future the
        /// heap thread can await. The output `T` comes back over a oneshot channel
        /// so no `Send` bound is required on the return site.
        pub fn offload<F, T>(&self, f: F) -> impl Future<Output = T> + 'static
        where
            F: Future<Output = T> + Send + 'static,
            T: Send + 'static,
        {
            let (tx, rx) = tokio::sync::oneshot::channel::<T>();
            self.rt.spawn(async move {
                let result = f.await;
                let _ = tx.send(result);
            });
            async move { rx.await.expect("cljrs-async: worker pool task panicked") }
        }

        /// Direct handle to the pool runtime for multi-task spawning.
        pub fn handle(&self) -> &Handle {
            self.rt.handle()
        }
    }
}

// ── wasm32 stub ───────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
mod inner {
    use std::future::Future;

    /// Stub pool for wasm32: no real pool threads exist, futures run locally.
    pub struct WorkerPool;

    impl WorkerPool {
        pub(super) fn new() -> Self {
            Self
        }

        /// On wasm32, offload just runs the future locally (no pool threads exist).
        pub fn offload<F, T>(&self, f: F) -> impl Future<Output = T> + 'static
        where
            F: Future<Output = T> + Send + 'static,
            T: Send + 'static,
        {
            f
        }
    }
}

// ── Public re-export ──────────────────────────────────────────────────────────

/// Global `Send`-only worker pool. Use `WorkerPool::global()` to access it.
pub struct WorkerPool(inner::WorkerPool);

static POOL: OnceLock<WorkerPool> = OnceLock::new();

impl WorkerPool {
    /// Get (or initialize) the global singleton pool.
    pub fn global() -> &'static Self {
        POOL.get_or_init(|| WorkerPool(inner::WorkerPool::new()))
    }

    /// Offload a `Send` future to the pool and return a `Future` the heap thread
    /// can await without blocking the `LocalSet` executor.
    ///
    /// The returned future is `'static` but not `Send` — it may be polled from
    /// the `LocalSet` executor thread (via `spawn_local` or direct `.await`).
    pub fn offload<F, T>(&self, f: F) -> impl Future<Output = T> + 'static
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        self.0.offload(f)
    }

    /// Direct handle to the pool runtime for multi-task spawning.
    ///
    /// Use this when you need to spawn multiple tasks independently (e.g. a
    /// pool_reader + pool_writer pair) rather than `offload` a single future.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn handle(&self) -> &tokio::runtime::Handle {
        self.0.handle()
    }
}
