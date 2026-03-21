//! Cranelift code generation: translate [`IrFunction`] to native machine code.
//!
//! The translator maps each IR instruction to calls into the C-ABI runtime
//! bridge (`rt_abi`).  Every Clojure `Value` is represented as an opaque
//! pointer (`I64` / `I32` depending on target) in the CLIF IR.

use std::collections::HashMap;
use std::sync::Arc;

use cranelift_codegen::ir::types;
use cranelift_codegen::ir::{AbiParam, BlockArg, InstBuilder};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::ir::{BlockId, Const, Inst, IrFunction, KnownFn, Terminator, VarId};

// ── Error type ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum CodegenError {
    Module(cranelift_module::ModuleError),
    Codegen(String),
}

impl From<cranelift_module::ModuleError> for CodegenError {
    fn from(e: cranelift_module::ModuleError) -> Self {
        CodegenError::Module(e)
    }
}

pub type CodegenResult<T> = Result<T, CodegenError>;

// ── Runtime function declarations ───────────────────────────────────────────

/// Cached `FuncId`s for runtime bridge functions.
struct RuntimeFuncs {
    rt_const_nil: FuncId,
    rt_const_true: FuncId,
    rt_const_false: FuncId,
    rt_const_long: FuncId,
    rt_const_double: FuncId,
    rt_const_char: FuncId,
    rt_const_string: FuncId,
    rt_const_keyword: FuncId,
    rt_const_symbol: FuncId,
    rt_truthiness: FuncId,
    rt_add: FuncId,
    rt_sub: FuncId,
    rt_mul: FuncId,
    rt_div: FuncId,
    rt_rem: FuncId,
    rt_eq: FuncId,
    rt_lt: FuncId,
    rt_gt: FuncId,
    rt_lte: FuncId,
    rt_gte: FuncId,
    rt_alloc_vector: FuncId,
    rt_alloc_map: FuncId,
    rt_alloc_set: FuncId,
    rt_alloc_list: FuncId,
    rt_alloc_cons: FuncId,
    rt_get: FuncId,
    rt_count: FuncId,
    rt_first: FuncId,
    rt_rest: FuncId,
    rt_assoc: FuncId,
    rt_conj: FuncId,
    rt_call: FuncId,
    rt_deref: FuncId,
    rt_println: FuncId,
    rt_pr: FuncId,
    rt_is_nil: FuncId,
    rt_is_vector: FuncId,
    rt_is_map: FuncId,
    rt_is_seq: FuncId,
    rt_identical: FuncId,
    rt_str: FuncId,
    rt_load_global: FuncId,
    rt_def_var: FuncId,
    rt_make_fn: FuncId,
    rt_throw: FuncId,
    rt_try: FuncId,
    rt_dissoc: FuncId,
    rt_disj: FuncId,
    rt_nth: FuncId,
    rt_contains: FuncId,
    rt_seq: FuncId,
    rt_lazy_seq: FuncId,
    rt_transient: FuncId,
    rt_assoc_bang: FuncId,
    rt_conj_bang: FuncId,
    rt_persistent_bang: FuncId,
    rt_atom_reset: FuncId,
    rt_atom_swap: FuncId,
    rt_apply: FuncId,
}

// ── Compiler context ────────────────────────────────────────────────────────

/// AOT compiler: translates IR functions to native object code via Cranelift.
pub struct Compiler {
    module: ObjectModule,
    ctx: cranelift_codegen::Context,
    fb_ctx: FunctionBuilderContext,
    rt: RuntimeFuncs,
    ptr_type: types::Type,
    /// Maps user-defined function names to their FuncIds.
    user_funcs: HashMap<Arc<str>, FuncId>,
}

impl Compiler {
    /// Create a new compiler targeting the host architecture.
    pub fn new() -> CodegenResult<Self> {
        let mut flag_builder = settings::builder();
        flag_builder.set("opt_level", "speed").unwrap();
        flag_builder.set("is_pic", "true").unwrap();

        let isa_builder = cranelift_native::builder()
            .map_err(|e| CodegenError::Codegen(format!("failed to create ISA builder: {e}")))?;
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .map_err(|e| CodegenError::Codegen(format!("failed to build ISA: {e}")))?;

        let ptr_type = isa.pointer_type();

        let obj_builder = ObjectBuilder::new(
            isa,
            "clojurust_aot",
            cranelift_module::default_libcall_names(),
        )?;
        let mut module = ObjectModule::new(obj_builder);

        let rt = declare_runtime_funcs(&mut module, ptr_type)?;

        Ok(Self {
            ctx: module.make_context(),
            fb_ctx: FunctionBuilderContext::new(),
            module,
            rt,
            ptr_type,
            user_funcs: HashMap::new(),
        })
    }

    /// Declare a user function (makes it available for calls before definition).
    pub fn declare_function(&mut self, name: &str, param_count: usize) -> CodegenResult<FuncId> {
        let mut sig = self.module.make_signature();
        for _ in 0..param_count {
            sig.params.push(AbiParam::new(self.ptr_type));
        }
        sig.returns.push(AbiParam::new(self.ptr_type));
        let func_id = self.module.declare_function(name, Linkage::Export, &sig)?;
        self.user_funcs.insert(Arc::from(name), func_id);
        Ok(func_id)
    }

    /// Compile an IR function and define it in the module.
    pub fn compile_function(&mut self, ir_func: &IrFunction, func_id: FuncId) -> CodegenResult<()> {
        self.ctx.func.signature = self
            .module
            .declarations()
            .get_function_decl(func_id)
            .signature
            .clone();
        self.ctx.func.name = cranelift_codegen::ir::UserFuncName::user(0, func_id.as_u32());

        {
            let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut self.fb_ctx);
            {
                let mut translator = FunctionTranslator {
                    builder: &mut builder,
                    module: &mut self.module,
                    rt: &self.rt,
                    ptr_type: self.ptr_type,
                    var_map: HashMap::new(),
                    block_map: HashMap::new(),
                    user_funcs: &self.user_funcs,
                };
                translator.translate(ir_func)?;
            }
            builder.finalize();
        }

