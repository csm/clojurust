// `EvalResult`'s error is returned by value from every evaluator entry point.
#![allow(clippy::result_large_err)]

//! Executor tasks that deliver their result through a Clojure `Future`.
//!
//! [`spawn_future`] is the single delivery point for every async primitive
//! (`^:async` calls, `go`, `timeout`, `alts`, and native I/O in sibling crates).
//! [`await_value`] lets Rust async code wait on a Clojure future or promise.
//! Both work on every target; the colored `eval_async` evaluator is wasm32-only.

use std::future::Future;

use cljrs_gc::GcPtr;
use cljrs_runtime::env::error::{EvalError, EvalResult};
use cljrs_value::{CljxFuture, FutureState, Value};

// Managed captures must be visible before the first task poll. Keep their
// shadow roots in a detached state so dropping a task uses the same registry.
#[cfg(not(feature = "no-gc"))]
struct CapturedRoots {
    _values: Box<[Value]>,
    state: cljrs_gc::mutator::MutatorState,
    guard: Option<cljrs_runtime::env::gc_roots::ValueRootGuard>,
}
#[cfg(not(feature = "no-gc"))]
impl CapturedRoots {
    fn new(values: Vec<Value>) -> Self {
        let values = values.into_boxed_slice();
        let mut state = cljrs_gc::mutator::MutatorState::fresh();
        state.swap_with_current();
        let guard = Some(cljrs_runtime::env::gc_roots::root_values(&values));
        state.swap_with_current();
        Self {
            _values: values,
            state,
            guard,
        }
    }
}
#[cfg(not(feature = "no-gc"))]
impl Drop for CapturedRoots {
    fn drop(&mut self) {
        self.state.swap_with_current();
        self.guard.take();
        self.state.swap_with_current();
    }
}

/// Spawn `task` on the current `LocalSet` and return a `Value::Future` that the
/// task settles on completion. The single delivery point for every async
/// primitive (`^:async` calls, `timeout`, `alts`).
///
/// Public so other native crates (e.g. `cljrs-io`) can drive their own async
/// work onto the shared executor and deliver results through the same `Future`
/// machinery.
///
/// Must be called from within a Tokio `LocalSet` context.
pub fn spawn_future<F>(task: F) -> Value
where
    F: Future<Output = EvalResult> + 'static,
{
    spawn_future_with_roots(Vec::new(), task)
}

/// Spawn a task with explicit managed captures. Use this for borrowed values
/// that can outlive their current cell binding or evaluation read scope.
pub fn spawn_future_with_roots<F>(roots: Vec<Value>, task: F) -> Value
where
    F: Future<Output = EvalResult> + 'static,
{
    // GC builds: heap-promotion fallback — the task's captured environment is
    // opaque to the publish-barrier scan, and the task may run after any
    // bump-region scope active right now has closed.  Poison the active
    // regions so they are retired (kept alive) instead of reset; a no-op (one
    // thread-local read) when no region is open, which is the common case.
    #[cfg(not(feature = "no-gc"))]
    cljrs_gc::region::poison_active_regions();
    #[cfg(feature = "no-gc")]
    let lease = {
        let mut lease = cljrs_gc::region::RegionLease::capture_active();
        lease.extend(&cljrs_gc::region::RegionLease::for_values(&roots));
        lease
    };
    #[cfg(feature = "no-gc")]
    let allocation = cljrs_gc::region::AllocationSnapshot::capture();
    let future = GcPtr::new(CljxFuture::new_local());
    let task_future = future.clone();
    #[cfg(not(feature = "no-gc"))]
    let captures = {
        let mut roots = roots;
        roots.push(Value::Future(task_future.clone()));
        CapturedRoots::new(roots)
    };
    #[cfg(feature = "no-gc")]
    let _ = roots;
    let gas_meters = cljrs_runtime::env::gas::active_meters();
    let interrupts = cljrs_runtime::env::gas::active_interrupts();
    tokio::task::spawn_local(async move {
        #[cfg(not(feature = "no-gc"))]
        let _captures = captures;
        #[cfg(feature = "no-gc")]
        let mut reads = cljrs_gc::region::ReadContext::default();
        let mut task = Box::pin(task);
        let result = std::future::poll_fn(|cx| {
            // LocalSet tasks share an OS thread, so TLS state must be scoped
            // to one poll and removed before another task can run. Meters
            // stack on the driver's; interrupt flags replace them, so this
            // task answers only to an interrupt of the eval that spawned it.
            let _interrupts = cljrs_runtime::env::gas::InterruptScope::enter(&interrupts);
            let _gas_guards = cljrs_runtime::env::gas::install_meters(&gas_meters);
            let _monitors = cljrs_runtime::env::monitor::AsyncMonitorScope::enter();
            #[cfg(feature = "no-gc")]
            let _lease_scope = lease.enter();
            #[cfg(feature = "no-gc")]
            let _allocation_scope = allocation.enter();
            #[cfg(feature = "no-gc")]
            let _reads = reads.enter();
            task.as_mut().poll(cx)
        })
        .await;
        #[cfg(feature = "no-gc")]
        let _lease_scope = lease.enter();
        #[cfg(feature = "no-gc")]
        let _allocation_scope = allocation.enter();
        settle_future(&task_future, result);
    });
    Value::Future(future)
}

