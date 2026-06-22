//! Tier 1 IR interpreter: executes [`IrFunction`] over a VarId→Value register file.
//!
//! This replaces tree-walking of `Form` ASTs with interpretation of the
//! ANF IR.  Benefits:
//! - Escape analysis results are available (region allocation)
//! - Same IR feeds both interpretation and JIT compilation
//! - Optimization passes (constant folding, etc.) apply uniformly
//!
//! The interpreter maintains a dense register file (`Vec<Option<Value>>`)
//! indexed by `VarId`.  Control flow follows the block graph via
//! `Terminator`s.

use std::sync::Arc;

use cljrs_gc::GcPtr;
use cljrs_ir::{
    BlockId, ClosureTemplate, Const, Inst, IrFunction, KnownFn, RegionAllocKind, Terminator, VarId,
};
use cljrs_value::value::{MapValue, SetValue};
use cljrs_value::{
    CljxCons, CljxFn, NativeFn, PersistentHashSet, PersistentList, PersistentVector, Value,
};

use cljrs_env::apply::apply_value;
use cljrs_env::env::{Env, GlobalEnv};
use cljrs_env::error::{EvalError, EvalResult};

// ── Register file ───────────────────────────────────────────────────────────

/// Dense register file indexed by `VarId`.
///
/// Uses `Box<[Option<Value>]>` rather than `Vec` so the heap address of the
/// slice data is stable after construction.  This lets us register the slice as
/// a GC root via `root_option_values` without worrying about reallocation
/// invalidating the stored raw pointer.
struct Registers {
    values: Box<[Option<Value>]>,
}

impl Registers {
    fn new(capacity: u32) -> Self {
        Self {
            values: vec![None; capacity as usize].into_boxed_slice(),
        }
    }

    fn get(&self, id: VarId) -> &Value {
        self.values[id.0 as usize]
            .as_ref()
            .unwrap_or_else(|| panic!("IR interpreter: uninitialized register {id}"))
    }

    fn set(&mut self, id: VarId, val: Value) {
        // Bounds check: VarIds are allocated sequentially up to ir_func.next_var,
        // so any out-of-range access indicates malformed IR.
        self.values[id.0 as usize] = Some(val);
    }

    fn get_cloned(&self, id: VarId) -> Value {
        self.get(id).clone()
    }
}

// ── Region state ────────────────────────────────────────────────────────────

/// A region entry in the interpreter's local region stack.
struct RegionEntry {
    /// `Some` until closed; taken by `Drop`.  The region is also pushed onto
    /// the cljrs_gc thread-local REGION_STACK via `push_region_raw`.
    region: Option<Box<cljrs_gc::region::Region>>,
}

impl Drop for RegionEntry {
    fn drop(&mut self) {
        if let Some(region) = self.region.take() {
            // Pops the thread-local stack entry, then resets the region — or
            // retires it if a publish barrier poisoned it (Phase 10.5
            // heap-promotion fallback).
            cljrs_gc::region::close_region(region);
        }
    }
}

/// Per-execution region state: the owned scope stack plus the handle map
/// binding region `VarId`s to their concrete regions.
///
/// The handle map is what makes `RegionAlloc(dst, region, …)` allocate into
/// the *named* region rather than blindly into the innermost open scope —
/// essential once a region-parameterised callee opens scopes of its own: an
/// alloc naming the inherited `RegionParam` must go into the caller's region
/// even while an inner scope is on top of the stack.  (Compiled code gets the
/// same semantics by passing the `*mut Region` handle to `rt_region_alloc_*`.)
#[derive(Default)]
struct RegionFrame {
    stack: Vec<RegionEntry>,
    /// `region VarId → live region` for every `RegionStart`/`RegionParam` in
    /// this frame.  Functions have at most a handful of region vars.
    handles: Vec<(VarId, *mut cljrs_gc::region::Region)>,
    /// The caller's region, when this frame is a region-parameterised callee
    /// entered via `CallWithRegion`.
    inherited: Option<*mut cljrs_gc::region::Region>,
}

impl RegionFrame {
    fn bind(&mut self, var: VarId, region: *mut cljrs_gc::region::Region) {
        self.handles.push((var, region));
    }

    fn lookup(&self, var: VarId) -> Option<*mut cljrs_gc::region::Region> {
        self.handles
            .iter()
            .rev()
            .find(|(v, _)| *v == var)
            .map(|&(_, r)| r)
    }
}

/// Allocate `val` into `target` when given, falling back to the innermost
/// thread-local region (then the GC heap).
fn region_alloc_val<T: cljrs_gc::Trace + 'static>(
    target: Option<*mut cljrs_gc::region::Region>,
    val: T,
) -> cljrs_gc::GcPtr<T> {
    match target {
        // SAFETY: handles are only bound to regions owned by a live
        // `RegionEntry` of this or a caller's frame.
        Some(region) => unsafe { (*region).alloc(val) },
        None => {
            if cljrs_gc::region::region_is_active() {
                unsafe { cljrs_gc::region::try_alloc_in_region(val).unwrap() }
            } else {
                GcPtr::new(val)
            }
        }
    }
}

// ── OSR (on-stack replacement) bookkeeping ──────────────────────────────────

/// Per-execution OSR state — Phase 10.4.
///
/// Allocated lazily on the first loop back-edge of an OSR-eligible execution
/// (a top-level arity dispatched through `execute_ir`), so straight-line
/// functions pay nothing.  Back-edge counts are local to one execution on
/// purpose: a loop that is hot *within a single call* is exactly the case
/// invocation-count tiering cannot promote; loops spread over many short
/// calls are already covered by the invocation counter.
struct OsrLocal {
    arity_id: u64,
    threshold: u32,
    /// Back-edge count per loop header (`BlockId.0`).
    counts: std::collections::HashMap<u32, u32>,
    /// Header we are waiting on: compilation requested (or already published),
    /// checked at each loop-header entry.
    polling: Option<BlockId>,
    /// Headers that must not be polled or re-requested in this execution
    /// (compilation failed, or a transfer was declined).
    dead: std::collections::HashSet<u32>,
}

/// Record one loop back-edge to `target`.  Crossing the threshold requests OSR
/// compilation (idempotent across executions via the global table).
fn record_back_edge(
    osr: &mut Option<Box<OsrLocal>>,
    arity_id: u64,
    target: BlockId,
    ir_func: &IrFunction,
) {
    let ol = osr.get_or_insert_with(|| {
        Box::new(OsrLocal {
            arity_id,
            threshold: crate::jit_state::osr_threshold().max(1),
            counts: std::collections::HashMap::new(),
            polling: None,
            dead: std::collections::HashSet::new(),
        })
    });
    if ol.polling == Some(target) || ol.dead.contains(&target.0) {
        return;
    }
    let count = {
        let c = ol.counts.entry(target.0).or_insert(0);
        *c += 1;
        *c
    };
    if count == 1 {
        // A previous execution may already have requested (or finished)
        // compilation of this loop — start polling right away.
        match crate::jit_state::osr_poll(arity_id, target.0) {
            crate::jit_state::OsrPoll::Ready(_) | crate::jit_state::OsrPoll::Pending => {
                ol.polling = Some(target);
                return;
            }
            crate::jit_state::OsrPoll::Failed => {
                ol.dead.insert(target.0);
                return;
            }
            crate::jit_state::OsrPoll::NotRequested => {}
        }
    }
    if count >= ol.threshold {
        crate::jit_state::osr_request(arity_id, target.0, ir_func);
        ol.polling = Some(target);
    }
}

