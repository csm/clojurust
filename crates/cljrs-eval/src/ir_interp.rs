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
struct Registers {
    values: Vec<Option<Value>>,
}

impl Registers {
    fn new(capacity: u32) -> Self {
        Self {
            values: vec![None; capacity as usize],
        }
    }

    fn get(&self, id: VarId) -> &Value {
        self.values[id.0 as usize]
            .as_ref()
            .unwrap_or_else(|| panic!("IR interpreter: uninitialized register {id}"))
    }

    fn set(&mut self, id: VarId, val: Value) {
        let idx = id.0 as usize;
        if idx >= self.values.len() {
            self.values.resize(idx + 1, None);
        }
        self.values[idx] = Some(val);
    }

    fn get_cloned(&self, id: VarId) -> Value {
        self.get(id).clone()
    }
}

// ── Region state ────────────────────────────────────────────────────────────

/// A region entry in the interpreter's local region stack.
struct RegionEntry {
    _region: Box<cljrs_gc::region::Region>,
    // The region is also pushed onto the cljrs_gc thread-local REGION_STACK
    // via push_region_raw.  We track it here for cleanup.
}

impl Drop for RegionEntry {
    fn drop(&mut self) {
        // Pop from cljrs_gc's thread-local stack.
        cljrs_gc::region::pop_region_guard();
        // Box<Region> drop handles destructor execution and chunk freeing.
    }
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
    // GC safepoint at function entry.
    cljrs_env::gc_roots::gc_safepoint(env);

    let mut regs = Registers::new(ir_func.next_var);
    let mut region_stack: Vec<RegionEntry> = Vec::new();

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