/// Write a completed result into a future and wake blocking `deref` waiters.
pub(crate) fn settle_future(future: &GcPtr<CljxFuture>, result: EvalResult) {
    let state = future.get().state.lock().unwrap();
    // Cancellation is sticky. `future-cancel` does not interrupt the task, so a
    // task cancelled mid-body still runs to completion and lands here; letting
    // it write its result would flip `future-cancelled?` back to false and hand
    // `await` a value it had already promised to raise on. Cancel wins: the
    // side effects happened, the result is discarded.
    if matches!(&*state, FutureState::Cancelled) {
        return;
    }
    drop(state);
    let settled = match result {
        Ok(v) => FutureState::Done(cljrs_value::publish::publish_for_owner(
            v,
            future.get() as *const _ as usize,
        )),
        // Preserve the thrown value (and any non-Thrown error as a fresh
        // Value::Error) so `await` can re-throw it with ex-data/ex-cause intact.
        Err(EvalError::GasExhausted) => FutureState::GasExhausted,
        Err(EvalError::Cancelled) => FutureState::Interrupted,
        Err(EvalError::MemoryLimitExceeded(fault)) => FutureState::MemoryLimitExceeded(fault),
        Err(e) => FutureState::Failed(cljrs_value::publish::publish_for_owner(
            e.to_error_value(),
            future.get() as *const _ as usize,
        )),
    };
    let mut state = future.get().state.lock().unwrap();
    if matches!(&*state, FutureState::Cancelled) {
        return;
    }
    *state = settled;
    drop(state);
    future.get().cond.notify_all();
    future.get().ready.notify_all();
}

/// Cooperatively await a Clojure value. Futures and promises yield to the
/// executor until resolved, a channel (what `go` returns) is taken from, and
/// any other value is returned as-is.
pub async fn await_value(val: Value) -> EvalResult {
    #[cfg(feature = "no-gc")]
    {
        let mut reads = cljrs_gc::region::ReadContext::default();
        let mut wait = Box::pin(await_value_inner(val));
        let result = std::future::poll_fn(|cx| {
            let _reads = reads.enter();
            wait.as_mut().poll(cx)
        })
        .await;
        cljrs_runtime::interp::eval::publish_result(result, None)
    }
    #[cfg(not(feature = "no-gc"))]
    await_value_inner(val).await
}

async fn await_value_inner(val: Value) -> EvalResult {
    match val {
        Value::Future(f) => {
            // Root the future across GC cycles: the alloc frame of the scope that
            // produced it may have dropped before this task reached a yield point.
            let anchor = Value::Future(f.clone());
            let _root = cljrs_runtime::env::gc_roots::root_value(&anchor);
            loop {
                let listener = f.get().ready.listen();
                if let Some(result) = f.get().settled_value() {
                    return result.map_err(cljrs_runtime::env::error::value_error_to_eval_error);
                }
                cljrs_runtime::env::gc_roots::async_gc_collect();
                listener.await;
            }
        }
        Value::Promise(p) => {
            let anchor = Value::Promise(p.clone());
            let _root = cljrs_runtime::env::gc_roots::root_value(&anchor);
            loop {
                let listener = p.get().ready.listen();
                let value = p.get().value.lock().unwrap().clone();
                if let Some(value) = value {
                    cljrs_value::publish::read_slot(p.get() as *const _ as usize, 0);
                    return Ok(value);
                }
                cljrs_runtime::env::gc_roots::async_gc_collect();
                listener.await;
            }
        }
        other => match crate::channel::as_channel(&other) {
            // `go` returns a result channel; awaiting it takes the value.
            Some(channel) => {
                let _root = cljrs_runtime::env::gc_roots::root_value(&other);
                channel.take_for_await().await
            }
            None => Ok(other),
        },
    }
}