/// Transfer this execution into compiled OSR-entry code: snapshot the live-in
/// registers, call the native entry, and hand back its result as the result of
/// the whole call.
///
/// Returns `None` (caller keeps interpreting) if any live-in register is not
/// yet initialized — a conservatively declined transfer, not an error.
fn try_osr_enter(
    slot: &crate::jit_state::OsrSlot,
    regs: &Registers,
    env: &mut Env,
) -> Option<EvalResult> {
    let mut call_args: Vec<Value> = Vec::with_capacity(slot.live_ins.len());
    for var in slot.live_ins.iter() {
        match regs.values.get(var.0 as usize).and_then(|v| v.as_ref()) {
            Some(v) => call_args.push(v.clone()),
            None => return None,
        }
    }
    cljrs_logging::feat_debug!(
        "jit",
        "osr entering native code ({} live-ins, epoch={})",
        call_args.len(),
        slot.epoch
    );

    // Same protocol as `call_jit_native` (apply.rs): keep the backing module
    // alive for the duration of the frame, root the caller env and the
    // snapshot, and track allocations made inside the native frame.
    let _jit_frame = crate::jit_state::push_jit_frame(slot.epoch);
    let _caller_root = cljrs_env::gc_roots::push_env_root(env);
    let _arg_roots = cljrs_env::gc_roots::root_values(&call_args);
    let _alloc_frame = cljrs_gc::push_alloc_frame();

    let arg_ptrs: Vec<*const Value> = call_args.iter().map(|v| v as *const Value).collect();
    // SAFETY: fn_ptr was produced by Cranelift JIT with the C ABI and exactly
    // `live_ins.len()` `*const Value` params (`build_osr_function` caps this at
    // the dispatch limit); all arg pointers are live for the call.
    let result_ptr = unsafe { crate::jit_state::dispatch_jit_call(slot.fn_ptr, &arg_ptrs) };
    // SAFETY: result_ptr points to a live Value in ALLOC_ROOTS; clone it
    // before the alloc frame drops.
    let result = unsafe { (*result_ptr).clone() };

    // Same as `call_jit_native`: an uncaught `(throw …)` inside native code
    // stashes the thrown value and returns the nil sentinel — surface it as an
    // error while the alloc frame still roots it.
    if let Some(thrown) = crate::jit_state::take_pending_exception() {
        return Some(Err(cljrs_env::error::EvalError::Thrown(thrown)));
    }
    Some(Ok(result))
}

// ── Public entry point ──────────────────────────────────────────────────────

/// Execute an IR function with the given arguments.
///
/// This is the Tier 1 execution path, called from `call_cljrs_fn` when
/// a cached `IrFunction` is available.
///
/// # Arguments
/// * `ir_func` — the IR function to execute
/// * `args` — argument values (positional, already matched to the arity)
/// * `globals` — the shared global environment
/// * `ns` — the namespace context for global lookups
/// * `env` — caller's Env (for calling back into `apply_value`)
pub fn interpret_ir(
    ir_func: &IrFunction,
    args: Vec<Value>,
    globals: &Arc<GlobalEnv>,
    ns: &Arc<str>,
    env: &mut Env,
) -> EvalResult {
    interpret_ir_with_osr(ir_func, args, globals, ns, env, None)
}

/// Like [`interpret_ir`], with OSR enabled when `osr_arity_id` is given.
///
/// `osr_arity_id` is the `ir_arity_id` under which loop back-edges are counted
/// and OSR-compiled entries are published.  Pass `None` (or use
/// [`interpret_ir`]) for executions that have no stable arity identity, e.g.
/// IR closures and region-parameterised subfunction calls.
pub fn interpret_ir_with_osr(
    ir_func: &IrFunction,
    args: Vec<Value>,
    globals: &Arc<GlobalEnv>,
    ns: &Arc<str>,
    env: &mut Env,
    osr_arity_id: Option<u64>,
) -> EvalResult {
    interpret_ir_inner(ir_func, args, globals, ns, env, osr_arity_id, None)
}

#[allow(clippy::too_many_arguments)]
fn interpret_ir_inner(
    ir_func: &IrFunction,
    args: Vec<Value>,
    globals: &Arc<GlobalEnv>,
    ns: &Arc<str>,
    env: &mut Env,
    osr_arity_id: Option<u64>,
    inherited_region: Option<*mut cljrs_gc::region::Region>,
) -> EvalResult {
    // GC safepoint at function entry.
    cljrs_env::gc_roots::gc_safepoint(env);

    let mut regs = Registers::new(ir_func.next_var);
    // Keep all values in the register file alive across GC safepoints.
    // The Box<[Option<Value>]> slice address is stable; this guard pops the
    // root entry when interpret_ir returns (or unwinds).
    let _regs_root = cljrs_env::gc_roots::root_option_values(&regs.values);
    let mut region_frame = RegionFrame {
        inherited: inherited_region,
        ..RegionFrame::default()
    };

    // Bind parameters to registers.
    for (i, (_name, var_id)) in ir_func.params.iter().enumerate() {
        if i < args.len() {
            regs.set(*var_id, args[i].clone());
        } else {
            regs.set(*var_id, Value::Nil);
        }
    }

    // Build a block index for O(1) lookup.
    // In the common case (dense sequential IDs), this is None and we use
    // block_id.0 directly as the index — no allocation at all.
    let block_index = ir_func.block_index();

    // Closure to resolve BlockId → index in ir_func.blocks.
    let resolve = |bid: &BlockId| -> usize {
        match &block_index {
            Some(table) => table[bid.0 as usize],
            None => bid.0 as usize,
        }
    };

    // Start at block 0.
    let mut current_block_idx: usize = 0;
    let mut prev_block_id = BlockId(u32::MAX); // sentinel

    // Lazily allocated OSR back-edge state (only for OSR-eligible executions
    // that actually take a loop back-edge).
    let mut osr: Option<Box<OsrLocal>> = None;

    loop {
        let block = &ir_func.blocks[current_block_idx];

        // Resolve phi nodes based on predecessor.
        for phi in &block.phis {
            if let Inst::Phi(dst, entries) = phi {
                for (from_block, var_id) in entries {
                    if *from_block == prev_block_id {
                        regs.set(*dst, regs.get_cloned(*var_id));
                        break;
                    }
                }
            }
        }

        // OSR transfer: once a hot loop header's compilation has been
        // requested, check for published native code each time we re-enter the
        // header (i.e. after its phis — the loop variables — are resolved).
        if let Some(ol) = osr.as_deref_mut()
            && ol.polling == Some(block.id)
        {
            match crate::jit_state::osr_poll(ol.arity_id, block.id.0) {
                crate::jit_state::OsrPoll::Ready(slot) => {
                    // Regions opened before the loop stay open across the
                    // transfer (the OSR variant drops their RegionEnds) and
                    // are closed as usual when `region_stack` unwinds after
                    // the native call returns.
                    if let Some(result) = try_osr_enter(&slot, &regs, env) {
                        return result;
                    }
                    ol.polling = None;
                    ol.dead.insert(block.id.0);
                }
                crate::jit_state::OsrPoll::Failed => {
                    ol.polling = None;
                    ol.dead.insert(block.id.0);
                }
                _ => {}
            }
        }

        // Execute instructions.
        for inst in &block.insts {
            execute_inst(
                inst,
                &mut regs,
                &mut region_frame,
                ir_func,
                globals,
                ns,
                env,
            )?;
        }

        // Execute terminator.
        match &block.terminator {
            Terminator::Return(var_id) => {
                return Ok(regs.get_cloned(*var_id));
            }
            Terminator::Jump(target) => {
                prev_block_id = block.id;
                current_block_idx = resolve(target);
            }
            Terminator::Branch {
                cond,
                then_block,
                else_block,
            } => {
                prev_block_id = block.id;
                let cond_val = regs.get(*cond);
                let truthy = is_truthy(cond_val);
                let target = if truthy { then_block } else { else_block };
                current_block_idx = resolve(target);
            }
            Terminator::RecurJump { target, args: _ } => {
                // GC safepoint at loop back-edge.
                cljrs_env::gc_roots::gc_safepoint(env);

                // Loop back-edge counter: a hot header triggers background OSR
                // compilation; the transfer happens at the header entry above.
                if let Some(arity_id) = osr_arity_id {
                    record_back_edge(&mut osr, arity_id, *target, ir_func);
                }

                // Jump to the target (loop header) block.
                // Phi nodes at the target block entry will resolve the new
                // loop variable values based on this block as predecessor.
                prev_block_id = block.id;
                current_block_idx = resolve(target);
            }
            Terminator::Unreachable => {
                return Err(EvalError::Runtime(
                    "IR interpreter: reached unreachable".to_string(),
                ));
            }
        }
    }
}

// ── Truthiness ──────────────────────────────────────────────────────────────

/// Clojure truthiness: everything is truthy except `nil` and `false`.
fn is_truthy(val: &Value) -> bool {
    !matches!(val, Value::Nil | Value::Bool(false))
}

// ── Instruction execution ───────────────────────────────────────────────────

