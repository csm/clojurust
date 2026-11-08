//! Call-depth cap for [`ExecutionMode::NoGcTransaction`].
//!
//! Every interpreted application consumes real Rust stack, so a hostile or
//! merely runaway transaction could overflow the host thread's stack and abort
//! the process.  A runtime built in [`ExecutionMode::NoGcTransaction`] routes
//! its calls through [`call_cljrs_fn`], which refuses to nest deeper than the
//! limit installed by [`DepthGuard`].
//!
//! This used to be a `call_cljrs_fn` function pointer that `cljrs-tx` stored in
//! `GlobalEnv`.  It is the one call-path override that had a reason to exist,
//! so it survives the removal of the seam — as an execution mode owned by the
//! runtime rather than as an arbitrary hook.
//!
//! [`ExecutionMode::NoGcTransaction`]: crate::ExecutionMode::NoGcTransaction

use std::cell::Cell;

use cljrs_value::{CljxFn, Value};

use crate::env::env::Env;
use crate::env::error::{EvalError, EvalResult};

/// Marker text for a depth-cap rejection.  Surfaced as
/// [`EvalError::Runtime`] because the interpreter's call path has a fixed
/// error type; `cljrs-tx` matches on it to report a depth overrun.
pub const DEPTH_EXCEEDED_MSG: &str = "cljrs-tx: transaction call depth exceeded";

thread_local! {
    /// `(limit, current)` nested-application counter for the running
    /// invocation; `None` outside one.
    static CALL_DEPTH: Cell<Option<(u64, u64)>> = const { Cell::new(None) };
}
cljrs_gc::mutator_slot!("cljrs-runtime/CALL_DEPTH", cell CALL_DEPTH: Option<(u64, u64)>);

/// Installs the call-depth budget for one invocation's dynamic extent.
///
/// The budget is thread-local and is cleared when the guard drops, including
/// on unwind.
pub struct DepthGuard;

impl DepthGuard {
    pub fn install(limit: u64) -> Self {
        CALL_DEPTH.with(|cell| cell.set(Some((limit, 0))));
        Self
    }
}

impl Drop for DepthGuard {
    fn drop(&mut self) {
        CALL_DEPTH.with(|cell| cell.set(None));
    }
}

/// Tree-walking function application with the transaction depth cap applied.
///
/// With no [`DepthGuard`] installed on this thread this is exactly
/// `crate::interp::apply::call_cljrs_fn`.
#[allow(clippy::result_large_err)]
pub fn call_cljrs_fn(f: &CljxFn, args: &[Value], env: &mut Env) -> EvalResult {
    let Some((limit, depth)) = CALL_DEPTH.with(Cell::get) else {
        return crate::interp::apply::call_cljrs_fn(f, args, env);
    };
    if depth >= limit {
        return Err(EvalError::Runtime(DEPTH_EXCEEDED_MSG.into()));
    }
    CALL_DEPTH.with(|cell| cell.set(Some((limit, depth + 1))));
    let result = crate::interp::apply::call_cljrs_fn(f, args, env);
    CALL_DEPTH.with(|cell| {
        if let Some((limit, current)) = cell.get() {
            cell.set(Some((limit, current.saturating_sub(1))));
        }
    });
    result
}
