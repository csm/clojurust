//! Compiled async state machines (Phase H runtime support).
//!
//! **No runtime path uses this ABI.** Since virtual-threads Phase 5 every
//! `^:async` call runs on a fiber, and TODO.md Phase H is closed. The poll
//! ABI remains only so `cljrs-compiler`'s `lower_async` / poll-codegen unit
//! tests compile; the poll-function registry was removed. Delete this module
//! together with that transform.
//!
//! When `cljrs_ir::lower::async_lower` rewrites an `^:async` function into a
//! poll function, codegen emits it as a C-ABI *poll function* with the
//! signature
//!
//! ```text
//! extern "C" fn(*mut CljxStateMachine, *mut *const Value) -> i32
//! ```
//!
//! returning [`POLL_PENDING`] / [`POLL_READY`] / [`POLL_THREW`] /
//! [`POLL_GAS_EXHAUSTED`].  This module
//! supplies the runtime side: the [`CljxStateMachine`] that holds the resume
//! state and the spilled live values, the [`CompiledAsyncTask`] `Future`
//! adapter that drives the poll function on the existing `LocalSet` executor,
//! and the readiness helpers the poll function calls at a suspend's resume
//! point.
//!
//! A completed poll function stores its result (or thrown value) into the
//! machine's GC-rooted [`CljxStateMachine::pending`] slot — via
//! `rt_async_set_result` on `Return`, or left there by the readiness check on a
//! failed await — rather than handing back a raw pointer through an
//! out-parameter.  The adapter then reads it as a plain owned `Value`, so no
//! externally-written raw pointer is ever dereferenced on the Rust side.
//!
//! ## GC safety
//!
//! The values that cross a suspend live in [`CljxStateMachine::slots`] (a
//! fixed-length `Vec<Value>` whose backing pointer is stable for the task's
//! lifetime) and the currently-awaited value lives in
//! [`CljxStateMachine::pending`].  [`CompiledAsyncTask`] registers both with the
//! thread-local GC root stacks (`root_values` / `root_value`) and holds those
//! guards across every `.await`, so the collector — which only runs at a
//! cooperative yield when no task is mid-poll (single-threaded `LocalSet`) —
//! traces every suspended task's live values.  This reuses the exact mechanism
//! `eval_async`/`spawn_future` already rely on; there is no separate async root
//! set.  The state machine is boxed (never a `GcPtr`) so its address — and the
//! addresses the root guards capture — stay fixed even as the `Future` moves.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use cljrs_runtime::env::env::GlobalEnv;
use cljrs_runtime::env::error::{EvalError, EvalResult};
use cljrs_runtime::env::gc_roots::{ValueRootGuard, root_value, root_values};
use cljrs_value::{CljxFuture, FutureState, Value};

/// The eval context (globals + namespace) a compiled poll function runs under,
/// so its `rt_*` global-lookup / call bridges work while the task runs detached
/// on the executor.
pub type EvalCtx = (Arc<GlobalEnv>, Arc<str>);

/// Poll-function return code: the task suspended; re-poll later.
pub const POLL_PENDING: i32 = 0;
/// Poll-function return code: the task finished; the result is in `pending`.
pub const POLL_READY: i32 = 1;
/// Poll-function return code: the task threw; the thrown value is in `pending`.
pub const POLL_THREW: i32 = 2;
/// Poll-function return code: the active gas meter was exhausted.
pub const POLL_GAS_EXHAUSTED: i32 = 3;
/// Execution was cancelled independently of the gas budget.
pub const POLL_CANCELLED: i32 = 4;

/// The C-ABI signature of a compiled poll function.  The result/thrown value is
/// returned in-band (`CljxStateMachine::pending`), not through an out-pointer.
pub type PollFn = extern "C" fn(*mut CljxStateMachine) -> i32;

/// Runtime state of one compiled `^:async` invocation.
///
/// Built by the dispatcher when an `^:async` function whose arity has a compiled
/// poll function is called: the arguments are materialised into the leading
/// slots, and the machine is handed to a [`CompiledAsyncTask`].
pub struct CljxStateMachine {
    /// Resume state; `0` is the entry.  The poll function reads this in its
    /// `switch(state)` prologue and updates it before suspending.
    pub state: i32,
    /// Spilled live values.  Slots `0..param_count` start as the call
    /// arguments; the rest hold values that cross a suspend.  Fixed length —
    /// the backing pointer must stay stable for GC rooting.
    pub slots: Vec<Value>,
    /// The value (future/promise/channel) registered at the current suspend
    /// point; `nil` when not suspended.  Read by the resume readiness check.
    pub pending: Value,
    /// The compiled poll function.
    pub poll_fn: PollFn,
    /// Eval context installed on the executor thread around each poll, so the
    /// poll function's global-lookup / call bridges resolve correctly.  `None`
    /// for hand-written poll functions in tests that need no global resolution.
    pub eval_ctx: Option<EvalCtx>,
}