fn execute_inst(
    inst: &Inst,
    regs: &mut Registers,
    regions: &mut RegionFrame,
    ir_func: &IrFunction,
    globals: &Arc<GlobalEnv>,
    ns: &Arc<str>,
    env: &mut Env,
) -> EvalResult<()> {
    match inst {
        Inst::Const(dst, c) => {
            regs.set(*dst, const_to_value(c));
        }

        Inst::LoadLocal(dst, name) => {
            // Look up in the caller's environment (captures, locals).
            let val = env
                .lookup(name)
                .ok_or_else(|| {
                    EvalError::Runtime(format!("IR interpreter: unbound local '{name}'"))
                })?
                .clone();
            regs.set(*dst, val);
        }

        Inst::LoadGlobal(dst, gns, name) => {
            let val = load_global_value(globals, gns, name, ns)?;
            regs.set(*dst, val);
        }

        Inst::LoadVar(dst, gns, name) => {
            let resolved_ns = globals
                .resolve_alias(ns, gns)
                .unwrap_or_else(|| Arc::from(&**gns));
            let var = globals
                .lookup_var_in_ns(&resolved_ns, name)
                .ok_or_else(|| {
                    EvalError::Runtime(format!(
                        "IR interpreter: var not found {resolved_ns}/{name}"
                    ))
                })?;
            regs.set(*dst, Value::Var(var));
        }

        Inst::AllocVector(dst, elems) => {
            let items: Vec<Value> = elems.iter().map(|v| regs.get_cloned(*v)).collect();
            let pv = PersistentVector::from_iter(items);
            regs.set(*dst, Value::Vector(GcPtr::new(pv)));
        }

        Inst::AllocMap(dst, pairs) => {
            let kv: Vec<(Value, Value)> = pairs
                .iter()
                .map(|(k, v)| (regs.get_cloned(*k), regs.get_cloned(*v)))
                .collect();
            regs.set(*dst, Value::Map(MapValue::from_pairs(kv)));
        }

        Inst::AllocSet(dst, elems) => {
            let items: Vec<Value> = elems.iter().map(|v| regs.get_cloned(*v)).collect();
            let set = PersistentHashSet::from_iter(items);
            regs.set(*dst, Value::Set(SetValue::Hash(GcPtr::new(set))));
        }

        Inst::AllocList(dst, elems) => {
            let items: Vec<Value> = elems.iter().map(|v| regs.get_cloned(*v)).collect();
            regs.set(
                *dst,
                Value::List(GcPtr::new(PersistentList::from_iter(items))),
            );
        }

        Inst::AllocCons(dst, head, tail) => {
            let h = regs.get_cloned(*head);
            let t = regs.get_cloned(*tail);
            regs.set(*dst, Value::Cons(GcPtr::new(CljxCons { head: h, tail: t })));
        }

        Inst::AllocClosure(dst, template, captures) => {
            let val = alloc_closure(template, captures, regs, ir_func, globals, ns)?;
            regs.set(*dst, val);
        }

        Inst::CallKnown(dst, known_fn, args) => {
            // GC safepoint before call.
            cljrs_env::gc_roots::gc_safepoint(env);
            let arg_vals: Vec<Value> = args.iter().map(|v| regs.get_cloned(*v)).collect();
            let result = dispatch_known_fn(known_fn, arg_vals, env)?;
            regs.set(*dst, result);
        }

        Inst::Call(dst, callee, args) => {
            cljrs_env::gc_roots::gc_safepoint(env);
            let callee_val = regs.get_cloned(*callee);
            let arg_vals: Vec<Value> = args.iter().map(|v| regs.get_cloned(*v)).collect();
            let result = dispatch_or_sentinel(callee_val, arg_vals, globals, ns, env)?;
            regs.set(*dst, result);
        }

        Inst::CallDirect(dst, name, args) => {
            cljrs_env::gc_roots::gc_safepoint(env);
            let arg_vals: Vec<Value> = args.iter().map(|v| regs.get_cloned(*v)).collect();
            let result = dispatch_sentinel_by_name(name, arg_vals, globals, ns, env)?;
            regs.set(*dst, result);
        }

        Inst::Deref(dst, src) => {
            let val = regs.get_cloned(*src);
            let derefed = cljrs_interp::eval::deref_value(val)?;
            regs.set(*dst, derefed);
        }

        Inst::DefVar(dst, def_ns, name, val_var) => {
            // Always create a fresh, call-local Var (not registered in globals).
            // The ANF compiler uses DefVar as a mutable cell for letfn / named-fn
            // self-recursion; the cell is accessed only via the register returned
            // here, never via LoadGlobal, so name collisions with real globals are
            // harmless and we never need to touch the global namespace.
            let val = regs.get_cloned(*val_var);
            let fresh = cljrs_value::Var::new(def_ns.clone(), name.clone());
            fresh.bind(val);
            regs.set(*dst, Value::Var(GcPtr::new(fresh)));
        }

        Inst::SetBang(var_id, val_id) => {
            let var_val = regs.get(*var_id);
            let new_val = regs.get_cloned(*val_id);
            if let Value::Var(var) = var_val {
                // Try dynamic binding first, then root.
                if !cljrs_env::dynamics::set_thread_local(var, new_val.clone()) {
                    var.get().bind(new_val);
                }
            } else {
                return Err(EvalError::Runtime("set! target is not a Var".to_string()));
            }
        }

        Inst::Throw(val_id) => {
            let val = regs.get_cloned(*val_id);
            return Err(EvalError::Thrown(val));
        }

        Inst::Phi(..) => {
            // Phis are resolved at block entry, not here.
        }

        Inst::Recur(args) => {
            let vals: Vec<Value> = args.iter().map(|v| regs.get_cloned(*v)).collect();
            return Err(EvalError::Recur(vals));
        }

        Inst::SourceLoc(_span) => {
            // No-op — could update a "current span" for error reporting.
        }

        // ── Region allocation ───────────────────────────────────────────
        Inst::RegionStart(dst) => {
            let mut region = Box::new(cljrs_gc::region::Region::new());
            let region_ptr: *mut cljrs_gc::region::Region = &mut *region;
            unsafe { cljrs_gc::region::push_region_raw(region_ptr) };
            regions.stack.push(RegionEntry {
                region: Some(region),
            });
            regions.bind(*dst, region_ptr);
            regs.set(*dst, Value::Nil);
        }

        Inst::RegionAlloc(dst, region, kind, operands) => {
            let val = alloc_in_region(*kind, operands, regs, regions.lookup(*region))?;
            regs.set(*dst, val);
        }

        Inst::RegionEnd(_region) => {
            // Pop and drop the region entry (Drop impl handles cleanup).
            regions.stack.pop();
        }

        Inst::RegionParam(dst) => {
            // Bind the caller's region (threaded through `CallWithRegion`) so
            // `RegionAlloc`s naming this handle allocate into it even when
            // this frame opens scopes of its own.  The register itself holds
            // nil — region handles are not first-class values here.
            if let Some(region) = regions.inherited {
                regions.bind(*dst, region);
            }
            regs.set(*dst, Value::Nil);
        }

        Inst::CallWithRegion(dst, name, args, region) => {
            cljrs_env::gc_roots::gc_safepoint(env);
            let target = ir_func
                .subfunctions
                .iter()
                .find(|sf| sf.name.as_deref() == Some(name.as_ref()))
                .ok_or_else(|| {
                    EvalError::Runtime(format!(
                        "IR interpreter: CallWithRegion target {name} not found in subfunctions"
                    ))
                })?;
            let arg_vals: Vec<Value> = args.iter().map(|v| regs.get_cloned(*v)).collect();
            // Thread our region handle into the callee so its inherited
            // `RegionParam` allocations land in *this* region — by name, not
            // by stack position (the callee may open scopes of its own).
            let result = interpret_ir_inner(
                target,
                arg_vals,
                globals,
                ns,
                env,
                None,
                regions.lookup(*region),
            )?;
            regs.set(*dst, result);
        }

        // ── Async instructions ───────────────────────────────────────────
        //
        // Async IR functions are routed to tree-walking eval_async (via the
        // `try_ir_path` bypass in apply.rs), so these arms are rarely reached
        // in practice.  They provide a graceful sync-context fallback for the
        // cases where they are reached (e.g. in tests or non-async callers).
        Inst::Await { src, dst } => {
            // Sync fallback: block the OS thread until the future/promise resolves.
            let val = regs.get_cloned(*src);
            let resolved = cljrs_interp::eval::deref_value(val)?;
            regs.set(*dst, resolved);
        }

        Inst::Spawn { fn_reg, args, dst } => {
            // Dispatch through the async runtime hook if available; otherwise error.
            let callee = regs.get_cloned(*fn_reg);
            let arg_vals: Vec<Value> = args.iter().map(|v| regs.get_cloned(*v)).collect();
            let result = if let Some(rt) = globals.async_runtime() {
                // Build a minimal env carrying globals; run_async_fn will construct
                // the closure env from the callee's captured bindings.
                let spawn_env = Env::new(globals.clone(), ns);
                rt.spawn_async_call(callee, arg_vals, spawn_env)
            } else {
                return Err(EvalError::Runtime(
                    "IR interpreter: Spawn instruction requires async runtime (cljrs-async)".into(),
                ));
            };
            regs.set(*dst, result);
        }

        Inst::ChanTake { chan, dst } => {
            let chan_val = regs.get_cloned(*chan);
            let result = if let Some(rt) = globals.async_runtime() {
                rt.chan_take_blocking(chan_val)?
            } else {
                return Err(EvalError::Runtime(
                    "IR interpreter: ChanTake requires async runtime (cljrs-async)".into(),
                ));
            };
            regs.set(*dst, result);
        }

        Inst::ChanPut { chan, val } => {
            let chan_val = regs.get_cloned(*chan);
            let put_val = regs.get_cloned(*val);
            if let Some(rt) = globals.async_runtime() {
                rt.chan_put_blocking(chan_val, put_val)?;
            } else {
                return Err(EvalError::Runtime(
                    "IR interpreter: ChanPut requires async runtime (cljrs-async)".into(),
                ));
            }
        }

        // State-machine instructions only ever appear in a compiled poll
        // function (`is_async_poll_fn`); the Tier-1 interpreter never runs one
        // (async arities are dispatched through `eval_async` or the compiled
        // state machine, never lowered to a poll-fn for interpretation).
        Inst::StateStore { .. }
        | Inst::StateLoad { .. }
        | Inst::AsyncSuspend { .. }
        | Inst::AsyncResume { .. } => {
            return Err(EvalError::Runtime(
                "IR interpreter: async state-machine instructions are compile-only".into(),
            ));
        }
    }

    Ok(())
}

