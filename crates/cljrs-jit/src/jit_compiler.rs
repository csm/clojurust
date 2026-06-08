//! Compile a single [`IrFunction`] to native code via Cranelift JIT.

use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Module};

use cljrs_compiler::codegen::new_compiler_from_module;
use cljrs_compiler::rt_abi;
use cljrs_ir::IrFunction;

/// A successfully compiled JIT function.
///
/// The `JITModule` inside owns the executable memory; it must remain alive
/// for as long as `fn_ptr` may be called.  Phase 10.2 implements unloading.
pub(crate) struct CompiledFn {
    pub(crate) fn_ptr: *const (),
    /// Keeps the executable memory alive.
    pub(crate) _module: JITModule,
}

// SAFETY: `fn_ptr` points into `JITModule`'s memory-mapped code section.
// The JITModule is stored alongside the pointer and never freed here.
// The function can be called from any thread that holds the pointer.
unsafe impl Send for CompiledFn {}

/// Compile `ir_func` to native code, returning the function pointer and the
/// owning module.
pub(crate) fn compile_jit(arity_id: u64, ir_func: &IrFunction) -> Result<CompiledFn, String> {
    let mut builder = JITBuilder::new(cranelift_module::default_libcall_names())
        .map_err(|e| format!("JITBuilder::new: {e}"))?;

    // Register all rt_abi runtime bridge symbols so JIT-emitted IMPORT calls
    // resolve in-process.  Without explicit registration, dlsym would be used;
    // on most platforms, static-binary symbols are not dlsym-visible.
    register_rt_abi_symbols(&mut builder);

    let jit_module = JITModule::new(builder);
    let ptr_type = jit_module.isa().pointer_type();

    let mut compiler = new_compiler_from_module(jit_module, ptr_type)
        .map_err(|e| format!("new_compiler_from_module: {e:?}"))?;

    let func_name = format!("__cljrs_jit_{arity_id}");
    let param_count = ir_func.params.len();

    let func_id: FuncId = compiler
        .declare_function(&func_name, param_count)
        .map_err(|e| format!("declare_function: {e:?}"))?;

    compiler
        .compile_function(ir_func, func_id)
        .map_err(|e| format!("compile_function: {e:?}"))?;

    let mut jit_module = compiler.into_inner_module();

    jit_module
        .finalize_definitions()
        .map_err(|e| format!("finalize_definitions: {e}"))?;

    let fn_ptr = jit_module.get_finalized_function(func_id) as *const ();

    Ok(CompiledFn {
        fn_ptr,
        _module: jit_module,
    })
}

// ── rt_abi symbol registration ────────────────────────────────────────────────

/// Register every `extern "C"` rt_abi bridge function with the JITBuilder.
fn register_rt_abi_symbols(builder: &mut JITBuilder) {
    // Helper: cast an `extern "C"` function pointer to *const u8.
    // All rt_abi functions are safe `extern "C"` fn; the cast is well-defined.
    macro_rules! sym {
        ($f:ident) => {
            (stringify!($f), rt_abi::$f as *const () as usize as *const u8)
        };
    }

    #[rustfmt::skip]
    let symbols: &[(&str, *const u8)] = &[
        sym!(rt_safepoint),
        sym!(rt_const_nil),
        sym!(rt_const_true),
        sym!(rt_const_false),
        sym!(rt_const_long),
        sym!(rt_const_double),
        sym!(rt_const_char),
        sym!(rt_const_string),
        sym!(rt_const_keyword),
        sym!(rt_const_symbol),
        sym!(rt_truthiness),
        sym!(rt_add),
        sym!(rt_sub),
        sym!(rt_mul),
        sym!(rt_div),
        sym!(rt_rem),
        sym!(rt_eq),
        sym!(rt_lt),
        sym!(rt_gt),
        sym!(rt_lte),
        sym!(rt_gte),
        sym!(rt_alloc_vector),
        sym!(rt_alloc_map),
        sym!(rt_alloc_set),
        sym!(rt_alloc_list),
        sym!(rt_alloc_cons),
        sym!(rt_get),
        sym!(rt_count),
        sym!(rt_count_filter),
        sym!(rt_into_filter),
        sym!(rt_into_mapcat),
        sym!(rt_into_map),
        sym!(rt_first),
        sym!(rt_rest),
        sym!(rt_assoc),
        sym!(rt_conj),
        sym!(rt_call),
        sym!(rt_deref),
        sym!(rt_println),
        sym!(rt_pr),
        sym!(rt_is_nil),
        sym!(rt_is_vector),
        sym!(rt_is_map),
        sym!(rt_is_seq),
        sym!(rt_identical),
        sym!(rt_str),
        sym!(rt_load_global),
        sym!(rt_def_var),
        sym!(rt_make_fn),
        sym!(rt_make_fn_variadic),
        sym!(rt_make_fn_multi),
        sym!(rt_throw),
        sym!(rt_try),
        sym!(rt_dissoc),
        sym!(rt_disj),
        sym!(rt_nth),
        sym!(rt_contains),
        sym!(rt_seq),
        sym!(rt_lazy_seq),
        sym!(rt_transient),
        sym!(rt_assoc_bang),
        sym!(rt_conj_bang),
        sym!(rt_persistent_bang),
        sym!(rt_atom_reset),
        sym!(rt_atom_swap),
        sym!(rt_apply),
        sym!(rt_set_bang),
        sym!(rt_with_bindings),
        sym!(rt_load_var),
        sym!(rt_reduce2),
        sym!(rt_reduce3),
        sym!(rt_map),
        sym!(rt_filter),
        sym!(rt_mapv),
        sym!(rt_filterv),
        sym!(rt_some),
        sym!(rt_every),
        sym!(rt_into),
        sym!(rt_into3),
        sym!(rt_group_by),
        sym!(rt_partition2),
        sym!(rt_partition3),
        sym!(rt_partition4),
        sym!(rt_frequencies),
        sym!(rt_keep),
        sym!(rt_remove),
        sym!(rt_map_indexed),
        sym!(rt_zipmap),
        sym!(rt_juxt),
        sym!(rt_comp),
        sym!(rt_partial),
        sym!(rt_complement),
        sym!(rt_concat),
        sym!(rt_range1),
        sym!(rt_range2),
        sym!(rt_range3),
        sym!(rt_take),
        sym!(rt_drop),
        sym!(rt_reverse),
        sym!(rt_sort),
        sym!(rt_sort_by),
        sym!(rt_keys),
        sym!(rt_vals),
        sym!(rt_merge),
        sym!(rt_update),
        sym!(rt_get_in),
        sym!(rt_assoc_in),
        sym!(rt_is_number),
        sym!(rt_is_string),
        sym!(rt_is_keyword),
        sym!(rt_is_symbol),
        sym!(rt_is_bool),
        sym!(rt_is_int),
        sym!(rt_prn),
        sym!(rt_print),
        sym!(rt_atom),
        sym!(rt_str_n),
        sym!(rt_println_n),
        sym!(rt_with_out_str),
        sym!(rt_peek),
        sym!(rt_pop),
        sym!(rt_vec),
        sym!(rt_mapcat),
        sym!(rt_is_empty),
        sym!(rt_repeatedly),
        sym!(rt_region_start),
        sym!(rt_region_end),
        sym!(rt_region_alloc_vector),
        sym!(rt_region_alloc_map),
        sym!(rt_region_alloc_set),
        sym!(rt_region_alloc_list),
        sym!(rt_region_alloc_cons),
    ];

    builder.symbols(symbols.iter().map(|&(n, p)| (n, p)));
}
