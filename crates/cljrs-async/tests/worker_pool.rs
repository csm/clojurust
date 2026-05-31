//! Tests for `WorkerPool` — the `Send`-only multi-thread worker pool.
//!
//! The harness mirrors real usage: a `current_thread` runtime + `LocalSet`
//! drives the heap thread, while the pool runs `Send` tasks on separate
//! worker threads.

use cljrs_async::worker_pool::WorkerPool;
use tokio::task::LocalSet;

/// Run a `!Send` future on a current-thread LocalSet executor (mirrors how the
/// heap thread actually runs during normal clojurust operation).
fn block_on_local<F: std::future::Future>(f: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");
    let local = LocalSet::new();
    local.block_on(&rt, f)
}

// ── Basic offload ─────────────────────────────────────────────────────────────

/// A simple Send computation offloaded to the pool returns the correct result.
#[test]
fn offload_simple_send_computation() {
    let result = block_on_local(async {
        WorkerPool::global()
            .offload(async { 6u64 * 7 })
            .await
    });
    assert_eq!(result, 42u64);
}

/// `offload` works with `String` results (heap-allocated, Send).
#[test]
fn offload_returns_string() {
    let result = block_on_local(async {
        WorkerPool::global()
            .offload(async { format!("hello-{}", 99) })
            .await
    });
    assert_eq!(result, "hello-99");
}

/// A pool task that does a small async sleep still resolves correctly.
#[test]
fn offload_with_async_sleep() {
    let result = block_on_local(async {
        WorkerPool::global()
            .offload(async {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                42u32
            })
            .await
    });
    assert_eq!(result, 42u32);
}

// ── Concurrent pool tasks ─────────────────────────────────────────────────────

/// Multiple concurrent pool tasks all complete and return distinct results.
#[test]
fn multiple_concurrent_pool_tasks() {
    let results = block_on_local(async {
        let pool = WorkerPool::global();
        let futs: Vec<_> = (0u64..8)
            .map(|i| pool.offload(async move { i * i }))
            .collect();
        // Await all in order (join_all pattern via a simple loop).
        let mut out = Vec::with_capacity(8);
        for f in futs {
            out.push(f.await);
        }
        out
    });
    let expected: Vec<u64> = (0u64..8).map(|i| i * i).collect();
    assert_eq!(results, expected);
}

/// Pool tasks can themselves spawn sub-tasks via `handle()` and the results
/// arrive back on the LocalSet.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn handle_spawn_sub_tasks() {
    let result = block_on_local(async {
        let pool = WorkerPool::global();
        // Use the handle to spawn two tasks and collect via oneshot channels.
        let (tx1, rx1) = tokio::sync::oneshot::channel::<u64>();
        let (tx2, rx2) = tokio::sync::oneshot::channel::<u64>();
        pool.handle().spawn(async move { let _ = tx1.send(10); });
        pool.handle().spawn(async move { let _ = tx2.send(32); });
        let a = rx1.await.unwrap();
        let b = rx2.await.unwrap();
        a + b
    });
    assert_eq!(result, 42u64);
}

// ── LocalSet context ──────────────────────────────────────────────────────────

/// The pool is accessible from within a `spawn_local` task (the actual usage
/// scenario where a LocalSet bridge task offloads work to the pool).
#[test]
fn offload_from_spawn_local_context() {
    let result = block_on_local(async {
        let (tx, rx) = tokio::sync::oneshot::channel::<u64>();
        tokio::task::spawn_local(async move {
            let val = WorkerPool::global()
                .offload(async { 21u64 * 2 })
                .await;
            let _ = tx.send(val);
        });
        rx.await.unwrap()
    });
    assert_eq!(result, 42u64);
}

/// `WorkerPool::global()` returns the same singleton on repeated calls.
#[test]
fn global_is_singleton() {
    let p1 = WorkerPool::global() as *const WorkerPool;
    let p2 = WorkerPool::global() as *const WorkerPool;
    assert_eq!(p1, p2, "WorkerPool::global() must return the same pointer");
}

// ── Vec<u8> round-trip (the primary use case) ─────────────────────────────────

/// A pool task that processes `Vec<u8>` data (e.g. hashing/compression) and
/// returns the result as another `Vec<u8>` to the LocalSet.
#[test]
fn offload_byte_processing() {
    let input = b"hello world".to_vec();
    let output = block_on_local(async {
        WorkerPool::global()
            .offload(async move {
                // Simulate byte-level work: XOR each byte with 0xFF.
                input.iter().map(|&b| b ^ 0xFF).collect::<Vec<u8>>()
            })
            .await
    });
    let expected: Vec<u8> = b"hello world".iter().map(|&b| b ^ 0xFF).collect();
    assert_eq!(output, expected);
}