impl CljxStateMachine {
    /// Build a state machine for a compiled poll function, placing `args` into
    /// the leading slots and zero-filling the rest up to `n_slots`.
    pub fn new(poll_fn: PollFn, n_slots: usize, args: Vec<Value>) -> Self {
        let mut slots = vec![Value::Nil; n_slots.max(args.len())];
        for (i, a) in args.into_iter().enumerate() {
            slots[i] = a;
        }
        Self {
            state: 0,
            slots,
            pending: Value::Nil,
            poll_fn,
            eval_ctx: None,
        }
    }
}

/// Readiness of a registered (`pending`) value at a resume point.
pub enum Readiness {
    /// Not resolved yet — the poll function returns [`POLL_PENDING`].
    Pending,
    /// Resolved; the awaited value.
    Ready(Value),
    /// The awaited future failed; the thrown value.
    Failed(Value),
    /// The awaited computation exhausted its evaluation budget.
    GasExhausted,
    /// The awaited computation failed with a managed-memory limit fault.
    MemoryLimitExceeded(cljrs_gc::MemoryLimitExceeded),
}

/// Check whether an awaited value has resolved, without blocking.  Mirrors
/// [`crate::task::await_value`] but for the poll-loop (no `.await`):
/// futures/promises report their settled state; any other value is immediately
/// ready (awaiting a non-future is the identity).
pub fn check_ready(val: &Value) -> Readiness {
    match val {
        Value::Future(f) => match &*f.get().state.lock().unwrap() {
            FutureState::Done(v) => {
                cljrs_value::publish::read_slot(f.get() as *const _ as usize, 0);
                f.get().mark_observed();
                Readiness::Ready(v.clone())
            }
            FutureState::Failed(v) => {
                cljrs_value::publish::read_slot(f.get() as *const _ as usize, 0);
                f.get().mark_observed();
                Readiness::Failed(v.clone())
            }
            FutureState::Interrupted => {
                f.get().mark_observed();
                Readiness::Failed(CljxFuture::cancelled_error())
            }
            FutureState::GasExhausted => {
                f.get().mark_observed();
                Readiness::GasExhausted
            }
            FutureState::MemoryLimitExceeded(fault) => {
                f.get().mark_observed();
                Readiness::MemoryLimitExceeded(*fault)
            }
            // Same catchable error the tree-walking `await` raises, so a
            // `catch` around `(await f)` sees one shape whether or not the
            // enclosing fn was async-JIT-compiled.
            FutureState::Cancelled => Readiness::Failed(CljxFuture::cancelled_error()),
            FutureState::Running => Readiness::Pending,
        },
        Value::Promise(p) => match p.get().value.lock().unwrap().as_ref() {
            Some(v) => {
                cljrs_value::publish::read_slot(p.get() as *const _ as usize, 0);
                Readiness::Ready(v.clone())
            }
            None => Readiness::Pending,
        },
        other => Readiness::Ready(other.clone()),
    }
}

/// A `Future` that drives a compiled poll function to completion on the current
/// `LocalSet`, keeping its state machine GC-rooted while suspended.
pub struct CompiledAsyncTask {
    sm: Box<CljxStateMachine>,
    // Root guards keep `sm.slots` and `sm.pending` reachable to the collector
    // across every `.await`.  Held for the whole task; dropped on completion.
    _slots_root: ValueRootGuard,
    _pending_root: ValueRootGuard,
}

impl CompiledAsyncTask {
    /// Wrap a state machine.  The machine is boxed so its slot buffer and
    /// `pending` field have stable addresses for the root guards.
    pub fn new(sm: CljxStateMachine) -> Self {
        let sm = Box::new(sm);
        // The Vec buffer (slots) and the boxed `pending` field both live behind
        // the box's stable heap allocation, so these raw-pointer roots remain
        // valid even as the `CompiledAsyncTask` (and thus the box pointer) moves.
        let slots_root = root_values(&sm.slots);
        let pending_root = root_value(&sm.pending);
        Self {
            sm,
            _slots_root: slots_root,
            _pending_root: pending_root,
        }
    }
}