// ── Constant conversion ─────────────────────────────────────────────────────

fn const_to_value(c: &Const) -> Value {
    match c {
        Const::Nil => Value::Nil,
        Const::Bool(b) => Value::Bool(*b),
        Const::Long(n) => Value::Long(*n),
        Const::Double(d) => Value::Double(*d),
        Const::Str(s) => Value::Str(GcPtr::new(s.to_string())),
        Const::Keyword(k) => Value::Keyword(GcPtr::new(cljrs_value::keyword::Keyword::parse(k))),
        Const::Symbol(s) => Value::symbol(cljrs_value::Symbol::simple(s.clone())),
        Const::Char(c) => Value::Char(*c),
    }
}

// ── Global value lookup ─────────────────────────────────────────────────────

fn load_global_value(
    globals: &Arc<GlobalEnv>,
    ns: &str,
    name: &str,
    defining_ns: &str,
) -> EvalResult {
    // Versioned reference (`name@hash`): resolve through the shared service,
    // which lazily loads the immutable `ns@hash` namespace and looks up the
    // base name in it (with the native HEAD fallback).
    #[cfg(not(target_arch = "wasm32"))]
    if let (base_name, Some(commit)) = cljrs_value::symbol::split_version(name) {
        return cljrs_env::versioned::resolve_versioned_value(
            globals,
            defining_ns,
            Some(ns),
            base_name,
            commit,
        );
    }

    // Try direct namespace lookup first, then resolve as alias.
    let resolved_ns = globals
        .resolve_alias(defining_ns, ns)
        .unwrap_or_else(|| Arc::from(ns));

    if let Some(var) = globals.lookup_var_in_ns(&resolved_ns, name) {
        if let Some(val) = cljrs_env::dynamics::deref_var(&var) {
            return Ok(val);
        }
        return Err(EvalError::Runtime(format!(
            "IR interpreter: unbound var {resolved_ns}/{name}"
        )));
    }

    // Reference into a versioned namespace (`lib@hash`/name) that has not
    // been loaded this session: load it lazily and retry the lookup.
    #[cfg(not(target_arch = "wasm32"))]
    if let (base, Some(commit)) = cljrs_value::symbol::split_version(&resolved_ns)
        && !globals.is_loaded(&resolved_ns)
    {
        cljrs_env::versioned::ensure_versioned_ns_loaded(globals, base, commit)?;
        if let Some(var) = globals.lookup_var_in_ns(&resolved_ns, name)
            && let Some(val) = cljrs_env::dynamics::deref_var(&var)
        {
            return Ok(val);
        }
    }

    // JVM class names resolve to themselves as symbols, mirroring eval_symbol.
    if cljrs_interp::eval::is_jvm_class_name(name) {
        return Ok(Value::symbol(cljrs_value::Symbol::simple(name)));
    }
    Err(EvalError::Runtime(format!(
        "IR interpreter: var not found {resolved_ns}/{name}"
    )))
}

// ── Region-aware allocation ─────────────────────────────────────────────────

fn alloc_in_region(
    kind: RegionAllocKind,
    operands: &[VarId],
    regs: &Registers,
    target: Option<*mut cljrs_gc::region::Region>,
) -> EvalResult {
    match kind {
        RegionAllocKind::Vector => {
            let items: Vec<Value> = operands.iter().map(|v| regs.get_cloned(*v)).collect();
            let pv = PersistentVector::from_iter(items);
            Ok(Value::Vector(region_alloc_val(target, pv)))
        }
        RegionAllocKind::Map => {
            // Operands are flattened [k, v, k, v, ...].
            let kv: Vec<(Value, Value)> = operands
                .chunks(2)
                .map(|pair| (regs.get_cloned(pair[0]), regs.get_cloned(pair[1])))
                .collect();
            Ok(Value::Map(MapValue::from_pairs(kv)))
        }
        RegionAllocKind::Set => {
            let items: Vec<Value> = operands.iter().map(|v| regs.get_cloned(*v)).collect();
            let set = PersistentHashSet::from_iter(items);
            Ok(Value::Set(SetValue::Hash(GcPtr::new(set))))
        }
        RegionAllocKind::List => {
            let items: Vec<Value> = operands.iter().map(|v| regs.get_cloned(*v)).collect();
            let list = PersistentList::from_iter(items);
            Ok(Value::List(region_alloc_val(target, list)))
        }
        RegionAllocKind::Cons => {
            if operands.len() == 2 {
                let h = regs.get_cloned(operands[0]);
                let t = regs.get_cloned(operands[1]);
                let cons = CljxCons { head: h, tail: t };
                Ok(Value::Cons(region_alloc_val(target, cons)))
            } else {
                Ok(Value::Nil)
            }
        }
    }
}

// ── Closure construction ────────────────────────────────────────────────────