        self.module.define_function(func_id, &mut self.ctx)?;
        self.ctx.clear();
        Ok(())
    }

    /// Finish compilation and return the object code bytes.
    pub fn finish(self) -> Vec<u8> {
        let product = self.module.finish();
        product.emit().expect("failed to emit object code")
    }
}

// ── Function translator ─────────────────────────────────────────────────────

/// Translates a single [`IrFunction`] into Cranelift IR using a
/// [`FunctionBuilder`].
struct FunctionTranslator<'a, 'b> {
    builder: &'b mut FunctionBuilder<'a>,
    module: &'b mut ObjectModule,
    rt: &'b RuntimeFuncs,
    ptr_type: types::Type,
    /// Maps IR VarId → Cranelift Variable.
    var_map: HashMap<VarId, Variable>,
    /// Maps function names → FuncId (for referencing compiled subfunctions).
    user_funcs: &'b HashMap<Arc<str>, FuncId>,
    /// Maps IR BlockId → Cranelift Block.
    block_map: HashMap<BlockId, cranelift_codegen::ir::Block>,
}

impl<'a, 'b> FunctionTranslator<'a, 'b> {
    fn translate(&mut self, ir_func: &IrFunction) -> CodegenResult<()> {
        // Create all CLIF blocks upfront.
        for block in &ir_func.blocks {
            let clif_block = self.builder.create_block();
            self.block_map.insert(block.id, clif_block);
        }

        // Entry block: append params.
        let entry_block = self.block_map[&ir_func.blocks[0].id];
        self.builder.switch_to_block(entry_block);
        self.builder
            .append_block_params_for_function_params(entry_block);

        // Bind function parameters to variables.
        for (i, (_name, var_id)) in ir_func.params.iter().enumerate() {
            let var = self.ensure_var(*var_id);
            let param_val = self.builder.block_params(entry_block)[i];
            self.builder.def_var(var, param_val);
        }

        // Translate each block.
        for (block_idx, ir_block) in ir_func.blocks.iter().enumerate() {
            let clif_block = self.block_map[&ir_block.id];

            if block_idx > 0 {
                self.builder.switch_to_block(clif_block);
            }

            // Phi nodes become block parameters in Cranelift.
            // We handle them specially: each phi adds a block parameter,
            // and predecessor jumps pass the right value.
            // For now, phi values are pre-declared as variables.
            for inst in &ir_block.phis {
                if let Inst::Phi(dst, _) = inst {
                    let var = self.ensure_var(*dst);
                    let param = self.builder.append_block_param(clif_block, self.ptr_type);
                    self.builder.def_var(var, param);
                }
            }

            // Translate regular instructions.
            for inst in &ir_block.insts {
                self.translate_inst(inst)?;
            }

            // Translate terminator.
            self.translate_terminator(&ir_block.terminator, ir_block.id, ir_func)?;
        }

        self.builder.seal_all_blocks();
        Ok(())
    }

