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

use crate::ir::{BlockId, Const, Inst, IrFunction, KnownFn, RegionAllocKind, Terminator, VarId};

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
    rt_safepoint: FuncId,
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
    rt_make_fn_variadic: FuncId,
    rt_make_fn_multi: FuncId,
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
    rt_set_bang: FuncId,
    rt_with_bindings: FuncId,
    rt_load_var: FuncId,
    rt_reduce2: FuncId,
    rt_reduce3: FuncId,
    rt_map: FuncId,
    rt_filter: FuncId,
    rt_mapv: FuncId,
    rt_filterv: FuncId,
    rt_some: FuncId,
    rt_every: FuncId,
    rt_into: FuncId,
    rt_into3: FuncId,
    rt_group_by: FuncId,
    rt_partition2: FuncId,
    rt_partition3: FuncId,
    rt_partition4: FuncId,
    rt_frequencies: FuncId,
    rt_keep: FuncId,
    rt_remove: FuncId,
    rt_map_indexed: FuncId,
    rt_zipmap: FuncId,
    rt_juxt: FuncId,
    rt_comp: FuncId,
    rt_partial: FuncId,
    rt_complement: FuncId,
    rt_concat: FuncId,
    rt_range1: FuncId,
    rt_range2: FuncId,
    rt_range3: FuncId,
    rt_take: FuncId,
    rt_drop: FuncId,
    rt_reverse: FuncId,
    rt_sort: FuncId,
    rt_sort_by: FuncId,
    rt_keys: FuncId,
    rt_vals: FuncId,
    rt_merge: FuncId,
    rt_update: FuncId,
    rt_get_in: FuncId,
    rt_assoc_in: FuncId,
    rt_is_number: FuncId,
    rt_is_string: FuncId,
    rt_is_keyword: FuncId,
    rt_is_symbol: FuncId,
    rt_is_bool: FuncId,
    rt_is_int: FuncId,
    rt_prn: FuncId,
    rt_print: FuncId,
    rt_atom: FuncId,
    rt_str_n: FuncId,
    rt_println_n: FuncId,
    rt_with_out_str: FuncId,
    // Region allocation
    rt_region_start: FuncId,
    rt_region_end: FuncId,
    rt_region_alloc_vector: FuncId,
    rt_region_alloc_map: FuncId,
    rt_region_alloc_set: FuncId,
    rt_region_alloc_list: FuncId,
    rt_region_alloc_cons: FuncId,
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

        // GC safepoint at function entry.
        self.emit_safepoint();

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

            Inst::LoadVar(dst, ns, name) => {
                let val = self.emit_load_var(ns, name)?;
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

            Inst::CallDirect(dst, fn_name, args) => {
                let val = self.emit_direct_call(fn_name, args)?;
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

            Inst::SetBang(var, val) => {
                let var_v = self.use_var(*var);
                let val_v = self.use_var(*val);
                let func_ref = self.import_func(self.rt.rt_set_bang);
                self.builder.ins().call(func_ref, &[var_v, val_v]);
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
                if template.arity_fn_names.is_empty() {
                    // No compiled arities — fall back to nil.
                    let val = self.call_rt_0(self.rt.rt_const_nil)?;
                    let var = self.ensure_var(*dst);
                    self.builder.def_var(var, val);
                } else {
                    // Emit the function name as a data constant.
                    let name_str = template
                        .name
                        .as_deref()
                        .unwrap_or(&template.arity_fn_names[0]);
                    let name_data = self.module.declare_anonymous_data(false, false)?;
                    let mut name_desc = cranelift_module::DataDescription::new();
                    name_desc.define(name_str.as_bytes().to_vec().into_boxed_slice());
                    self.module.define_data(name_data, &name_desc)?;
                    let name_gv = self
                        .module
                        .declare_data_in_func(name_data, self.builder.func);
                    let name_ptr = self.builder.ins().global_value(self.ptr_type, name_gv);
                    let name_len = self.builder.ins().iconst(types::I64, name_str.len() as i64);

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

                    let n_arities = template.arity_fn_names.len();
                    if n_arities == 1 && !template.is_variadic[0] {
                        // Single fixed arity — use rt_make_fn (simpler path).
                        let arity_fn_name = &template.arity_fn_names[0];
                        let param_count = template.param_counts[0];
                        let arity_func_id = self.user_funcs[arity_fn_name];
                        let func_ref = self
                            .module
                            .declare_func_in_func(arity_func_id, self.builder.func);
                        let fn_ptr = self.builder.ins().func_addr(self.ptr_type, func_ref);
                        let param_count_val =
                            self.builder.ins().iconst(types::I64, param_count as i64);

                        let rt_ref = self.import_func(self.rt.rt_make_fn);
                        let call = self.builder.ins().call(
                            rt_ref,
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
                    } else if n_arities == 1 && template.is_variadic[0] {
                        // Single variadic arity — use rt_make_fn_variadic.
                        let arity_fn_name = &template.arity_fn_names[0];
                        let param_count = template.param_counts[0];
                        let arity_func_id = self.user_funcs[arity_fn_name];
                        let func_ref = self
                            .module
                            .declare_func_in_func(arity_func_id, self.builder.func);
                        let fn_ptr = self.builder.ins().func_addr(self.ptr_type, func_ref);
                        let param_count_val =
                            self.builder.ins().iconst(types::I64, param_count as i64);

                        let rt_ref = self.import_func(self.rt.rt_make_fn_variadic);
                        let call = self.builder.ins().call(
                            rt_ref,
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
                    } else {
                        // Multi-arity — spill fn_ptrs, param_counts, and is_variadic arrays,
                        // then call rt_make_fn_multi.

                        // Stack-spill function pointers array.
                        let fn_ptrs_slot = self.builder.create_sized_stack_slot(
                            cranelift_codegen::ir::StackSlotData::new(
                                cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                                (n_arities * 8) as u32,
                                3,
                            ),
                        );
                        for (i, arity_fn_name) in template.arity_fn_names.iter().enumerate() {
                            let arity_func_id = self.user_funcs[arity_fn_name];
                            let func_ref = self
                                .module
                                .declare_func_in_func(arity_func_id, self.builder.func);
                            let fn_ptr = self.builder.ins().func_addr(self.ptr_type, func_ref);
                            self.builder
                                .ins()
                                .stack_store(fn_ptr, fn_ptrs_slot, (i * 8) as i32);
                        }
                        let fn_ptrs_addr =
                            self.builder
                                .ins()
                                .stack_addr(self.ptr_type, fn_ptrs_slot, 0);

                        // Stack-spill param_counts array (as i64 values).
                        let pc_slot = self.builder.create_sized_stack_slot(
                            cranelift_codegen::ir::StackSlotData::new(
                                cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                                (n_arities * 8) as u32,
                                3,
                            ),
                        );
                        for (i, &pc) in template.param_counts.iter().enumerate() {
                            let pc_val = self.builder.ins().iconst(types::I64, pc as i64);
                            self.builder
                                .ins()
                                .stack_store(pc_val, pc_slot, (i * 8) as i32);
                        }
                        let pc_addr = self.builder.ins().stack_addr(self.ptr_type, pc_slot, 0);

                        // Stack-spill is_variadic array (as u8 values, 1 byte each).
                        let var_slot = self.builder.create_sized_stack_slot(
                            cranelift_codegen::ir::StackSlotData::new(
                                cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                                n_arities as u32,
                                0,
                            ),
                        );
                        for (i, &v) in template.is_variadic.iter().enumerate() {
                            let v_val = self.builder.ins().iconst(types::I8, if v { 1 } else { 0 });
                            self.builder.ins().stack_store(v_val, var_slot, i as i32);
                        }
                        let var_addr = self.builder.ins().stack_addr(self.ptr_type, var_slot, 0);

                        let n_arities_val = self.builder.ins().iconst(types::I64, n_arities as i64);

                        // Call rt_make_fn_multi(name_ptr, name_len, fn_ptrs, param_counts,
                        //                      is_variadic, n_arities, captures, ncaptures)
                        let rt_ref = self.import_func(self.rt.rt_make_fn_multi);
                        let call = self.builder.ins().call(
                            rt_ref,
                            &[
                                name_ptr,
                                name_len,
                                fn_ptrs_addr,
                                pc_addr,
                                var_addr,
                                n_arities_val,
                                captures_ptr,
                                ncaptures_val,
                            ],
                        );
                        let result = self.builder.inst_results(call)[0];
                        let var = self.ensure_var(*dst);
                        self.builder.def_var(var, result);
                    }
                }
            }

            Inst::RegionStart(dst) => {
                // Allocate and activate a bump region on the thread-local stack.
                let val = self.call_rt_0(self.rt.rt_region_start)?;
                let var = self.ensure_var(*dst);
                self.builder.def_var(var, val);
            }

            Inst::RegionAlloc(dst, region, kind, operands) => {
                let region_handle = self.use_var(*region);
                let val = match kind {
                    RegionAllocKind::Vector => self.emit_region_alloc_collection(
                        self.rt.rt_region_alloc_vector,
                        region_handle,
                        operands,
                    )?,
                    RegionAllocKind::Map => self.emit_region_alloc_collection(
                        self.rt.rt_region_alloc_map,
                        region_handle,
                        operands,
                    )?,
                    RegionAllocKind::Set => self.emit_region_alloc_collection(
                        self.rt.rt_region_alloc_set,
                        region_handle,
                        operands,
                    )?,
                    RegionAllocKind::List => self.emit_region_alloc_collection(
                        self.rt.rt_region_alloc_list,
                        region_handle,
                        operands,
                    )?,
                    RegionAllocKind::Cons => {
                        if operands.len() == 2 {
                            let h = self.use_var(operands[0]);
                            let t = self.use_var(operands[1]);
                            let func_ref = self.import_func(self.rt.rt_region_alloc_cons);
                            let call = self.builder.ins().call(func_ref, &[region_handle, h, t]);
                            self.builder.inst_results(call)[0]
                        } else {
                            self.call_rt_0(self.rt.rt_const_nil)?
                        }
                    }
                };
                let var = self.ensure_var(*dst);
                self.builder.def_var(var, val);
            }

            Inst::RegionEnd(region) => {
                // Pop and free the bump region.
                let handle = self.use_var(*region);
                let func_ref = self.import_func(self.rt.rt_region_end);
                self.builder.ins().call(func_ref, &[handle]);
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
                // GC safepoint before looping back, so tight recur
                // loops cooperate with the collector.
                self.emit_safepoint();
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

    /// Emit a call to `rt_safepoint()`.
    fn emit_safepoint(&mut self) {
        let func_ref = self.import_func(self.rt.rt_safepoint);
        self.builder.ins().call(func_ref, &[]);
    }

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

    /// Emit a LoadVar (ns/name lookup) — returns the Var object, not its value.
    fn emit_load_var(
        &mut self,
        ns: &str,
        name: &str,
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

        let func_ref = self.import_func(self.rt.rt_load_var);
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

    /// Emit a region-aware collection allocation call.
    ///
    /// Like `emit_alloc_collection` but prepends the region handle as the first
    /// argument: `rt_region_alloc_*(handle, elems_ptr, count)`.
    fn emit_region_alloc_collection(
        &mut self,
        func_id: FuncId,
        region_handle: cranelift_codegen::ir::Value,
        elems: &[VarId],
    ) -> CodegenResult<cranelift_codegen::ir::Value> {
        let n = elems.len();
        if n == 0 {
            let func_ref = self.import_func(func_id);
            let null = self.builder.ins().iconst(self.ptr_type, 0);
            let zero = self.builder.ins().iconst(types::I64, 0);
            let call = self
                .builder
                .ins()
                .call(func_ref, &[region_handle, null, zero]);
            return Ok(self.builder.inst_results(call)[0]);
        }

        let slot = self
            .builder
            .create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                (n * 8) as u32,
                3,
            ));

        for (i, elem_var) in elems.iter().enumerate() {
            let val = self.use_var(*elem_var);
            self.builder.ins().stack_store(val, slot, (i * 8) as i32);
        }

        let slot_addr = self.builder.ins().stack_addr(self.ptr_type, slot, 0);
        let func_ref = self.import_func(func_id);
        let actual_count = if func_id == self.rt.rt_region_alloc_map {
            self.builder.ins().iconst(types::I64, (n / 2) as i64)
        } else {
            self.builder.ins().iconst(types::I64, n as i64)
        };
        let call = self
            .builder
            .ins()
            .call(func_ref, &[region_handle, slot_addr, actual_count]);
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
            KnownFn::WithBindings => return self.emit_with_bindings(args),
            KnownFn::Concat => return self.emit_alloc_collection(self.rt.rt_concat, args),
            KnownFn::Str if args.len() != 1 => {
                return self.emit_alloc_collection(self.rt.rt_str_n, args);
            }
            KnownFn::Println if args.len() != 1 => {
                return self.emit_alloc_collection(self.rt.rt_println_n, args);
            }
            KnownFn::Merge => return self.emit_alloc_collection(self.rt.rt_merge, args),
            KnownFn::Juxt => return self.emit_alloc_collection(self.rt.rt_juxt, args),
            KnownFn::Comp => return self.emit_alloc_collection(self.rt.rt_comp, args),
            KnownFn::Partial => return self.emit_alloc_collection(self.rt.rt_partial, args),
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
            KnownFn::SetBangVar => self.rt.rt_set_bang,
            KnownFn::Reduce2 => self.rt.rt_reduce2,
            KnownFn::Reduce3 => self.rt.rt_reduce3,
            KnownFn::Map => self.rt.rt_map,
            KnownFn::Filter => self.rt.rt_filter,
            KnownFn::Mapv => self.rt.rt_mapv,
            KnownFn::Filterv => self.rt.rt_filterv,
            KnownFn::Some => self.rt.rt_some,
            KnownFn::Every => self.rt.rt_every,
            KnownFn::Into => self.rt.rt_into,
            KnownFn::Into3 => self.rt.rt_into3,
            KnownFn::Range1 => self.rt.rt_range1,
            KnownFn::Range2 => self.rt.rt_range2,
            KnownFn::Range3 => self.rt.rt_range3,
            KnownFn::Take => self.rt.rt_take,
            KnownFn::Drop => self.rt.rt_drop,
            KnownFn::Reverse => self.rt.rt_reverse,
            KnownFn::Sort => self.rt.rt_sort,
            KnownFn::SortBy => self.rt.rt_sort_by,
            KnownFn::Keys => self.rt.rt_keys,
            KnownFn::Vals => self.rt.rt_vals,
            KnownFn::Update => self.rt.rt_update,
            KnownFn::GetIn => self.rt.rt_get_in,
            KnownFn::AssocIn => self.rt.rt_assoc_in,
            KnownFn::IsNumber => self.rt.rt_is_number,
            KnownFn::IsString => self.rt.rt_is_string,
            KnownFn::IsKeyword => self.rt.rt_is_keyword,
            KnownFn::IsSymbol => self.rt.rt_is_symbol,
            KnownFn::IsBool => self.rt.rt_is_bool,
            KnownFn::IsInt => self.rt.rt_is_int,
            KnownFn::Prn => self.rt.rt_prn,
            KnownFn::Print => self.rt.rt_print,
            KnownFn::Atom => self.rt.rt_atom,
            KnownFn::GroupBy => self.rt.rt_group_by,
            KnownFn::Partition2 => self.rt.rt_partition2,
            KnownFn::Partition3 => self.rt.rt_partition3,
            KnownFn::Partition4 => self.rt.rt_partition4,
            KnownFn::Frequencies => self.rt.rt_frequencies,
            KnownFn::Keep => self.rt.rt_keep,
            KnownFn::Remove => self.rt.rt_remove,
            KnownFn::MapIndexed => self.rt.rt_map_indexed,
            KnownFn::Zipmap => self.rt.rt_zipmap,
            KnownFn::Complement => self.rt.rt_complement,
            KnownFn::WithOutStr => self.rt.rt_with_out_str,
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
    fn emit_atom_swap(&mut self, args: &[VarId]) -> CodegenResult<cranelift_codegen::ir::Value> {
        // args[0] = atom, args[1] = f, args[2..] = extra args
        let atom_val = self.use_var(args[0]);
        let f_val = self.use_var(args[1]);
        let extra = &args[2..];
        let n = extra.len();

        let (extra_ptr, extra_count) = if n > 0 {
            let slot =
                self.builder
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

    /// Emit `(binding [var val ...] body)` via rt_with_bindings.
    ///
    /// args layout: [var0, val0, var1, val1, ..., body_closure]
    fn emit_with_bindings(
        &mut self,
        args: &[VarId],
    ) -> CodegenResult<cranelift_codegen::ir::Value> {
        // Last arg is the body closure, everything before is var/val pairs
        let body_var = *args.last().unwrap();
        let binding_args = &args[..args.len() - 1];
        let npairs = binding_args.len() / 2;

        let body_val = self.use_var(body_var);

        let (bindings_ptr, npairs_val) = if npairs > 0 {
            let n = binding_args.len();
            let slot =
                self.builder
                    .create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                        cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                        (n * 8) as u32,
                        3,
                    ));
            for (i, arg) in binding_args.iter().enumerate() {
                let val = self.use_var(*arg);
                self.builder.ins().stack_store(val, slot, (i * 8) as i32);
            }
            let addr = self.builder.ins().stack_addr(self.ptr_type, slot, 0);
            let count = self.builder.ins().iconst(types::I64, npairs as i64);
            (addr, count)
        } else {
            let null = self.builder.ins().iconst(self.ptr_type, 0);
            let zero = self.builder.ins().iconst(types::I64, 0);
            (null, zero)
        };

        let func_ref = self.import_func(self.rt.rt_with_bindings);
        let call = self
            .builder
            .ins()
            .call(func_ref, &[bindings_ptr, npairs_val, body_val]);
        Ok(self.builder.inst_results(call)[0])
    }

    /// Emit a direct function call (bypasses rt_call dynamic dispatch).
    fn emit_direct_call(
        &mut self,
        fn_name: &str,
        args: &[VarId],
    ) -> CodegenResult<cranelift_codegen::ir::Value> {
        let func_id = self.user_funcs.get(fn_name).ok_or_else(|| {
            CodegenError::Codegen(format!("CallDirect: unknown function {fn_name}"))
        })?;
        let func_ref = self.import_func(*func_id);
        let arg_vals: Vec<_> = args.iter().map(|a| self.use_var(*a)).collect();
        let call = self.builder.ins().call(func_ref, &arg_vals);
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
    // Declare rt_safepoint: void -> void.  We declare it as returning ptr
    // (ignored) to keep the signature uniform with declare_rt.
    let rt_safepoint = {
        let mut sig = module.make_signature();
        sig.call_conv = CallConv::SystemV;
        module.declare_function("rt_safepoint", Linkage::Import, &sig)?
    };

    Ok(RuntimeFuncs {
        rt_safepoint,
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
        // rt_make_fn_variadic(name_ptr, name_len, fn_ptr, fixed_param_count, captures, ncaptures)
        rt_make_fn_variadic: declare_rt(
            module,
            "rt_make_fn_variadic",
            &[ptr, types::I64, ptr, types::I64, ptr, types::I64],
            ptr,
        )?,
        // rt_make_fn_multi(name_ptr, name_len, fn_ptrs, param_counts, is_variadic, n_arities, captures, ncaptures)
        rt_make_fn_multi: declare_rt(
            module,
            "rt_make_fn_multi",
            &[ptr, types::I64, ptr, ptr, ptr, types::I64, ptr, types::I64],
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
        rt_atom_swap: declare_rt(module, "rt_atom_swap", &[ptr, ptr, ptr, types::I64], ptr)?,
        rt_apply: declare_rt(module, "rt_apply", &[ptr, ptr], ptr)?,
        rt_set_bang: declare_rt(module, "rt_set_bang", &[ptr, ptr], ptr)?,
        rt_with_bindings: declare_rt(module, "rt_with_bindings", &[ptr, types::I64, ptr], ptr)?,
        rt_load_var: declare_rt(
            module,
            "rt_load_var",
            &[ptr, types::I64, ptr, types::I64],
            ptr,
        )?,
        rt_reduce2: declare_rt(module, "rt_reduce2", &[ptr, ptr], ptr)?,
        rt_reduce3: declare_rt(module, "rt_reduce3", &[ptr, ptr, ptr], ptr)?,
        rt_map: declare_rt(module, "rt_map", &[ptr, ptr], ptr)?,
        rt_filter: declare_rt(module, "rt_filter", &[ptr, ptr], ptr)?,
        rt_mapv: declare_rt(module, "rt_mapv", &[ptr, ptr], ptr)?,
        rt_filterv: declare_rt(module, "rt_filterv", &[ptr, ptr], ptr)?,
        rt_some: declare_rt(module, "rt_some", &[ptr, ptr], ptr)?,
        rt_every: declare_rt(module, "rt_every", &[ptr, ptr], ptr)?,
        rt_into: declare_rt(module, "rt_into", &[ptr, ptr], ptr)?,
        rt_into3: declare_rt(module, "rt_into3", &[ptr, ptr, ptr], ptr)?,
        rt_group_by: declare_rt(module, "rt_group_by", &[ptr, ptr], ptr)?,
        rt_partition2: declare_rt(module, "rt_partition2", &[ptr, ptr], ptr)?,
        rt_partition3: declare_rt(module, "rt_partition3", &[ptr, ptr, ptr], ptr)?,
        rt_partition4: declare_rt(module, "rt_partition4", &[ptr, ptr, ptr, ptr], ptr)?,
        rt_frequencies: declare_rt(module, "rt_frequencies", &[ptr], ptr)?,
        rt_keep: declare_rt(module, "rt_keep", &[ptr, ptr], ptr)?,
        rt_remove: declare_rt(module, "rt_remove", &[ptr, ptr], ptr)?,
        rt_map_indexed: declare_rt(module, "rt_map_indexed", &[ptr, ptr], ptr)?,
        rt_zipmap: declare_rt(module, "rt_zipmap", &[ptr, ptr], ptr)?,
        rt_juxt: declare_rt(module, "rt_juxt", &[ptr, types::I64], ptr)?,
        rt_comp: declare_rt(module, "rt_comp", &[ptr, types::I64], ptr)?,
        rt_partial: declare_rt(module, "rt_partial", &[ptr, types::I64], ptr)?,
        rt_complement: declare_rt(module, "rt_complement", &[ptr], ptr)?,
        rt_concat: declare_rt(module, "rt_concat", &[ptr, types::I64], ptr)?,
        rt_range1: declare_rt(module, "rt_range1", &[ptr], ptr)?,
        rt_range2: declare_rt(module, "rt_range2", &[ptr, ptr], ptr)?,
        rt_range3: declare_rt(module, "rt_range3", &[ptr, ptr, ptr], ptr)?,
        rt_take: declare_rt(module, "rt_take", &[ptr, ptr], ptr)?,
        rt_drop: declare_rt(module, "rt_drop", &[ptr, ptr], ptr)?,
        rt_reverse: declare_rt(module, "rt_reverse", &[ptr], ptr)?,
        rt_sort: declare_rt(module, "rt_sort", &[ptr], ptr)?,
        rt_sort_by: declare_rt(module, "rt_sort_by", &[ptr, ptr], ptr)?,
        rt_keys: declare_rt(module, "rt_keys", &[ptr], ptr)?,
        rt_vals: declare_rt(module, "rt_vals", &[ptr], ptr)?,
        rt_merge: declare_rt(module, "rt_merge", &[ptr, types::I64], ptr)?,
        rt_update: declare_rt(module, "rt_update", &[ptr, ptr, ptr], ptr)?,
        rt_get_in: declare_rt(module, "rt_get_in", &[ptr, ptr], ptr)?,
        rt_assoc_in: declare_rt(module, "rt_assoc_in", &[ptr, ptr, ptr], ptr)?,
        rt_is_number: declare_rt(module, "rt_is_number", &[ptr], ptr)?,
        rt_is_string: declare_rt(module, "rt_is_string", &[ptr], ptr)?,
        rt_is_keyword: declare_rt(module, "rt_is_keyword", &[ptr], ptr)?,
        rt_is_symbol: declare_rt(module, "rt_is_symbol", &[ptr], ptr)?,
        rt_is_bool: declare_rt(module, "rt_is_bool", &[ptr], ptr)?,
        rt_is_int: declare_rt(module, "rt_is_int", &[ptr], ptr)?,
        rt_prn: declare_rt(module, "rt_prn", &[ptr], ptr)?,
        rt_print: declare_rt(module, "rt_print", &[ptr], ptr)?,
        rt_atom: declare_rt(module, "rt_atom", &[ptr], ptr)?,
        rt_str_n: declare_rt(module, "rt_str_n", &[ptr, types::I64], ptr)?,
        rt_println_n: declare_rt(module, "rt_println_n", &[ptr, types::I64], ptr)?,
        rt_with_out_str: declare_rt(module, "rt_with_out_str", &[ptr], ptr)?,
        // Region allocation
        rt_region_start: declare_rt(module, "rt_region_start", &[], ptr)?,
        rt_region_end: declare_rt(module, "rt_region_end", &[ptr], ptr)?,
        rt_region_alloc_vector: declare_rt(
            module,
            "rt_region_alloc_vector",
            &[ptr, ptr, types::I64],
            ptr,
        )?,
        rt_region_alloc_map: declare_rt(
            module,
            "rt_region_alloc_map",
            &[ptr, ptr, types::I64],
            ptr,
        )?,
        rt_region_alloc_set: declare_rt(
            module,
            "rt_region_alloc_set",
            &[ptr, ptr, types::I64],
            ptr,
        )?,
        rt_region_alloc_list: declare_rt(
            module,
            "rt_region_alloc_list",
            &[ptr, ptr, types::I64],
            ptr,
        )?,
        rt_region_alloc_cons: declare_rt(module, "rt_region_alloc_cons", &[ptr, ptr, ptr], ptr)?,
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