fn alloc_closure(
    template: &ClosureTemplate,
    capture_vars: &[VarId],
    regs: &Registers,
    ir_func: &IrFunction,
    globals: &Arc<GlobalEnv>,
    ns: &Arc<str>,
) -> EvalResult {
    let captured_values: Vec<Value> = capture_vars.iter().map(|v| regs.get_cloned(*v)).collect();

    // Build a CljxFn-like wrapper using NativeFunction.
    // Each arity maps to a named subfunction in ir_func.subfunctions.
    // We use arity_fn_names (when present) to match each arity slot to its
    // correct subfunction by name, rather than relying on positional indexing.
    // Positional indexing is wrong when the parent function has more
    // subfunctions than arities (e.g. fnil has two inner lambdas but the
    // returned closure is only the second one).
    let subfuncs: Vec<Arc<IrFunction>> = if !template.arity_fn_names.is_empty() {
        template
            .arity_fn_names
            .iter()
            .map(|fn_name| {
                let sf = ir_func
                    .subfunctions
                    .iter()
                    .find(|sf| sf.name.as_deref() == Some(fn_name.as_ref()))
                    .unwrap_or_else(|| {
                        ir_func.subfunctions.first().unwrap_or_else(|| {
                            panic!("IR closure: no subfunctions for '{fn_name}'")
                        })
                    });
                Arc::new(clone_ir_function(sf))
            })
            .collect()
    } else {
        ir_func
            .subfunctions
            .iter()
            .map(clone_ir_function)
            .map(Arc::new)
            .collect()
    };

    let param_counts = template.param_counts.clone();
    let is_variadic = template.is_variadic.clone();
    let fn_name = template.name.clone();
    let closure_ns = ns.clone();
    let closure_globals = globals.clone();

    // Create a native function that dispatches to the IR interpreter.
    let nf = NativeFn {
        name: fn_name.as_deref().unwrap_or("<ir-closure>").into(),
        arity: if param_counts.len() == 1 && !is_variadic[0] {
            cljrs_value::Arity::Fixed(param_counts[0])
        } else {
            cljrs_value::Arity::Variadic {
                min: param_counts.iter().copied().min().unwrap_or(0),
            }
        },
        func: Arc::new({
            let captured_values = captured_values.clone();
            let subfuncs = subfuncs.clone();
            let param_counts = param_counts.clone();
            let is_variadic = is_variadic.clone();
            let closure_globals = closure_globals.clone();
            let closure_ns = closure_ns.clone();
            move |call_args: &[Value]| {
                // Select the right arity.
                let nargs = call_args.len();
                let mut best_idx = None;
                for (i, &pc) in param_counts.iter().enumerate() {
                    if is_variadic[i] && nargs >= pc {
                        // Variadic: accept pc or more.
                        match best_idx {
                            None => best_idx = Some(i),
                            Some(prev) => {
                                if pc > param_counts[prev] {
                                    best_idx = Some(i);
                                }
                            }
                        }
                    } else if !is_variadic[i] && nargs == pc {
                        best_idx = Some(i);
                        break;
                    }
                }

                let idx = best_idx.ok_or_else(|| {
                    cljrs_value::ValueError::Other(format!(
                        "Wrong number of args ({nargs}) passed to IR closure"
                    ))
                })?;

                let subfunc = &subfuncs[idx];

                // Build full args: captures + call_args (+ rest list for variadic).
                let mut full_args = captured_values.clone();
                if is_variadic[idx] {
                    let pc = param_counts[idx];
                    // Fixed params.
                    for a in call_args[..pc.min(nargs)].iter() {
                        full_args.push(a.clone());
                    }
                    // Rest as a list.
                    let rest: Vec<Value> = if nargs > pc {
                        call_args[pc..].to_vec()
                    } else {
                        Vec::new()
                    };
                    full_args.push(Value::List(GcPtr::new(PersistentList::from_iter(rest))));
                } else {
                    full_args.extend_from_slice(call_args);
                }

                // Call back into the interpreter via callback::invoke infrastructure.
                // We need an Env, which we get from the callback context.
                let result = cljrs_env::callback::with_eval_context(|env| {
                    interpret_ir(subfunc, full_args, &closure_globals, &closure_ns, env)
                });
                match result {
                    Ok(v) => Ok(v),
                    Err(EvalError::Runtime(msg)) => Err(cljrs_value::ValueError::Other(msg)),
                    Err(EvalError::Thrown(v)) => Err(cljrs_value::ValueError::Thrown(v)),
                    Err(EvalError::Recur(vals)) => Err(cljrs_value::ValueError::Other(format!(
                        "recur from non-tail position ({} values)",
                        vals.len()
                    ))),
                    Err(other) => Err(cljrs_value::ValueError::Other(format!("{other}"))),
                }
            }
        }),
    };

    Ok(Value::NativeFunction(GcPtr::new(nf)))
}

/// Deep clone an IrFunction (it doesn't implement Clone due to Debug derive).
fn clone_ir_function(f: &IrFunction) -> IrFunction {
    IrFunction {
        name: f.name.clone(),
        params: f.params.clone(),
        blocks: f.blocks.clone(),
        next_var: f.next_var,
        next_block: f.next_block,
        span: f.span.clone(),
        subfunctions: f.subfunctions.iter().map(clone_ir_function).collect(),
        is_async: f.is_async,
        is_async_poll_fn: f.is_async_poll_fn,
        async_resume_blocks: f.async_resume_blocks.clone(),
        seed_reprs: f.seed_reprs.clone(),
        local_seed_reprs: f.local_seed_reprs.clone(),
    }
}

// ── Sentinel-aware call dispatch ────────────────────────────────────────────

/// Dispatch a generic callee value, intercepting sentinel `NativeFunction`s.
///
/// Several clojure.core entries (volatile!, vswap!, make-delay, etc.) are
/// sentinel stubs that unconditionally error when called normally — the real
/// work happens in `eval_call`'s special-form dispatch, which the IR
/// interpreter bypasses.  We intercept those by name here so that IR code
/// calling them works correctly.
fn dispatch_or_sentinel(
    callee: Value,
    args: Vec<Value>,
    globals: &Arc<GlobalEnv>,
    ns: &Arc<str>,
    env: &mut Env,
) -> EvalResult {
    if let Value::NativeFunction(nf) = &callee
        && is_sentinel(nf.get().name.as_ref())
    {
        return dispatch_sentinel_by_name(nf.get().name.as_ref(), args, globals, ns, env);
    }
    apply_value(&callee, args, env)
}

/// Dispatch a call to a sentinel operation by name, or fall through to a
/// global lookup + `apply_value` for non-sentinel names.
fn dispatch_sentinel_by_name(
    name: &str,
    args: Vec<Value>,
    globals: &Arc<GlobalEnv>,
    ns: &Arc<str>,
    env: &mut Env,
) -> EvalResult {
    match name {
        "volatile!" => cljrs_interp::apply::eval_volatile(args),
        "reset!" => cljrs_interp::apply::eval_reset_bang(args, env),
        "vreset!" => cljrs_interp::apply::eval_vreset_bang(args),
        "vswap!" => cljrs_interp::apply::eval_vswap_bang(args, env),
        "make-delay" => {
            let f = args.into_iter().next().ok_or_else(|| EvalError::Arity {
                name: "make-delay".into(),
                expected: "1".into(),
                got: 0,
            })?;
            cljrs_interp::apply::make_delay_from_fn(&f, globals.clone(), ns.clone())
        }
        "alter-var-root" => cljrs_interp::apply::eval_alter_var_root(args, env),
        "vary-meta" => cljrs_interp::apply::eval_vary_meta(args, env),
        "with-bindings*" => cljrs_interp::apply::eval_with_bindings_star(args, env),
        "send" | "send-off" => cljrs_interp::apply::eval_send_to_agent(args, env),
        _ => {
            let callee = load_global_value(globals, ns, name, ns)?;
            apply_value(&callee, args, env)
        }
    }
}

fn is_sentinel(name: &str) -> bool {
    matches!(
        name,
        "volatile!"
            | "reset!"
            | "vreset!"
            | "vswap!"
            | "make-delay"
            | "alter-var-root"
            | "vary-meta"
            | "with-bindings*"
            | "send"
            | "send-off"
    )
}

// ── KnownFn dispatch ────────────────────────────────────────────────────────