    fn translate_inst(&mut self, inst: &Inst) -> CodegenResult<()> {
        match inst {
            Inst::Const(dst, c) => {
                let val = self.emit_const(c)?;
                let var = self.ensure_var(*dst);
                self.builder.def_var(var, val);
            }

            Inst::LoadLocal(dst, _name) => {
                // In AOT, locals are already bound via parameters or let bindings.
                // LoadLocal should have been resolved by the ANF lowering.
                // For now, define as nil.
                let val = self.call_rt_0(self.rt.rt_const_nil)?;
                let var = self.ensure_var(*dst);
                self.builder.def_var(var, val);
            }

            Inst::LoadGlobal(dst, ns, name) => {
                let val = self.emit_load_global(ns, name)?;
                let var = self.ensure_var(*dst);
                self.builder.def_var(var, val);
            }

            Inst::AllocVector(dst, elems) => {
                let val = self.emit_alloc_collection(self.rt.rt_alloc_vector, elems)?;
                let var = self.ensure_var(*dst);
                self.builder.def_var(var, val);
            }

            Inst::AllocMap(dst, pairs) => {
                let flat: Vec<VarId> = pairs.iter().flat_map(|(k, v)| [*k, *v]).collect();
                let val = self.emit_alloc_collection(self.rt.rt_alloc_map, &flat)?;
                let var = self.ensure_var(*dst);
                self.builder.def_var(var, val);
            }

            Inst::AllocSet(dst, elems) => {
                let val = self.emit_alloc_collection(self.rt.rt_alloc_set, elems)?;
                let var = self.ensure_var(*dst);
                self.builder.def_var(var, val);
            }

            Inst::AllocList(dst, elems) => {
                let val = self.emit_alloc_collection(self.rt.rt_alloc_list, elems)?;
                let var = self.ensure_var(*dst);
                self.builder.def_var(var, val);
            }

            Inst::AllocCons(dst, head, tail) => {
                let h = self.use_var(*head);
                let t = self.use_var(*tail);
                let val = self.call_rt_2(self.rt.rt_alloc_cons, h, t)?;
                let var = self.ensure_var(*dst);
                self.builder.def_var(var, val);
            }

            Inst::CallKnown(dst, known_fn, args) => {
                let val = self.emit_known_call(known_fn, args)?;
                let var = self.ensure_var(*dst);
                self.builder.def_var(var, val);
            }

            Inst::Call(dst, callee, args) => {
                let val = self.emit_unknown_call(*callee, args)?;
                let var = self.ensure_var(*dst);
                self.builder.def_var(var, val);
            }

            Inst::Deref(dst, src) => {
                let s = self.use_var(*src);
                let val = self.call_rt_1(self.rt.rt_deref, s)?;
                let var = self.ensure_var(*dst);
                self.builder.def_var(var, val);
            }

            Inst::DefVar(dst, ns, name, val_var) => {
                let val = self.emit_def_var(ns, name, *val_var)?;
                let var = self.ensure_var(*dst);
                self.builder.def_var(var, val);
            }

            Inst::SetBang(_var, _val) => {
                // TODO: implement set!
            }

            Inst::Throw(val) => {
                let v = self.use_var(*val);
                let func_ref = self.import_func(self.rt.rt_throw);
                self.builder.ins().call(func_ref, &[v]);
                // rt_throw stores the exception in a thread-local and returns.
                // The block ends with Unreachable which returns nil, allowing
                // the caller (rt_try) to check the thread-local.
            }

            Inst::Phi(_, _) => {
                // Handled above in block preamble.
            }

            Inst::Recur(_) => {
                // Handled by RecurJump terminator.
            }

            Inst::SourceLoc(_) => {
                // No-op in codegen (could add debug info later).
            }

            Inst::AllocClosure(dst, template, captures) => {
                // Get the first arity's compiled function name and param count.
                // For now we support single-arity closures; multi-arity TBD.
                if template.arity_fn_names.is_empty() {
                    // No compiled arities — fall back to nil.
                    let val = self.call_rt_0(self.rt.rt_const_nil)?;
                    let var = self.ensure_var(*dst);
                    self.builder.def_var(var, val);
                } else {
                    let arity_fn_name = &template.arity_fn_names[0];
                    let param_count = template.param_counts[0];

                    // Get the FuncId for the compiled arity function.
                    let arity_func_id = self.user_funcs[arity_fn_name];

                    // Get a function pointer to the compiled function.
                    let func_ref = self
                        .module
                        .declare_func_in_func(arity_func_id, self.builder.func);
                    let fn_ptr = self.builder.ins().func_addr(self.ptr_type, func_ref);

                    // Emit the function name as a data constant.
                    let name_str = template.name.as_deref().unwrap_or(arity_fn_name);
                    let name_data = self.module.declare_anonymous_data(false, false)?;
                    let mut name_desc = cranelift_module::DataDescription::new();
                    name_desc.define(name_str.as_bytes().to_vec().into_boxed_slice());
                    self.module.define_data(name_data, &name_desc)?;
                    let name_gv = self
                        .module
                        .declare_data_in_func(name_data, self.builder.func);
                    let name_ptr = self.builder.ins().global_value(self.ptr_type, name_gv);
                    let name_len = self.builder.ins().iconst(types::I64, name_str.len() as i64);

                    let param_count_val = self.builder.ins().iconst(types::I64, param_count as i64);

                    // Spill captures to stack.
                    let ncaptures = captures.len();
                    let (captures_ptr, ncaptures_val) = if ncaptures == 0 {
                        let null = self.builder.ins().iconst(self.ptr_type, 0);
                        let zero = self.builder.ins().iconst(types::I64, 0);
                        (null, zero)
                    } else {
                        let slot = self.builder.create_sized_stack_slot(
                            cranelift_codegen::ir::StackSlotData::new(
                                cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                                (ncaptures * 8) as u32,
                                3,
                            ),
                        );
                        for (i, cap_var) in captures.iter().enumerate() {
                            let cap_val = self.use_var(*cap_var);
                            self.builder
                                .ins()
                                .stack_store(cap_val, slot, (i * 8) as i32);
                        }
                        let slot_addr = self.builder.ins().stack_addr(self.ptr_type, slot, 0);
                        let n = self.builder.ins().iconst(types::I64, ncaptures as i64);
                        (slot_addr, n)
                    };

                    // Call rt_make_fn(name_ptr, name_len, fn_ptr, param_count, captures, ncaptures)
                    let func_ref = self.import_func(self.rt.rt_make_fn);
                    let call = self.builder.ins().call(
                        func_ref,
                        &[
                            name_ptr,
                            name_len,
                            fn_ptr,
                            param_count_val,
                            captures_ptr,
                            ncaptures_val,
                        ],
                    );
                    let result = self.builder.inst_results(call)[0];
                    let var = self.ensure_var(*dst);
                    self.builder.def_var(var, result);
                }
            }

            Inst::RegionStart(dst) | Inst::RegionAlloc(dst, _, _, _) => {
                // TODO: implement region operations
                let val = self.call_rt_0(self.rt.rt_const_nil)?;
                let var = self.ensure_var(*dst);
                self.builder.def_var(var, val);
            }

            Inst::RegionEnd(_) => {
                // TODO: implement region end
            }
        }
        Ok(())
    }

    fn translate_terminator(
        &mut self,
        term: &Terminator,
        current_block_id: BlockId,
        ir_func: &IrFunction,
    ) -> CodegenResult<()> {
        match term {
            Terminator::Return(var_id) => {
                let val = self.use_var(*var_id);
                self.builder.ins().return_(&[val]);
            }

            Terminator::Jump(target) => {
                let clif_block = self.block_map[target];
                let phi_args = self.collect_phi_args(*target, current_block_id, ir_func);
                self.builder.ins().jump(clif_block, &phi_args);
            }

            Terminator::Branch {
                cond,
                then_block,
                else_block,
            } => {
                let cond_val = self.use_var(*cond);
                let truthy = self.call_rt_1_i8(self.rt.rt_truthiness, cond_val)?;
                let then_b = self.block_map[then_block];
                let else_b = self.block_map[else_block];
                let then_args = self.collect_phi_args(*then_block, current_block_id, ir_func);
                let else_args = self.collect_phi_args(*else_block, current_block_id, ir_func);
                self.builder
                    .ins()
                    .brif(truthy, then_b, &then_args, else_b, &else_args);
            }

            Terminator::RecurJump { target, args } => {
                let clif_block = self.block_map[target];
                let arg_vals: Vec<BlockArg> = args
                    .iter()
                    .map(|a| BlockArg::Value(self.use_var(*a)))
                    .collect();
                self.builder.ins().jump(clif_block, &arg_vals);
            }

            Terminator::Unreachable => {
                // Return nil as a safe fallback. In practice, throw paths
                // return before reaching here (see Inst::Throw above).
                let nil_ref = self.import_func(self.rt.rt_const_nil);
                let nil_call = self.builder.ins().call(nil_ref, &[]);
                let nil_val = self.builder.inst_results(nil_call)[0];
                self.builder.ins().return_(&[nil_val]);
            }
        }
        Ok(())
    }