        // Execute instructions.
        for inst in &block.insts {
            execute_inst(
                inst,
                &mut regs,
                &mut region_stack,
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
    region_stack: &mut Vec<RegionEntry>,
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
            let val = regs.get_cloned(*val_var);
            globals.intern(def_ns, name.clone(), val.clone());
            let var = globals
                .lookup_var_in_ns(def_ns, name)
                .expect("just interned");
            regs.set(*dst, Value::Var(var));
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
            region_stack.push(RegionEntry { _region: region });
            regs.set(*dst, Value::Nil);
        }

        Inst::RegionAlloc(dst, _region, kind, operands) => {
            let val = alloc_in_region(*kind, operands, regs)?;
            regs.set(*dst, val);
        }

        Inst::RegionEnd(_region) => {
            // Pop and drop the region entry (Drop impl handles cleanup).
            region_stack.pop();
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
        Const::Symbol(s) => Value::Symbol(GcPtr::new(cljrs_value::Symbol {
            namespace: None,
            name: s.clone(),
        })),
        Const::Char(c) => Value::Char(*c),
    }
}

// ── Global value lookup ─────────────────────────────────────────────────────

fn load_global_value(globals: &GlobalEnv, ns: &str, name: &str, defining_ns: &str) -> EvalResult {
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
    Err(EvalError::Runtime(format!(
        "IR interpreter: var not found {resolved_ns}/{name}"
    )))
}

// ── Region-aware allocation ─────────────────────────────────────────────────

fn alloc_in_region(kind: RegionAllocKind, operands: &[VarId], regs: &Registers) -> EvalResult {
    match kind {
        RegionAllocKind::Vector => {
            let items: Vec<Value> = operands.iter().map(|v| regs.get_cloned(*v)).collect();
            let pv = PersistentVector::from_iter(items);
            let ptr = if cljrs_gc::region::region_is_active() {
                unsafe { cljrs_gc::region::try_alloc_in_region(pv).unwrap() }
            } else {
                GcPtr::new(pv)
            };
            Ok(Value::Vector(ptr))
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
            let ptr = if cljrs_gc::region::region_is_active() {
                unsafe { cljrs_gc::region::try_alloc_in_region(list).unwrap() }
            } else {
                GcPtr::new(list)
            };
            Ok(Value::List(ptr))
        }
        RegionAllocKind::Cons => {
            if operands.len() == 2 {
                let h = regs.get_cloned(operands[0]);
                let t = regs.get_cloned(operands[1]);
                let cons = CljxCons { head: h, tail: t };
                let ptr = if cljrs_gc::region::region_is_active() {
                    unsafe { cljrs_gc::region::try_alloc_in_region(cons).unwrap() }
                } else {
                    GcPtr::new(cons)
                };
                Ok(Value::Cons(ptr))
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
    // Each arity maps to a subfunction in ir_func.subfunctions.
    let subfuncs: Vec<Arc<IrFunction>> = ir_func
        .subfunctions
        .iter()
        .map(clone_ir_function)
        .map(Arc::new)
        .collect();

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
        KnownFn::IsInt => Ok(Value::Bool(matches!(
            args.first(),
            Some(Value::Long(_) | Value::BigInt(_))
        ))),

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
        KnownFn::Count => builtin_call_native("count", &args),
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
        KnownFn::AtomReset => builtin_call_native("reset!", &args),
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
        KnownFn::WithBindings => cljrs_interp::apply::eval_with_bindings_star(args, env),
        KnownFn::WithOutStr | KnownFn::TryCatchFinally => {
            let fn_name = known_fn_to_name(known_fn);
            let callee = load_builtin(env, fn_name)?;
            apply_value(&callee, args, env)
        }
    }
}

// ── Helpers for KnownFn dispatch ────────────────────────────────────────────

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
            "+" => Ok(Value::Long(x.wrapping_add(*y))),
            "-" => Ok(Value::Long(x.wrapping_sub(*y))),
            "*" => Ok(Value::Long(x.wrapping_mul(*y))),
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
            "+" => Ok(Value::Double(x + y)),
            "-" => Ok(Value::Double(x - y)),
            "*" => Ok(Value::Double(x * y)),
            "/" => Ok(Value::Double(x / y)),
            "rem" => Ok(Value::Double(x % y)),
            _ => builtin_call_native(op, args),
        },
        (Value::Long(x), Value::Double(y)) => {
            let x = *x as f64;
            match op {
                "+" => Ok(Value::Double(x + y)),
                "-" => Ok(Value::Double(x - y)),
                "*" => Ok(Value::Double(x * y)),
                "/" => Ok(Value::Double(x / y)),
                "rem" => Ok(Value::Double(x % y)),
                _ => builtin_call_native(op, args),
            }
        }
        (Value::Double(x), Value::Long(y)) => {
            let y = *y as f64;
            match op {
                "+" => Ok(Value::Double(*x + y)),
                "-" => Ok(Value::Double(*x - y)),
                "*" => Ok(Value::Double(*x * y)),
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

    // Skip if eager lowering is disabled.
    if !crate::apply::eager_lower_enabled() {
        return;
    }

    // Don't lower macros (they operate on forms, not values).
    if f.is_macro {
        return;
    }

    // Only lower if compiler is already ready (don't trigger loading).
    if !env
        .globals
        .compiler_ready
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return;
    }

    // Don't nest lowering calls.
    if IR_LOWERING_ACTIVE.get() {
        return;
    }

    IR_LOWERING_ACTIVE.set(true);

    for arity in &f.arities {
        let arity_id = arity.ir_arity_id;
        if !crate::ir_cache::should_attempt(arity_id) {
            continue;
        }

        match crate::lower::lower_arity(
            f.name.as_deref(),
            &arity.params,
            arity.rest_param.as_ref(),
            &arity.body,
            &f.defining_ns,
            env,
        ) {
            Ok(ir_func) => {
                crate::ir_cache::store_cached(arity_id, std::sync::Arc::new(ir_func));
            }
            Err(_) => {
                crate::ir_cache::store_unsupported(arity_id);
            }
        }
    }

    IR_LOWERING_ACTIVE.set(false);
}