/// Dispatch a call to a known built-in function.
///
/// This maps each `KnownFn` variant to the corresponding Rust builtin
/// from `builtins.rs`, or falls back to `apply_value` for complex cases.
fn dispatch_known_fn(known_fn: &KnownFn, args: Vec<Value>, env: &mut Env) -> EvalResult {
    match known_fn {
        // ── Arithmetic ──────────────────────────────────────────────────
        KnownFn::Add => builtin_arith(&args, "+"),
        KnownFn::Sub => builtin_arith(&args, "-"),
        KnownFn::Mul => builtin_arith(&args, "*"),
        KnownFn::Div => builtin_arith(&args, "/"),
        KnownFn::Rem => builtin_arith(&args, "rem"),
        KnownFn::UncheckedAdd => builtin_arith(&args, "unchecked-add"),
        KnownFn::UncheckedSub => builtin_arith(&args, "unchecked-subtract"),
        KnownFn::UncheckedMul => builtin_arith(&args, "unchecked-multiply"),

        // ── Comparison ──────────────────────────────────────────────────
        KnownFn::Eq => Ok(Value::Bool(args.len() == 2 && args[0] == args[1])),
        KnownFn::Lt | KnownFn::Gt | KnownFn::Lte | KnownFn::Gte => builtin_compare(known_fn, &args),
        KnownFn::Identical => Ok(Value::Bool(
            args.len() == 2 && std::ptr::eq(&args[0] as *const _, &args[1] as *const _),
        )),

        // ── Type predicates ─────────────────────────────────────────────
        KnownFn::IsNil => Ok(Value::Bool(matches!(args.first(), Some(Value::Nil)))),
        KnownFn::IsSeq => Ok(Value::Bool(matches!(
            args.first(),
            Some(Value::List(_) | Value::Cons(_) | Value::LazySeq(_))
        ))),
        KnownFn::IsVector => Ok(Value::Bool(matches!(args.first(), Some(Value::Vector(_))))),
        KnownFn::IsMap => Ok(Value::Bool(matches!(args.first(), Some(Value::Map(_))))),
        KnownFn::IsNumber => Ok(Value::Bool(matches!(
            args.first(),
            Some(
                Value::Long(_)
                    | Value::Double(_)
                    | Value::BigInt(_)
                    | Value::Ratio(_)
                    | Value::BigDecimal(_)
            )
        ))),
        KnownFn::IsString => Ok(Value::Bool(matches!(args.first(), Some(Value::Str(_))))),
        KnownFn::IsKeyword => Ok(Value::Bool(matches!(args.first(), Some(Value::Keyword(_))))),
        KnownFn::IsSymbol => Ok(Value::Bool(matches!(args.first(), Some(Value::Symbol(_))))),
        KnownFn::IsBool => Ok(Value::Bool(matches!(args.first(), Some(Value::Bool(_))))),
        KnownFn::IsInt => Ok(Value::Bool(matches!(args.first(), Some(Value::Long(_))))),

        // ── String ──────────────────────────────────────────────────────
        KnownFn::Str => {
            let s: String = args
                .iter()
                .map(|v| format!("{}", cljrs_value::value::PrintValue(v)))
                .collect();
            Ok(Value::Str(GcPtr::new(s)))
        }

        // ── Collection construction ─────────────────────────────────────
        KnownFn::Vector => Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter(args)))),
        KnownFn::HashMap => {
            let pairs: Vec<(Value, Value)> = args
                .chunks(2)
                .map(|c| (c[0].clone(), c.get(1).cloned().unwrap_or(Value::Nil)))
                .collect();
            Ok(Value::Map(MapValue::from_pairs(pairs)))
        }
        KnownFn::HashSet => Ok(Value::Set(SetValue::Hash(GcPtr::new(
            PersistentHashSet::from_iter(args),
        )))),
        KnownFn::List => Ok(Value::List(GcPtr::new(PersistentList::from_iter(args)))),

        // ── Collection operations ───────────────────────────────────────
        KnownFn::Get => {
            let result = builtin_call_native("get", &args)?;
            Ok(result)
        }
        KnownFn::Nth => builtin_call_native("nth", &args),
        KnownFn::NthLenient => {
            // Destructuring nth: out-of-bounds yields nil, never throws.  The
            // 3-arg `nth` returns its default (nil here) for a short collection.
            let mut args = args;
            args.push(Value::Nil);
            builtin_call_native("nth", &args)
        }
        KnownFn::Aget => builtin_call_native("aget", &args),
        KnownFn::Aset => builtin_call_native("aset", &args),
        KnownFn::Alength => builtin_call_native("alength", &args),
        KnownFn::Count => builtin_call_native("count", &args),
        KnownFn::CountFilter => {
            // Synthesized fused op == (count (filter pred coll)).
            let filter_fn = load_builtin(env, "filter")?;
            let seq = apply_value(&filter_fn, args, env)?;
            builtin_call_native("count", &[seq])
        }
        KnownFn::IntoFilter | KnownFn::IntoMapcat | KnownFn::IntoMap => {
            // Synthesized fused ops == (into to (filter|mapcat|map f coll)).
            let hof = match known_fn {
                KnownFn::IntoFilter => "filter",
                KnownFn::IntoMapcat => "mapcat",
                _ => "map",
            };
            let mut args = args;
            let to = args.remove(0);
            let hof_fn = load_builtin(env, hof)?;
            let seq = apply_value(&hof_fn, args, env)?;
            let into_fn = load_builtin(env, "into")?;
            apply_value(&into_fn, vec![to, seq], env)
        }
        KnownFn::Contains => builtin_call_native("contains?", &args),
        KnownFn::Assoc => builtin_call_native("assoc", &args),
        KnownFn::Dissoc => builtin_call_native("dissoc", &args),
        KnownFn::Conj => builtin_call_native("conj", &args),
        KnownFn::Disj => builtin_call_native("disj", &args),
        KnownFn::First => builtin_call_native("first", &args),
        KnownFn::Rest => builtin_call_native("rest", &args),
        KnownFn::Next => builtin_call_native("next", &args),
        KnownFn::Cons => builtin_call_native("cons", &args),
        KnownFn::Seq => builtin_call_native("seq", &args),
        KnownFn::Keys => builtin_call_native("keys", &args),
        KnownFn::Vals => builtin_call_native("vals", &args),
        KnownFn::Merge => builtin_call_native("merge", &args),
        KnownFn::Update => builtin_call_native("update", &args),
        KnownFn::GetIn => builtin_call_native("get-in", &args),
        KnownFn::AssocIn => builtin_call_native("assoc-in", &args),
        KnownFn::Concat => builtin_call_native("concat", &args),
        KnownFn::Reverse => builtin_call_native("reverse", &args),
        KnownFn::Frequencies => builtin_call_native("frequencies", &args),
        KnownFn::Zipmap => builtin_call_native("zipmap", &args),

        // ── Transient operations ────────────────────────────────────────
        KnownFn::Transient => builtin_call_native("transient", &args),
        KnownFn::AssocBang => builtin_call_native("assoc!", &args),
        KnownFn::ConjBang => builtin_call_native("conj!", &args),
        KnownFn::PersistentBang => builtin_call_native("persistent!", &args),

        // ── Sequence operations ─────────────────────────────────────────
        KnownFn::Take => builtin_call_native("take", &args),
        KnownFn::Drop => builtin_call_native("drop", &args),
        KnownFn::Range1 | KnownFn::Range2 | KnownFn::Range3 => builtin_call_native("range", &args),
        KnownFn::LazySeq => {
            // `(lazy-seq f)` wraps a zero-arg Clojure fn in a LazySeq.  The
            // builtin `make-lazy-seq` registered in clojure.core is a
            // sentinel that intentionally errors (it's only callable
            // through eval_call's special dispatch); from the IR
            // interpreter we need to construct the LazySeq directly.
            if let Some(f) = args.first() {
                cljrs_env::callback::with_eval_context(|env| {
                    cljrs_interp::apply::make_lazy_seq_from_fn(
                        f,
                        env.globals.clone(),
                        env.current_ns.clone(),
                    )
                })
            } else {
                Ok(Value::Nil)
            }
        }

        // ── Atom operations ─────────────────────────────────────────────
        KnownFn::Atom => builtin_call_native("atom", &args),
        KnownFn::Deref | KnownFn::AtomDeref => {
            if let Some(v) = args.into_iter().next() {
                cljrs_interp::eval::deref_value(v)
            } else {
                Ok(Value::Nil)
            }
        }
        KnownFn::AtomReset => cljrs_interp::apply::eval_reset_bang(args, env),
        KnownFn::AtomSwap => cljrs_interp::apply::eval_swap_bang(args, env),

        // ── I/O ─────────────────────────────────────────────────────────
        KnownFn::Println => builtin_call_native("println", &args),
        KnownFn::Pr => builtin_call_native("pr", &args),
        KnownFn::Prn => builtin_call_native("prn", &args),
        KnownFn::Print => builtin_call_native("print", &args),

        // ── HOFs (need env for callbacks) ───────────────────────────────
        KnownFn::Map
        | KnownFn::Filter
        | KnownFn::Mapv
        | KnownFn::Filterv
        | KnownFn::Reduce2
        | KnownFn::Reduce3
        | KnownFn::Some
        | KnownFn::Every
        | KnownFn::Into
        | KnownFn::Into3
        | KnownFn::Sort
        | KnownFn::SortBy
        | KnownFn::GroupBy
        | KnownFn::Partition2
        | KnownFn::Partition3
        | KnownFn::Partition4
        | KnownFn::Keep
        | KnownFn::Remove
        | KnownFn::MapIndexed
        | KnownFn::Juxt
        | KnownFn::Comp
        | KnownFn::Partial
        | KnownFn::Complement => {
            let fn_name = known_fn_to_name(known_fn);
            let callee = load_builtin(env, fn_name)?;
            apply_value(&callee, args, env)
        }

        KnownFn::Apply => {
            if args.len() < 2 {
                return Err(EvalError::Arity {
                    name: "apply".into(),
                    expected: "2+".into(),
                    got: args.len(),
                });
            }
            let mut args = args;
            let f = args.remove(0);
            let last = args.pop().unwrap();
            let spread = cljrs_interp::destructure::value_to_seq_vec(&last);
            args.extend(spread);
            apply_value(&f, args, env)
        }

        // ── Dynamic binding / exception handling ────────────────────────
        KnownFn::SetBangVar => builtin_call_native("set!", &args),
        KnownFn::WithBindings => eval_ir_with_bindings(args, env),
        KnownFn::TryCatchFinally => eval_ir_try_catch_finally(args, env),
        KnownFn::WithOutStr => {
            let fn_name = known_fn_to_name(known_fn);
            let callee = load_builtin(env, fn_name)?;
            apply_value(&callee, args, env)
        }
        // Analysis-only KnownFns: variants the analyzer cares about but
        // the interpreter has no specialised path for.  Fall back to a
        // dynamic lookup of the builtin by name.
        KnownFn::IsEmpty
        | KnownFn::Peek
        | KnownFn::Pop
        | KnownFn::Vec
        | KnownFn::Mapcat
        | KnownFn::Repeatedly => {
            let fn_name = known_fn_to_name(known_fn);
            let callee = load_builtin(env, fn_name)?;
            apply_value(&callee, args, env)
        }
    }
}