    /// Collect phi arguments needed when jumping from `from_block` to `to_block`.
    fn collect_phi_args(
        &mut self,
        to_block: BlockId,
        from_block: BlockId,
        ir_func: &IrFunction,
    ) -> Vec<BlockArg> {
        let target = ir_func.blocks.iter().find(|b| b.id == to_block);
        let Some(target) = target else {
            return vec![];
        };
        target
            .phis
            .iter()
            .filter_map(|inst| {
                if let Inst::Phi(_, entries) = inst {
                    // Find the entry for the predecessor block.
                    entries
                        .iter()
                        .find(|(pred, _)| *pred == from_block)
                        .map(|(_, var_id)| BlockArg::Value(self.use_var(*var_id)))
                } else {
                    None
                }
            })
            .collect()
    }

    // ── Helpers ─────────────────────────────────────────────────────────────

    fn ensure_var(&mut self, var_id: VarId) -> Variable {
        if let Some(&var) = self.var_map.get(&var_id) {
            var
        } else {
            let var = self.builder.declare_var(self.ptr_type);
            self.var_map.insert(var_id, var);
            var
        }
    }

    fn use_var(&mut self, var_id: VarId) -> cranelift_codegen::ir::Value {
        let var = self.ensure_var(var_id);
        self.builder.use_var(var)
    }

    /// Emit a constant value.
    fn emit_const(&mut self, c: &Const) -> CodegenResult<cranelift_codegen::ir::Value> {
        match c {
            Const::Nil => self.call_rt_0(self.rt.rt_const_nil),
            Const::Bool(true) => self.call_rt_0(self.rt.rt_const_true),
            Const::Bool(false) => self.call_rt_0(self.rt.rt_const_false),
            Const::Long(n) => {
                let func_ref = self.import_func(self.rt.rt_const_long);
                let arg = self.builder.ins().iconst(types::I64, *n);
                let call = self.builder.ins().call(func_ref, &[arg]);
                Ok(self.builder.inst_results(call)[0])
            }
            Const::Double(f) => {
                let func_ref = self.import_func(self.rt.rt_const_double);
                let arg = self.builder.ins().f64const(*f);
                let call = self.builder.ins().call(func_ref, &[arg]);
                Ok(self.builder.inst_results(call)[0])
            }
            Const::Char(ch) => {
                let func_ref = self.import_func(self.rt.rt_const_char);
                let arg = self.builder.ins().iconst(types::I32, *ch as i64);
                let call = self.builder.ins().call(func_ref, &[arg]);
                Ok(self.builder.inst_results(call)[0])
            }
            Const::Str(s) => self.emit_string_const(self.rt.rt_const_string, s),
            Const::Keyword(s) => self.emit_string_const(self.rt.rt_const_keyword, s),
            Const::Symbol(s) => self.emit_string_const(self.rt.rt_const_symbol, s),
        }
    }

    /// Emit a call to a runtime function that takes (ptr, len) for a string.
    fn emit_string_const(
        &mut self,
        func_id: FuncId,
        s: &str,
    ) -> CodegenResult<cranelift_codegen::ir::Value> {
        // Store the string bytes as a data object in the module.
        let data_id = self
            .module
            .declare_anonymous_data(false, false)
            .map_err(CodegenError::Module)?;

        let mut data_desc = cranelift_module::DataDescription::new();
        data_desc.define(s.as_bytes().to_vec().into_boxed_slice());
        self.module
            .define_data(data_id, &data_desc)
            .map_err(CodegenError::Module)?;

        let global_val = self.module.declare_data_in_func(data_id, self.builder.func);
        let ptr = self.builder.ins().global_value(self.ptr_type, global_val);
        let len = self.builder.ins().iconst(types::I64, s.len() as i64);

        let func_ref = self.import_func(func_id);
        let call = self.builder.ins().call(func_ref, &[ptr, len]);
        Ok(self.builder.inst_results(call)[0])
    }

    /// Emit a LoadGlobal (ns/name lookup).
    fn emit_load_global(
        &mut self,
        ns: &str,
        name: &str,
    ) -> CodegenResult<cranelift_codegen::ir::Value> {
        // Create data objects for ns and name strings.
        let ns_data = self.module.declare_anonymous_data(false, false)?;
        let mut ns_desc = cranelift_module::DataDescription::new();
        ns_desc.define(ns.as_bytes().to_vec().into_boxed_slice());
        self.module.define_data(ns_data, &ns_desc)?;

        let name_data = self.module.declare_anonymous_data(false, false)?;
        let mut name_desc = cranelift_module::DataDescription::new();
        name_desc.define(name.as_bytes().to_vec().into_boxed_slice());
        self.module.define_data(name_data, &name_desc)?;

        let ns_gv = self.module.declare_data_in_func(ns_data, self.builder.func);
        let ns_ptr = self.builder.ins().global_value(self.ptr_type, ns_gv);
        let ns_len = self.builder.ins().iconst(types::I64, ns.len() as i64);

        let name_gv = self
            .module
            .declare_data_in_func(name_data, self.builder.func);
        let name_ptr = self.builder.ins().global_value(self.ptr_type, name_gv);
        let name_len = self.builder.ins().iconst(types::I64, name.len() as i64);

        let func_ref = self.import_func(self.rt.rt_load_global);
        let call = self
            .builder
            .ins()
            .call(func_ref, &[ns_ptr, ns_len, name_ptr, name_len]);
        Ok(self.builder.inst_results(call)[0])
    }

