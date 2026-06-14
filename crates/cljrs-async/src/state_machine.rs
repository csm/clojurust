//! Compiled async state machines (Phase H runtime support).
//!
//! When `cljrs_ir::lower::async_lower` rewrites an `^:async` function into a
//! poll function, codegen emits it as a C-ABI *poll function* with the
//! signature
//!
//! ```text
//! extern "C" fn(*mut CljxStateMachine, *mut *const Value) -> i32
//! ```
//!
//! returning [`POLL_PENDING`] / [`POLL_READY`] / [`POLL_THREW`].  This module
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
use std::task::{Context, Poll};

use cljrs_env::error::{EvalError, EvalResult};
use cljrs_env::gc_roots::{ValueRootGuard, root_value, root_values};
use cljrs_value::{FutureState, Value};

use crate::eval_async::spawn_future;

/// Poll-function return code: the task suspended; re-poll later.
pub const POLL_PENDING: i32 = 0;
/// Poll-function return code: the task finished; the result is in `pending`.
pub const POLL_READY: i32 = 1;
/// Poll-function return code: the task threw; the thrown value is in `pending`.
pub const POLL_THREW: i32 = 2;

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
}

/// Check whether an awaited value has resolved, without blocking.  Mirrors
/// [`crate::eval_async::await_value`] but for the poll-loop (no `.await`):
/// futures/promises report their settled state; any other value is immediately
/// ready (awaiting a non-future is the identity).
pub fn check_ready(val: &Value) -> Readiness {
    match val {
        Value::Future(f) => match &*f.get().state.lock().unwrap() {
            FutureState::Done(v) => {
                f.get().mark_observed();
                Readiness::Ready(v.clone())
            }
            FutureState::Failed(v) => {
                f.get().mark_observed();
                Readiness::Failed(v.clone())
            }
            FutureState::Cancelled => Readiness::Failed(Value::Str(cljrs_gc::GcPtr::new(
                "future was cancelled".into(),
            ))),
            FutureState::Running => Readiness::Pending,
        },
        Value::Promise(p) => match p.get().value.lock().unwrap().as_ref() {
            Some(v) => Readiness::Ready(v.clone()),
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
        let code = (this.sm.poll_fn)(sm_ptr);
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
            other => Poll::Ready(Err(EvalError::Runtime(format!(
                "compiled async poll returned invalid code {other}"
            )))),
        }
    }
}

/// Build a state machine for `poll_fn`, materialise `args` into its slots, and
/// spawn it on the current `LocalSet`, returning the `Value::Future` it settles.
///
/// This is the compiled-async counterpart of
/// [`crate::runtime::AsyncRuntimeImpl::spawn_async_call`]'s `spawn_future`
/// path: the dispatcher calls it when the callee arity has a compiled poll
/// function.
pub fn spawn_state_machine(poll_fn: PollFn, n_slots: usize, args: Vec<Value>) -> Value {
    let sm = CljxStateMachine::new(poll_fn, n_slots, args);
    spawn_future(CompiledAsyncTask::new(sm))
}

// ── Compiled poll-function registry ──────────────────────────────────────────
//
// AOT/JIT compilation of an `^:async` function emits a poll function and
// registers it here, keyed by the defining namespace, name, and fixed arity.
// `AsyncRuntimeImpl::spawn_async_call` consults the registry: a hit runs the
// native state machine via `spawn_state_machine`, a miss falls back to the
// tree-walking `run_async_fn`.  Function pointers are `Send + Sync`, so the
// registry is a simple global map.

use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Clone, Copy)]
struct PollEntry {
    poll_fn: PollFn,
    n_slots: usize,
}

static POLL_REGISTRY: RwLock<Option<HashMap<(String, String, usize), PollEntry>>> =
    RwLock::new(None);

/// Register a compiled poll function for `ns/name` at a fixed `arity` (number of
/// positional parameters).  Called by the AOT harness / JIT once per compiled
/// `^:async` arity.
pub fn register_poll_fn(ns: &str, name: &str, arity: usize, poll_fn: PollFn, n_slots: usize) {
    let mut guard = POLL_REGISTRY.write().unwrap();
    guard.get_or_insert_with(HashMap::new).insert(
        (ns.to_string(), name.to_string(), arity),
        PollEntry { poll_fn, n_slots },
    );
}

/// Look up a compiled poll function for `ns/name` at the given `arity`.
pub fn lookup_poll_fn(ns: &str, name: &str, arity: usize) -> Option<(PollFn, usize)> {
    let guard = POLL_REGISTRY.read().unwrap();
    guard
        .as_ref()?
        .get(&(ns.to_string(), name.to_string(), arity))
        .map(|e| (e.poll_fn, e.n_slots))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval_async::{await_value, spawn_future};

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
            },
            _ => POLL_PENDING,
        }
    }

    #[test]
    fn state_machine_awaits_and_returns_result() {
        block_on_local(async {
            // The awaited value: a future that resolves to 21.
            let arg = spawn_future(async { Ok(Value::Long(21)) });
            let result = spawn_state_machine(double_poll, 2, vec![arg]);
            let v = await_value(result).await.expect("resolves");
            assert!(matches!(v, Value::Long(42)), "got {v:?}");
        });
    }

    #[test]
    fn state_machine_awaiting_plain_value_is_identity() {
        block_on_local(async {
            // Awaiting a non-future value resolves immediately on first resume.
            let result = spawn_state_machine(double_poll, 2, vec![Value::Long(5)]);
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
            let result = spawn_state_machine(throw_poll, 1, vec![]);
            let err = await_value(result).await;
            assert!(err.is_err(), "expected thrown error, got {err:?}");
        });
    }
}