// ── Helpers for KnownFn dispatch ────────────────────────────────────────────

/// Handle `KnownFn::WithBindings` emitted by `lower-binding`.
///
/// The ANF lowerer emits flat args `[var0, val0, var1, val1, ..., body-fn]`.
/// This is different from the `with-bindings*` public API which takes a map,
/// so we assemble the frame here rather than delegating to eval_with_bindings_star.
fn eval_ir_with_bindings(args: Vec<Value>, env: &mut Env) -> EvalResult {
    use std::collections::HashMap;
    if args.is_empty() {
        return Err(EvalError::Arity {
            name: "with-bindings".into(),
            expected: "1+".into(),
            got: 0,
        });
    }
    // Last arg is the body thunk; preceding args are (Var, value) pairs.
    let body = args.last().unwrap().clone();
    let pairs = &args[..args.len() - 1];
    if !pairs.len().is_multiple_of(2) {
        return Err(EvalError::Runtime(
            "with-bindings: odd number of var/val pairs".into(),
        ));
    }
    let mut frame: HashMap<usize, Value> = HashMap::new();
    for chunk in pairs.chunks(2) {
        if let Value::Var(vp) = &chunk[0] {
            frame.insert(cljrs_env::dynamics::var_key_of(vp), chunk[1].clone());
        } else {
            return Err(EvalError::Runtime(format!(
                "with-bindings: binding key must be a Var, got {}",
                chunk[0].type_name()
            )));
        }
    }
    let _guard = cljrs_env::dynamics::push_frame(frame);
    cljrs_env::apply::apply_value(&body, vec![], env)
}

/// Handle `KnownFn::TryCatchFinally` emitted by `lower_try`.
///
/// Args are `[body-fn, catch-fn-or-nil, finally-fn-or-nil]`.  The body thunk
/// is called with no arguments.  On a thrown exception the catch thunk (if not
/// nil) is called with the exception value as its sole argument.  The finally
/// thunk (if not nil) is always called with no arguments before returning.
fn eval_ir_try_catch_finally(args: Vec<Value>, env: &mut Env) -> EvalResult {
    let body = args.first().cloned().unwrap_or(Value::Nil);
    let catch_fn = args.get(1).cloned().unwrap_or(Value::Nil);
    let finally_fn = args.get(2).cloned().unwrap_or(Value::Nil);

    let body_result = apply_value(&body, vec![], env);

    let ret = match body_result {
        Ok(val) => Ok(val),
        Err(EvalError::Thrown(thrown_val)) => {
            if matches!(catch_fn, Value::Nil) {
                Err(EvalError::Thrown(thrown_val))
            } else {
                apply_value(&catch_fn, vec![thrown_val], env)
            }
        }
        err => err,
    };

    if !matches!(finally_fn, Value::Nil) {
        let _ = apply_value(&finally_fn, vec![], env);
    }

    ret
}

/// Call a native builtin by name from the global environment.
fn builtin_call_native(name: &str, args: &[Value]) -> EvalResult {
    // Use the callback infrastructure to get an eval context.
    cljrs_env::callback::with_eval_context(|env| {
        let callee = load_builtin(env, name)?;
        if let Value::NativeFunction(nf) = &callee {
            (nf.get().func)(args).map_err(|e| EvalError::Runtime(e.to_string()))
        } else {
            apply_value(&callee, args.to_vec(), env)
        }
    })
}

/// Look up a builtin function by name in the global environment.
fn load_builtin(env: &Env, name: &str) -> EvalResult {
    env.globals
        .lookup_in_ns("clojure.core", name)
        .ok_or_else(|| EvalError::Runtime(format!("IR interpreter: builtin not found: {name}")))
}

/// Map KnownFn variants to their Clojure function names.
fn known_fn_to_name(kf: &KnownFn) -> &'static str {
    match kf {
        KnownFn::Map => "map",
        KnownFn::Filter => "filter",
        KnownFn::Mapv => "mapv",
        KnownFn::Filterv => "filterv",
        KnownFn::Reduce2 | KnownFn::Reduce3 => "reduce",
        KnownFn::Some => "some",
        KnownFn::Every => "every?",
        KnownFn::Into | KnownFn::Into3 => "into",
        KnownFn::Sort => "sort",
        KnownFn::SortBy => "sort-by",
        KnownFn::GroupBy => "group-by",
        KnownFn::Partition2 | KnownFn::Partition3 | KnownFn::Partition4 => "partition",
        KnownFn::Keep => "keep",
        KnownFn::Remove => "remove",
        KnownFn::MapIndexed => "map-indexed",
        KnownFn::Juxt => "juxt",
        KnownFn::Comp => "comp",
        KnownFn::Partial => "partial",
        KnownFn::Complement => "complement",
        KnownFn::Apply => "apply",
        KnownFn::WithBindings => "with-bindings*",
        KnownFn::WithOutStr => "with-out-str",
        KnownFn::TryCatchFinally => "try",
        KnownFn::SetBangVar => "set!",
        KnownFn::IsEmpty => "empty?",
        KnownFn::Peek => "peek",
        KnownFn::Pop => "pop",
        KnownFn::Vec => "vec",
        KnownFn::Mapcat => "mapcat",
        KnownFn::Repeatedly => "repeatedly",
        _ => "unknown",
    }
}

/// Arithmetic dispatch for +, -, *, /, rem.
fn builtin_arith(args: &[Value], op: &str) -> EvalResult {
    if args.len() != 2 {
        return builtin_call_native(op, args);
    }
    let (a, b) = (&args[0], &args[1]);
    match (a, b) {
        (Value::Long(x), Value::Long(y)) => match op {
            // Checked: primitive long arithmetic throws on overflow (matches
            // the compiled tier).  The wrapping variants are `unchecked-*`.
            "+" => x
                .checked_add(*y)
                .map(Value::Long)
                .ok_or_else(|| EvalError::Runtime("integer overflow".to_string())),
            "-" => x
                .checked_sub(*y)
                .map(Value::Long)
                .ok_or_else(|| EvalError::Runtime("integer overflow".to_string())),
            "*" => x
                .checked_mul(*y)
                .map(Value::Long)
                .ok_or_else(|| EvalError::Runtime("integer overflow".to_string())),
            "unchecked-add" => Ok(Value::Long(x.wrapping_add(*y))),
            "unchecked-subtract" => Ok(Value::Long(x.wrapping_sub(*y))),
            "unchecked-multiply" => Ok(Value::Long(x.wrapping_mul(*y))),
            "/" => {
                if *y == 0 {
                    Err(EvalError::Runtime("Divide by zero".to_string()))
                } else {
                    Ok(Value::Long(x / y))
                }
            }
            "rem" => {
                if *y == 0 {
                    Err(EvalError::Runtime("Divide by zero".to_string()))
                } else {
                    Ok(Value::Long(x % y))
                }
            }
            _ => builtin_call_native(op, args),
        },
        (Value::Double(x), Value::Double(y)) => match op {
            "+" | "unchecked-add" => Ok(Value::Double(x + y)),
            "-" | "unchecked-subtract" => Ok(Value::Double(x - y)),
            "*" | "unchecked-multiply" => Ok(Value::Double(x * y)),
            "/" => Ok(Value::Double(x / y)),
            "rem" => Ok(Value::Double(x % y)),
            _ => builtin_call_native(op, args),
        },
        (Value::Long(x), Value::Double(y)) => {
            let x = *x as f64;
            match op {
                "+" | "unchecked-add" => Ok(Value::Double(x + y)),
                "-" | "unchecked-subtract" => Ok(Value::Double(x - y)),
                "*" | "unchecked-multiply" => Ok(Value::Double(x * y)),
                "/" => Ok(Value::Double(x / y)),
                "rem" => Ok(Value::Double(x % y)),
                _ => builtin_call_native(op, args),
            }
        }
        (Value::Double(x), Value::Long(y)) => {
            let y = *y as f64;
            match op {
                "+" | "unchecked-add" => Ok(Value::Double(*x + y)),
                "-" | "unchecked-subtract" => Ok(Value::Double(*x - y)),
                "*" | "unchecked-multiply" => Ok(Value::Double(*x * y)),
                "/" => Ok(Value::Double(*x / y)),
                "rem" => Ok(Value::Double(*x % y)),
                _ => builtin_call_native(op, args),
            }
        }
        _ => builtin_call_native(op, args),
    }
}