    /// Emit a `(def ns/name val)` — interns the var in the global env.
    fn emit_def_var(
        &mut self,
        ns: &str,
        name: &str,
        val_var: VarId,
    ) -> CodegenResult<cranelift_codegen::ir::Value> {
        let ns_data = self.module.declare_anonymous_data(false, false)?;
        let mut ns_desc = cranelift_module::DataDescription::new();
        ns_desc.define(ns.as_bytes().to_vec().into_boxed_slice());
        self.module.define_data(ns_data, &ns_desc)?;

        let name_data = self.module.declare_anonymous_data(false, false)?;
        let mut name_desc = cranelift_module::DataDescription::new();
        name_desc.define(name.as_bytes().to_vec().into_boxed_slice());
        self.module.define_data(name_data, &name_desc)?;

        let ns_gv = self.module.declare_data_in_func(ns_data, self.builder.func);
        let ns_ptr = self.builder.ins().global_value(self.ptr_type, ns_gv);
        let ns_len = self.builder.ins().iconst(types::I64, ns.len() as i64);

        let name_gv = self
            .module
            .declare_data_in_func(name_data, self.builder.func);
        let name_ptr = self.builder.ins().global_value(self.ptr_type, name_gv);
        let name_len = self.builder.ins().iconst(types::I64, name.len() as i64);

        let val = self.use_var(val_var);

        let func_ref = self.import_func(self.rt.rt_def_var);
        let call = self
            .builder
            .ins()
            .call(func_ref, &[ns_ptr, ns_len, name_ptr, name_len, val]);
        Ok(self.builder.inst_results(call)[0])
    }