impl Future for CompiledAsyncTask {
    type Output = EvalResult;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<EvalResult> {
        // `CompiledAsyncTask` is `Unpin` (Box + guards), so `get_mut` is sound.
        let this = self.get_mut();
        let sm_ptr: *mut CljxStateMachine = &mut *this.sm;
        // Install the eval context so the poll function's `rt_*` global-lookup /
        // call bridges work while running detached on the executor thread.
        let _ctx_guard = this.sm.eval_ctx.as_ref().map(|(globals, ns)| {
            cljrs_runtime::env::callback::install_eval_context_guard(globals.clone(), ns.clone())
        });
        let code = (this.sm.poll_fn)(sm_ptr);
        // A memory-limit fault (from an awaited future, or raised by the
        // compiled body) waits in its own slot; it outranks the poll code.
        if let Some(fault) = cljrs_gc::resolve_runtime_fault() {
            return Poll::Ready(Err(EvalError::MemoryLimitExceeded(fault)));
        }
        // The poll function reports its result in-band via `pending` (a plain,
        // GC-rooted `Value`), so completion is a safe field read — no raw
        // FFI-provenance pointer is dereferenced here.
        match code {
            POLL_PENDING => {
                // Cooperative re-poll, mirroring the tree-walker's `yield_now`:
                // the GC service runs between polls at the LocalSet's yield.
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            POLL_READY => Poll::Ready(Ok(this.sm.pending.clone())),
            POLL_THREW => Poll::Ready(Err(EvalError::Thrown(this.sm.pending.clone()))),
            POLL_GAS_EXHAUSTED => Poll::Ready(Err(EvalError::GasExhausted)),
            POLL_CANCELLED => Poll::Ready(Err(EvalError::Cancelled)),
            other => Poll::Ready(Err(EvalError::Runtime(format!(
                "compiled async poll returned invalid code {other}"
            )))),
        }
    }
}

/// Build a state machine for `poll_fn`, materialise `args` into its slots, and
/// spawn it on the current `LocalSet`, returning the `Value::Future` it settles.
///
/// Test-only: no dispatch path runs compiled state machines since `^:async`
/// calls moved to fibers (virtual-threads plan Phase 5).
#[cfg(test)]
pub fn spawn_state_machine(
    poll_fn: PollFn,
    n_slots: usize,
    args: Vec<Value>,
    eval_ctx: Option<EvalCtx>,
) -> Value {
    let roots = args.clone();
    let mut sm = CljxStateMachine::new(poll_fn, n_slots, args);
    sm.eval_ctx = eval_ctx;
    crate::task::spawn_future_with_roots(roots, CompiledAsyncTask::new(sm))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{await_value, spawn_future};

    fn block_on_local<F: std::future::Future>(f: F) -> F::Output {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("build runtime");
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, f)
    }

    /// A hand-written two-state poll function standing in for compiled output:
    /// `state 0` registers the awaited value (slot 0) and suspends; `state 1`
    /// resumes, doubles the resolved Long, and stores it in `pending`.
    extern "C" fn double_poll(sm: *mut CljxStateMachine) -> i32 {
        // SAFETY: the adapter passes a valid, exclusively-borrowed machine.
        let sm = unsafe { &mut *sm };
        match sm.state {
            0 => {
                sm.pending = sm.slots[0].clone();
                sm.state = 1;
                POLL_PENDING
            }
            1 => match check_ready(&sm.pending) {
                Readiness::Pending => POLL_PENDING,
                Readiness::Ready(v) => {
                    let n = match v {
                        Value::Long(n) => n,
                        _ => 0,
                    };
                    sm.pending = Value::Long(n * 2);
                    POLL_READY
                }
                Readiness::Failed(e) => {
                    sm.pending = e;
                    POLL_THREW
                }
                Readiness::GasExhausted => POLL_GAS_EXHAUSTED,
                Readiness::MemoryLimitExceeded(fault) => {
                    cljrs_gc::governor::confirm_runtime_fault(fault);
                    POLL_THREW
                }
            },
            _ => POLL_PENDING,
        }
    }

    #[test]
    fn state_machine_awaits_and_returns_result() {
        block_on_local(async {
            // The awaited value: a future that resolves to 21.
            let arg = spawn_future(async { Ok(Value::Long(21)) });
            let result = spawn_state_machine(double_poll, 2, vec![arg], None);
            let v = await_value(result).await.expect("resolves");
            assert!(matches!(v, Value::Long(42)), "got {v:?}");
        });
    }

    #[test]
    fn state_machine_awaiting_plain_value_is_identity() {
        block_on_local(async {
            // Awaiting a non-future value resolves immediately on first resume.
            let result = spawn_state_machine(double_poll, 2, vec![Value::Long(5)], None);
            let v = await_value(result).await.expect("resolves");
            assert!(matches!(v, Value::Long(10)), "got {v:?}");
        });
    }

    /// A poll function that fails surfaces as a thrown error.
    extern "C" fn throw_poll(sm: *mut CljxStateMachine) -> i32 {
        let sm = unsafe { &mut *sm };
        sm.pending = Value::Str(cljrs_gc::GcPtr::new("boom".into()));
        POLL_THREW
    }

    #[test]
    fn state_machine_throw_propagates() {
        block_on_local(async {
            let result = spawn_state_machine(throw_poll, 1, vec![], None);
            let err = await_value(result).await;
            assert!(err.is_err(), "expected thrown error, got {err:?}");
        });
    }
}