/// Comparison dispatch for <, >, <=, >=.
fn builtin_compare(known_fn: &KnownFn, args: &[Value]) -> EvalResult {
    if args.len() != 2 {
        return Ok(Value::Bool(false));
    }
    let (a, b) = (&args[0], &args[1]);
    let result = match (a, b) {
        (Value::Long(x), Value::Long(y)) => match known_fn {
            KnownFn::Lt => x < y,
            KnownFn::Gt => x > y,
            KnownFn::Lte => x <= y,
            KnownFn::Gte => x >= y,
            _ => false,
        },
        (Value::Double(x), Value::Double(y)) => match known_fn {
            KnownFn::Lt => x < y,
            KnownFn::Gt => x > y,
            KnownFn::Lte => x <= y,
            KnownFn::Gte => x >= y,
            _ => false,
        },
        (Value::Long(x), Value::Double(y)) => {
            let x = *x as f64;
            match known_fn {
                KnownFn::Lt => x < *y,
                KnownFn::Gt => x > *y,
                KnownFn::Lte => x <= *y,
                KnownFn::Gte => x >= *y,
                _ => false,
            }
        }
        (Value::Double(x), Value::Long(y)) => {
            let y = *y as f64;
            match known_fn {
                KnownFn::Lt => *x < y,
                KnownFn::Gt => *x > y,
                KnownFn::Lte => *x <= y,
                KnownFn::Gte => *x >= y,
                _ => false,
            }
        }
        _ => {
            // Delegate to the native comparison function for all other types
            // (BigInt, Ratio, mixed, etc.) so they compare correctly.
            let op = match known_fn {
                KnownFn::Lt => "<",
                KnownFn::Gt => ">",
                KnownFn::Lte => "<=",
                KnownFn::Gte => ">=",
                _ => return Ok(Value::Bool(false)),
            };
            return builtin_call_native(op, args);
        }
    };
    Ok(Value::Bool(result))
}

// IR lowering

/// Eagerly lower all arities of a function to IR, storing the results
/// in the IR cache.  Failures are silently recorded as `Unsupported`.
///
/// Only lowers if the compiler is already loaded (`compiler_ready` is true).
/// Compiler loading is triggered separately (e.g., by the binary at startup).
pub(crate) fn eager_lower_fn(f: &CljxFn, env: &mut Env) {
    use crate::apply::IR_LOWERING_ACTIVE;
    let mut lowered = 0;
    let mut cached = 0;
    let mut failed = 0;

    // Skip if eager lowering is disabled.
    if !crate::apply::eager_lower_enabled() {
        return;
    }

    cljrs_logging::feat_trace!("ir", "eager_lower_fn {:?}", f.name);

    // Don't lower macros (they operate on forms, not values).
    if f.is_macro {
        cljrs_logging::feat_debug!("ir", "not lowering macro: {:?}", f.name);
        return;
    }

    // Only lower if compiler is already ready (don't trigger loading).
    if !env
        .globals
        .compiler_ready
        .load(std::sync::atomic::Ordering::Acquire)
    {
        cljrs_logging::feat_debug!("ir", "compiler not ready, not lowering");
        return;
    }

    // Don't lower closures that capture variables from an enclosing scope.
    // lower-fn-body only knows about the explicit arity params; captured names
    // are invisible to it, so any reference to a capture would be emitted as
    // LoadGlobal(defining-ns, name) — which either resolves to the wrong var or
    // fails at runtime with "var not found".  Top-level defns have no captures,
    // so they are safe to lower.  Inner closures will fall back to tree-walking.
    if !f.closed_over_names.is_empty() {
        return;
    }

    // Don't nest lowering calls.
    if IR_LOWERING_ACTIVE.get() {
        cljrs_logging::feat_trace!("ir", "lowering active, not continuing");
        return;
    }

    IR_LOWERING_ACTIVE.set(true);

    // Arities lowered in this call, collected for the cross-defn registry
    // (param_count, is_variadic, ir).
    let mut registered: Vec<(usize, bool, Arc<IrFunction>)> = Vec::new();

    for arity in &f.arities {
        let arity_id = arity.ir_arity_id;
        if !crate::ir_cache::should_attempt(arity_id) {
            cached += 1;
            // Keep already-cached arities in the registration so a partial
            // re-lower doesn't drop them from the cross-defn registry.
            if let Some(ir) = crate::ir_cache::get_cached(arity_id) {
                registered.push((arity.params.len(), arity.rest_param.is_some(), ir));
            }
            continue;
        }

        // Destructured params are expanded into explicit IR-prologue bindings by
        // lower_and_optimize_arity (passing the patterns below), so they no
        // longer force a tree-walk fallback.

        match crate::lower::lower_and_optimize_arity_tracked(
            f.name.as_deref(),
            &arity.params,
            arity.rest_param.as_ref(),
            &arity.destructure_params,
            arity.destructure_rest.as_ref(),
            &arity.body,
            &f.defining_ns,
            env,
            f.is_async,
        ) {
            Ok((mut ir_func, used_externals)) => {
                // Attach static primitive type-hint seeds so the JIT can skip
                // its profiling warmup and guard/unbox these params directly.
                ir_func.seed_reprs = crate::lower::seed_reprs_from_hints(&arity.param_hints);
                let ir_func = Arc::new(ir_func);
                crate::ir_cache::store_cached(arity_id, ir_func.clone());
                // The lowering specialized against these defns — invalidate
                // it (and re-lower lazily) if any of them is rebound.
                crate::defn_registry::record_dependents(arity_id, used_externals);
                registered.push((arity.params.len(), arity.rest_param.is_some(), ir_func));
                lowered += 1;
            }
            Err(_) => {
                crate::ir_cache::store_unsupported(arity_id);
                failed += 1;
            }
        }
    }

    // Publish this defn so later lowerings of *other* functions can
    // region-promote calls into it (stage 4).  Anonymous, async, or capturing
    // fns are not callable cross-defn by name, so skip them.
    if !f.is_async
        && !registered.is_empty()
        && let Some(name) = f.name.as_deref()
    {
        crate::defn_registry::install_invalidation_hook();
        crate::defn_registry::register_defn(
            crate::lower::globals_id(env),
            &f.defining_ns,
            &Arc::from(name),
            registered,
        );
    }

    cljrs_logging::feat_debug!(
        "ir",
        "ir complete {:?} lowered:{} cached:{} failed:{}",
        f.name,
        lowered,
        cached,
        failed
    );

    IR_LOWERING_ACTIVE.set(false);
}

#[cfg(test)]
mod arith_tests {
    use super::builtin_arith;
    use cljrs_value::Value;

    #[test]
    fn checked_add_overflow_throws() {
        let r = builtin_arith(&[Value::Long(i64::MAX), Value::Long(1)], "+");
        assert!(r.is_err(), "checked + overflow must throw");
    }

    #[test]
    fn checked_mul_overflow_throws() {
        let r = builtin_arith(&[Value::Long(i64::MAX), Value::Long(2)], "*");
        assert!(r.is_err(), "checked * overflow must throw");
    }

    #[test]
    fn checked_add_normal_ok() {
        let r = builtin_arith(&[Value::Long(3), Value::Long(4)], "+").unwrap();
        assert_eq!(r, Value::Long(7));
    }

    #[test]
    fn unchecked_add_wraps() {
        let r = builtin_arith(&[Value::Long(i64::MAX), Value::Long(1)], "unchecked-add").unwrap();
        assert_eq!(r, Value::Long(i64::MIN));
    }

    #[test]
    fn unchecked_multiply_wraps() {
        let r = builtin_arith(
            &[Value::Long(i64::MAX), Value::Long(2)],
            "unchecked-multiply",
        )
        .unwrap();
        assert_eq!(r, Value::Long(-2));
    }
}