    /// Emit allocation of a collection.  Spills element pointers to the stack,
    /// then calls the runtime allocator.
    fn emit_alloc_collection(
        &mut self,
        func_id: FuncId,
        elems: &[VarId],
    ) -> CodegenResult<cranelift_codegen::ir::Value> {
        let n = elems.len();
        if n == 0 {
            let func_ref = self.import_func(func_id);
            // Pass null pointer and 0.
            let null = self.builder.ins().iconst(self.ptr_type, 0);
            let zero = self.builder.ins().iconst(types::I64, 0);
            let call = self.builder.ins().call(func_ref, &[null, zero]);
            return Ok(self.builder.inst_results(call)[0]);
        }

        // Allocate stack space for element pointers.
        let slot = self
            .builder
            .create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                (n * 8) as u32,
                3, // align to 8 bytes
            ));

        // Store each element pointer.
        for (i, elem_var) in elems.iter().enumerate() {
            let val = self.use_var(*elem_var);
            self.builder.ins().stack_store(val, slot, (i * 8) as i32);
        }

        // Get the stack slot address.
        let slot_addr = self.builder.ins().stack_addr(self.ptr_type, slot, 0);
        let count = self.builder.ins().iconst(types::I64, n as i64);

        let func_ref = self.import_func(func_id);
        // For maps, count is number of pairs (n/2).
        let actual_count = if func_id == self.rt.rt_alloc_map {
            self.builder.ins().iconst(types::I64, (n / 2) as i64)
        } else {
            count
        };
        let call = self
            .builder
            .ins()
            .call(func_ref, &[slot_addr, actual_count]);
        Ok(self.builder.inst_results(call)[0])
    }

    /// Emit a call to a known function.
    fn emit_known_call(
        &mut self,
        known_fn: &KnownFn,
        args: &[VarId],
    ) -> CodegenResult<cranelift_codegen::ir::Value> {
        // Collection constructors use variadic stack-spill pattern.
        match known_fn {
            KnownFn::Vector => return self.emit_alloc_collection(self.rt.rt_alloc_vector, args),
            KnownFn::HashMap => return self.emit_alloc_collection(self.rt.rt_alloc_map, args),
            KnownFn::HashSet => return self.emit_alloc_collection(self.rt.rt_alloc_set, args),
            KnownFn::List => return self.emit_alloc_collection(self.rt.rt_alloc_list, args),
            KnownFn::AtomSwap => return self.emit_atom_swap(args),
            _ => {}
        }

        let rt_func = match known_fn {
            KnownFn::Add => self.rt.rt_add,
            KnownFn::Sub => self.rt.rt_sub,
            KnownFn::Mul => self.rt.rt_mul,
            KnownFn::Div => self.rt.rt_div,
            KnownFn::Rem => self.rt.rt_rem,
            KnownFn::Eq => self.rt.rt_eq,
            KnownFn::Lt => self.rt.rt_lt,
            KnownFn::Gt => self.rt.rt_gt,
            KnownFn::Lte => self.rt.rt_lte,
            KnownFn::Gte => self.rt.rt_gte,
            KnownFn::Get => self.rt.rt_get,
            KnownFn::Count => self.rt.rt_count,
            KnownFn::First => self.rt.rt_first,
            KnownFn::Rest | KnownFn::Next => self.rt.rt_rest,
            KnownFn::Assoc => self.rt.rt_assoc,
            KnownFn::Conj => self.rt.rt_conj,
            KnownFn::Deref | KnownFn::AtomDeref => self.rt.rt_deref,
            KnownFn::Println => self.rt.rt_println,
            KnownFn::Pr => self.rt.rt_pr,
            KnownFn::IsNil => self.rt.rt_is_nil,
            KnownFn::IsVector => self.rt.rt_is_vector,
            KnownFn::IsMap => self.rt.rt_is_map,
            KnownFn::IsSeq => self.rt.rt_is_seq,
            KnownFn::Identical => self.rt.rt_identical,
            KnownFn::Str => self.rt.rt_str,
            KnownFn::TryCatchFinally => self.rt.rt_try,
            KnownFn::Dissoc => self.rt.rt_dissoc,
            KnownFn::Disj => self.rt.rt_disj,
            KnownFn::Nth => self.rt.rt_nth,
            KnownFn::Contains => self.rt.rt_contains,
            KnownFn::Cons => self.rt.rt_alloc_cons,
            KnownFn::Seq => self.rt.rt_seq,
            KnownFn::LazySeq => self.rt.rt_lazy_seq,
            KnownFn::Transient => self.rt.rt_transient,
            KnownFn::AssocBang => self.rt.rt_assoc_bang,
            KnownFn::ConjBang => self.rt.rt_conj_bang,
            KnownFn::PersistentBang => self.rt.rt_persistent_bang,
            KnownFn::AtomReset => self.rt.rt_atom_reset,
            KnownFn::Apply => self.rt.rt_apply,
            _ => {
                return self.emit_unknown_call_from_args(args);
            }
        };

        // Call the specific runtime function with the right arity.
        let arg_vals: Vec<_> = args.iter().map(|a| self.use_var(*a)).collect();
        let func_ref = self.import_func(rt_func);
        let call = self.builder.ins().call(func_ref, &arg_vals);
        Ok(self.builder.inst_results(call)[0])
    }

    /// Emit `(swap! atom f extra-args...)` — variadic via stack-spill.
    fn emit_atom_swap(
        &mut self,
        args: &[VarId],
    ) -> CodegenResult<cranelift_codegen::ir::Value> {
        // args[0] = atom, args[1] = f, args[2..] = extra args
        let atom_val = self.use_var(args[0]);
        let f_val = self.use_var(args[1]);
        let extra = &args[2..];
        let n = extra.len();

        let (extra_ptr, extra_count) = if n > 0 {
            let slot = self
                .builder
                .create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                    cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                    (n * 8) as u32,
                    3,
                ));
            for (i, arg) in extra.iter().enumerate() {
                let val = self.use_var(*arg);
                self.builder.ins().stack_store(val, slot, (i * 8) as i32);
            }
            let addr = self.builder.ins().stack_addr(self.ptr_type, slot, 0);
            let count = self.builder.ins().iconst(types::I64, n as i64);
            (addr, count)
        } else {
            let null = self.builder.ins().iconst(self.ptr_type, 0);
            let zero = self.builder.ins().iconst(types::I64, 0);
            (null, zero)
        };

        let func_ref = self.import_func(self.rt.rt_atom_swap);
        let call = self
            .builder
            .ins()
            .call(func_ref, &[atom_val, f_val, extra_ptr, extra_count]);
        Ok(self.builder.inst_results(call)[0])
    }

    /// Emit an unknown function call through rt_call.
    fn emit_unknown_call(
        &mut self,
        callee: VarId,
        args: &[VarId],
    ) -> CodegenResult<cranelift_codegen::ir::Value> {
        let callee_val = self.use_var(callee);
        let n = args.len();

        if n == 0 {
            let null = self.builder.ins().iconst(self.ptr_type, 0);
            let zero = self.builder.ins().iconst(types::I64, 0);
            let func_ref = self.import_func(self.rt.rt_call);
            let call = self.builder.ins().call(func_ref, &[callee_val, null, zero]);
            return Ok(self.builder.inst_results(call)[0]);
        }

        // Spill args to stack.
        let slot = self
            .builder
            .create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                (n * 8) as u32,
                3,
            ));
        for (i, arg) in args.iter().enumerate() {
            let val = self.use_var(*arg);
            self.builder.ins().stack_store(val, slot, (i * 8) as i32);
        }
        let slot_addr = self.builder.ins().stack_addr(self.ptr_type, slot, 0);
        let count = self.builder.ins().iconst(types::I64, n as i64);

        let func_ref = self.import_func(self.rt.rt_call);
        let call = self
            .builder
            .ins()
            .call(func_ref, &[callee_val, slot_addr, count]);
        Ok(self.builder.inst_results(call)[0])
    }

    /// Fallback for known functions without a specific rt_ bridge.
    fn emit_unknown_call_from_args(
        &mut self,
        _args: &[VarId],
    ) -> CodegenResult<cranelift_codegen::ir::Value> {
        // For now, return nil.  A real implementation would look up the
        // function by name and call through rt_call.
        self.call_rt_0(self.rt.rt_const_nil)
    }

    // ── Call helpers ────────────────────────────────────────────────────────

    fn import_func(&mut self, func_id: FuncId) -> cranelift_codegen::ir::FuncRef {
        self.module.declare_func_in_func(func_id, self.builder.func)
    }

    fn call_rt_0(&mut self, func_id: FuncId) -> CodegenResult<cranelift_codegen::ir::Value> {
        let func_ref = self.import_func(func_id);
        let call = self.builder.ins().call(func_ref, &[]);
        Ok(self.builder.inst_results(call)[0])
    }

    fn call_rt_1(
        &mut self,
        func_id: FuncId,
        arg: cranelift_codegen::ir::Value,
    ) -> CodegenResult<cranelift_codegen::ir::Value> {
        let func_ref = self.import_func(func_id);
        let call = self.builder.ins().call(func_ref, &[arg]);
        Ok(self.builder.inst_results(call)[0])
    }

    /// Call a runtime function that returns u8 (for truthiness).
    fn call_rt_1_i8(
        &mut self,
        func_id: FuncId,
        arg: cranelift_codegen::ir::Value,
    ) -> CodegenResult<cranelift_codegen::ir::Value> {
        let func_ref = self.import_func(func_id);
        let call = self.builder.ins().call(func_ref, &[arg]);
        Ok(self.builder.inst_results(call)[0])
    }

    fn call_rt_2(
        &mut self,
        func_id: FuncId,
        a: cranelift_codegen::ir::Value,
        b: cranelift_codegen::ir::Value,
    ) -> CodegenResult<cranelift_codegen::ir::Value> {
        let func_ref = self.import_func(func_id);
        let call = self.builder.ins().call(func_ref, &[a, b]);
        Ok(self.builder.inst_results(call)[0])
    }
}

