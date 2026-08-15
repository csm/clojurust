//! The seam between a runtime and its JIT compiler.
//!
//! The JIT is an optional system: a runtime runs its tiers without one (tree
//! walk → Tier-1 IR), and the compiler package that provides one
//! (`cljrs-compiler`) depends on this package, so the call can only go one
//! way.  That is what this trait is for — and it is the *only* thing it is
//! for.  Everything the JIT accumulates (invocation counters, argument-type
//! profiles, published function pointers, OSR entries) is runtime-owned state
//! in [`JitState`](crate::tiered::jit_state::JitState), not something a
//! process-global hook keeps on the side.
//!
//! A backend is installed per runtime, by `cljrs_compiler::jit::install`:
//!
//! ```rust,ignore
//! let runtime = Runtime::builder().execution_mode(ExecutionMode::Tiered).build()?;
//! cljrs_compiler::jit::install(&runtime);
//! ```
//!
//! Two runtimes in one process may install different backends, or one may
//! install none — dispatch reads the backend from the runtime it is
//! executing in, never from a global.

use std::sync::Arc;
use std::sync::Weak;

use cljrs_ir::IrFunction;
use cljrs_value::Value;

use crate::env::env::Env;
use crate::tiered::tiers::Tiers;

/// A JIT compiler attached to one runtime.
///
/// Implementations are shared across mutator threads and the background
/// compile worker, hence `Send + Sync`.  Individual methods may take `!Send`
/// arguments (`Value`, `Env`): those are only ever called on a mutator
/// thread.
pub trait JitBackend: Send + Sync {
    /// Compile `arity_id` in the background; `tiers` names the runtime to
    /// publish into.  Called from the Tier-1 dispatch hot path when an arity
    /// crosses the invocation threshold, at most once per arity, so it must
    /// not block: enqueue and return.
    /// Returns whether the request was accepted.  A saturated backend must
    /// return `false` so the hot path can retry on a later call.
    fn enqueue_function(&self, tiers: Weak<Tiers>, arity_id: u64, ir_func: Arc<IrFunction>)
    -> bool;

    /// Compile an OSR entry for the loop at `header` inside `arity_id`
    /// (Phase 10.4).  Same non-blocking contract as [`Self::enqueue_function`].
    /// Returns whether the request was accepted.  A rejected request is not
    /// left permanently pending; the interpreter may request it again.
    fn enqueue_osr(
        &self,
        tiers: Weak<Tiers>,
        arity_id: u64,
        header: u32,
        ir_func: Arc<IrFunction>,
    ) -> bool;

    /// Supersede the compiled module tagged `epoch`: no new call will reach
    /// it, and its memory is freed once no frame is executing it.
    fn mark_stale(&self, epoch: u64);

    /// Take (and clear) this thread's pending exception.
    ///
    /// Compiled code signals `(throw …)` by stashing the thrown value in a
    /// thread-local owned by the compiler's runtime ABI and returning the nil
    /// sentinel; the dispatch seam calls this immediately after native code
    /// returns so an uncaught throw becomes `EvalError::Thrown` instead of a
    /// silent nil (and cannot leak into the next `rt_try` on this thread).
    fn take_pending_exception(&self) -> Option<Value>;

    /// Address of the pointer a failed entry guard returns (Phase 10.6).
    ///
    /// The dispatch seam compares native results against it to detect a
    /// deoptimization; the address is a compiler-owned leaked allocation that
    /// no real result can ever alias.
    fn deopt_sentinel(&self) -> usize;

    /// Compile the called `^:async` arity to a native poll function and
    /// register it, so subsequent dispatches run the state machine instead of
    /// the `eval_async` tree-walker (Phase H).
    ///
    /// A no-op on any lowering or codegen failure.  Called once per arity by
    /// the async dispatcher, which cannot reach a compiler itself.
    fn compile_async_arity(&self, callee: &Value, nargs: usize, env: &mut Env);
}