// ── Runtime function declaration ────────────────────────────────────────────

/// Helper to declare a single runtime function.
fn declare_rt(
    module: &mut ObjectModule,
    name: &str,
    params: &[types::Type],
    ret: types::Type,
) -> CodegenResult<FuncId> {
    let mut sig = module.make_signature();
    sig.call_conv = CallConv::SystemV;
    for &t in params {
        sig.params.push(AbiParam::new(t));
    }
    sig.returns.push(AbiParam::new(ret));
    Ok(module.declare_function(name, Linkage::Import, &sig)?)
}

/// Declare all runtime bridge functions and return cached FuncIds.
fn declare_runtime_funcs(
    module: &mut ObjectModule,
    ptr: types::Type,
) -> CodegenResult<RuntimeFuncs> {
    Ok(RuntimeFuncs {
        rt_const_nil: declare_rt(module, "rt_const_nil", &[], ptr)?,
        rt_const_true: declare_rt(module, "rt_const_true", &[], ptr)?,
        rt_const_false: declare_rt(module, "rt_const_false", &[], ptr)?,
        rt_const_long: declare_rt(module, "rt_const_long", &[types::I64], ptr)?,
        rt_const_double: declare_rt(module, "rt_const_double", &[types::F64], ptr)?,
        rt_const_char: declare_rt(module, "rt_const_char", &[types::I32], ptr)?,
        rt_const_string: declare_rt(module, "rt_const_string", &[ptr, types::I64], ptr)?,
        rt_const_keyword: declare_rt(module, "rt_const_keyword", &[ptr, types::I64], ptr)?,
        rt_const_symbol: declare_rt(module, "rt_const_symbol", &[ptr, types::I64], ptr)?,
        rt_truthiness: declare_rt(module, "rt_truthiness", &[ptr], types::I8)?,
        rt_add: declare_rt(module, "rt_add", &[ptr, ptr], ptr)?,
        rt_sub: declare_rt(module, "rt_sub", &[ptr, ptr], ptr)?,
        rt_mul: declare_rt(module, "rt_mul", &[ptr, ptr], ptr)?,
        rt_div: declare_rt(module, "rt_div", &[ptr, ptr], ptr)?,
        rt_rem: declare_rt(module, "rt_rem", &[ptr, ptr], ptr)?,
        rt_eq: declare_rt(module, "rt_eq", &[ptr, ptr], ptr)?,
        rt_lt: declare_rt(module, "rt_lt", &[ptr, ptr], ptr)?,
        rt_gt: declare_rt(module, "rt_gt", &[ptr, ptr], ptr)?,
        rt_lte: declare_rt(module, "rt_lte", &[ptr, ptr], ptr)?,
        rt_gte: declare_rt(module, "rt_gte", &[ptr, ptr], ptr)?,
        rt_alloc_vector: declare_rt(module, "rt_alloc_vector", &[ptr, types::I64], ptr)?,
        rt_alloc_map: declare_rt(module, "rt_alloc_map", &[ptr, types::I64], ptr)?,
        rt_alloc_set: declare_rt(module, "rt_alloc_set", &[ptr, types::I64], ptr)?,
        rt_alloc_list: declare_rt(module, "rt_alloc_list", &[ptr, types::I64], ptr)?,
        rt_alloc_cons: declare_rt(module, "rt_alloc_cons", &[ptr, ptr], ptr)?,
        rt_get: declare_rt(module, "rt_get", &[ptr, ptr], ptr)?,
        rt_count: declare_rt(module, "rt_count", &[ptr], ptr)?,
        rt_first: declare_rt(module, "rt_first", &[ptr], ptr)?,
        rt_rest: declare_rt(module, "rt_rest", &[ptr], ptr)?,
        rt_assoc: declare_rt(module, "rt_assoc", &[ptr, ptr, ptr], ptr)?,
        rt_conj: declare_rt(module, "rt_conj", &[ptr, ptr], ptr)?,
        rt_call: declare_rt(module, "rt_call", &[ptr, ptr, types::I64], ptr)?,
        rt_deref: declare_rt(module, "rt_deref", &[ptr], ptr)?,
        rt_println: declare_rt(module, "rt_println", &[ptr], ptr)?,
        rt_pr: declare_rt(module, "rt_pr", &[ptr], ptr)?,
        rt_is_nil: declare_rt(module, "rt_is_nil", &[ptr], ptr)?,
        rt_is_vector: declare_rt(module, "rt_is_vector", &[ptr], ptr)?,
        rt_is_map: declare_rt(module, "rt_is_map", &[ptr], ptr)?,
        rt_is_seq: declare_rt(module, "rt_is_seq", &[ptr], ptr)?,
        rt_identical: declare_rt(module, "rt_identical", &[ptr, ptr], ptr)?,
        rt_str: declare_rt(module, "rt_str", &[ptr], ptr)?,
        rt_load_global: declare_rt(
            module,
            "rt_load_global",
            &[ptr, types::I64, ptr, types::I64],
            ptr,
        )?,
        rt_def_var: declare_rt(
            module,
            "rt_def_var",
            &[ptr, types::I64, ptr, types::I64, ptr],
            ptr,
        )?,
        rt_make_fn: declare_rt(
            module,
            "rt_make_fn",
            &[ptr, types::I64, ptr, types::I64, ptr, types::I64],
            ptr,
        )?,
        rt_throw: declare_rt(module, "rt_throw", &[ptr], ptr)?,
        rt_try: declare_rt(module, "rt_try", &[ptr, ptr, ptr], ptr)?,
        rt_dissoc: declare_rt(module, "rt_dissoc", &[ptr, ptr], ptr)?,
        rt_disj: declare_rt(module, "rt_disj", &[ptr, ptr], ptr)?,
        rt_nth: declare_rt(module, "rt_nth", &[ptr, ptr], ptr)?,
        rt_contains: declare_rt(module, "rt_contains", &[ptr, ptr], ptr)?,
        rt_seq: declare_rt(module, "rt_seq", &[ptr], ptr)?,
        rt_lazy_seq: declare_rt(module, "rt_lazy_seq", &[ptr], ptr)?,
        rt_transient: declare_rt(module, "rt_transient", &[ptr], ptr)?,
        rt_assoc_bang: declare_rt(module, "rt_assoc_bang", &[ptr, ptr, ptr], ptr)?,
        rt_conj_bang: declare_rt(module, "rt_conj_bang", &[ptr, ptr], ptr)?,
        rt_persistent_bang: declare_rt(module, "rt_persistent_bang", &[ptr], ptr)?,
        rt_atom_reset: declare_rt(module, "rt_atom_reset", &[ptr, ptr], ptr)?,
        rt_atom_swap: declare_rt(
            module,
            "rt_atom_swap",
            &[ptr, ptr, ptr, types::I64],
            ptr,
        )?,
        rt_apply: declare_rt(module, "rt_apply", &[ptr, ptr], ptr)?,
    })
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cljrs_reader::Parser;

    fn parse_body(src: &str) -> Vec<cljrs_reader::Form> {
        let mut parser = Parser::new(src.to_string(), "<test>".to_string());
        let mut forms = Vec::new();
        while let Ok(Some(form)) = parser.parse_one() {
            forms.push(form);
        }
        forms
    }

    fn lower(
        name: &str,
        params: &[Arc<str>],
        body: &[cljrs_reader::Form],
    ) -> crate::ir::IrFunction {
        // Run on a thread with a larger stack since Clojure eval is deeply recursive.
        let name = name.to_string();
        let params = params.to_vec();
        let body = body.to_vec();
        std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let globals = cljrs_stdlib::standard_env();
                let mut env = cljrs_eval::Env::new(globals, "user");
                crate::aot::lower_via_clojure(Some(&name), "user", &params, &body, &mut env)
                    .unwrap()
            })
            .unwrap()
            .join()
            .unwrap()
    }

    #[test]
    fn test_compile_constant_function() {
        // (defn f [] 42)
        let body = parse_body("42");
        let ir = lower("f", &[], &body);

        let mut compiler = Compiler::new().unwrap();
        let func_id = compiler.declare_function("f", 0).unwrap();
        compiler.compile_function(&ir, func_id).unwrap();
        let obj = compiler.finish();
        assert!(!obj.is_empty(), "should produce non-empty object code");
    }

    #[test]
    fn test_compile_add_function() {
        // (defn add [a b] (+ a b))
        let body = parse_body("(+ a b)");
        let params: Vec<Arc<str>> = vec![Arc::from("a"), Arc::from("b")];
        let ir = lower("add", &params, &body);

        let mut compiler = Compiler::new().unwrap();
        let func_id = compiler.declare_function("add", 2).unwrap();
        compiler.compile_function(&ir, func_id).unwrap();
        let obj = compiler.finish();
        assert!(!obj.is_empty());
    }

    #[test]
    fn test_compile_if_expression() {
        // (defn f [x] (if x 1 2))
        let body = parse_body("(if x 1 2)");
        let params: Vec<Arc<str>> = vec![Arc::from("x")];
        let ir = lower("f", &params, &body);

        let mut compiler = Compiler::new().unwrap();
        let func_id = compiler.declare_function("f", 1).unwrap();
        compiler.compile_function(&ir, func_id).unwrap();
        let obj = compiler.finish();
        assert!(!obj.is_empty());
    }

    #[test]
    fn test_compile_let_expression() {
        // (defn f [x] (let [y (+ x 1)] y))
        let body = parse_body("(let [y (+ x 1)] y)");
        let params: Vec<Arc<str>> = vec![Arc::from("x")];
        let ir = lower("f", &params, &body);

        let mut compiler = Compiler::new().unwrap();
        let func_id = compiler.declare_function("f", 1).unwrap();
        compiler.compile_function(&ir, func_id).unwrap();
        let obj = compiler.finish();
        assert!(!obj.is_empty());
    }

    #[test]
    fn test_compile_loop_recur() {
        // (defn sum [n] (loop [i 0 acc 0] (if (= i n) acc (recur (+ i 1) (+ acc i)))))
        let body = parse_body("(loop [i 0 acc 0] (if (= i n) acc (recur (+ i 1) (+ acc i))))");
        let params: Vec<Arc<str>> = vec![Arc::from("n")];
        let ir = lower("sum", &params, &body);

        let mut compiler = Compiler::new().unwrap();
        let func_id = compiler.declare_function("sum", 1).unwrap();
        compiler.compile_function(&ir, func_id).unwrap();
        let obj = compiler.finish();
        assert!(!obj.is_empty());
    }
}
