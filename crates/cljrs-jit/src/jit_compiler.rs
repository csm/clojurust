//! Compile a single [`IrFunction`] to native code via Cranelift JIT.

use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Module};

use cljrs_compiler::codegen::new_compiler_from_module;
use cljrs_compiler::rt_abi;
use cljrs_compiler::typeinfer::Repr;
use cljrs_ir::IrFunction;

/// A successfully compiled JIT function.
///
/// The `JITModule` inside owns the executable memory; it must remain alive for
/// as long as `fn_ptr` may be called.  The code cache (`code_cache.rs`) takes
/// ownership and, once the code is stale and no frame executes it, reclaims the
/// memory via [`JITModule::free_memory`] at a stop-the-world safepoint.
pub(crate) struct CompiledFn {
    pub(crate) fn_ptr: *const (),
    /// Owns the executable memory; reclaimed via `module.free_memory()`.
    pub(crate) module: JITModule,
    /// Machine-code size in bytes (for memory accounting / diagnostics).
    pub(crate) code_size: u32,
}

// SAFETY: `fn_ptr` points into `JITModule`'s memory-mapped code section, which
// the owned `JITModule` keeps alive.  The function can be called from any thread
// that holds the pointer; the module is `Send`.
unsafe impl Send for CompiledFn {}

/// Compile `ir_func` to native code, returning the function pointer and the
/// owning module.  `func_name` only names the symbol inside this module
/// (each compilation gets its own `JITModule`).
///
/// `specs` are the per-parameter type specializations (Phase 10.6) for the
/// *top-level* function only; subfunctions (closures, region variants) are
/// always compiled generically.  Pass `&[]` for an unspecialized compile.
pub(crate) fn compile_jit(
    func_name: &str,
    ir_func: &IrFunction,
    specs: &[Repr],
) -> Result<CompiledFn, String> {
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

    // `abi_param_count` includes the hidden trailing region parameter of
    // region-parameterised variants (the top-level function never has one,
    // but its `__rg` subfunction variants do).
    let param_count = ir_func.abi_param_count();

    let func_id: FuncId = compiler
        .declare_function(func_name, param_count)
        .map_err(|e| format!("declare_function: {e:?}"))?;

    // Declare and compile closure subfunctions into the same module (exactly
    // as AOT does), so `AllocClosure` codegen can resolve each arity's
    // function by name.  Closure values built from these pointers may outlive
    // the call — `rt_make_fn*` fires the closure-escape hook, which pins this
    // module's reclamation epoch (see `code_cache::pin_epoch`).
    declare_subfunctions(ir_func, &mut compiler)?;
    compile_subfunctions(ir_func, &mut compiler)?;

    compiler
        .compile_function_with_specs(ir_func, func_id, specs)
        .map_err(|e| format!("compile_function: {e:?}"))?;

    let code_size = compiler.last_code_size();

    let mut jit_module = compiler.into_inner_module();

    jit_module
        .finalize_definitions()
        .map_err(|e| format!("finalize_definitions: {e}"))?;

    let fn_ptr = jit_module.get_finalized_function(func_id) as *const ();

    Ok(CompiledFn {
        fn_ptr,
        module: jit_module,
        code_size,
    })
}

/// Recursively declare all closure subfunctions so they can reference each
/// other (mirrors `aot.rs::declare_subfunctions`).
fn declare_subfunctions<M: cranelift_module::Module>(
    ir_func: &IrFunction,
    compiler: &mut cljrs_compiler::codegen::Compiler<M>,
) -> Result<(), String> {
    for sub in &ir_func.subfunctions {
        let name = sub.name.as_deref().unwrap_or("__cljrs_anon");
        compiler
            .declare_function(name, sub.abi_param_count())
            .map_err(|e| format!("declare sub {name}: {e:?}"))?;
        declare_subfunctions(sub, compiler)?;
    }
    Ok(())
}

/// Recursively compile all closure subfunctions, innermost first (mirrors
/// `aot.rs::compile_subfunctions`).
fn compile_subfunctions<M: cranelift_module::Module>(
    ir_func: &IrFunction,
    compiler: &mut cljrs_compiler::codegen::Compiler<M>,
) -> Result<(), String> {
    for sub in &ir_func.subfunctions {
        compile_subfunctions(sub, compiler)?;
        let name = sub.name.as_deref().unwrap_or("__cljrs_anon");
        let func_id = compiler
            .declare_function(name, sub.abi_param_count())
            .map_err(|e| format!("redeclare sub {name}: {e:?}"))?;
        compiler
            .compile_function(sub, func_id)
            .map_err(|e| format!("compile sub {name}: {e:?}"))?;
    }
    Ok(())
}

// ── rt_abi symbol registration ────────────────────────────────────────────────

/// Register every `extern "C"` rt_abi bridge function with the JITBuilder.
fn register_rt_abi_symbols(builder: &mut JITBuilder) {
    // Helper: cast an `extern "C"` function pointer to *const u8.
    // All rt_abi functions are safe `extern "C"` fn; the cast is well-defined.
    macro_rules! sym {
        ($f:ident) => {
            (
                stringify!($f),
                rt_abi::$f as *const () as usize as *const u8,
            )
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
        sym!(rt_unchecked_add),
        sym!(rt_unchecked_sub),
        sym!(rt_unchecked_mul),
        sym!(rt_overflow_error),
        sym!(rt_alength),
        sym!(rt_aget_long),
        sym!(rt_aget_double),
        sym!(rt_aset_long),
        sym!(rt_aset_double),
        sym!(rt_aget),
        sym!(rt_aset),
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
        // Specialization & inline caches (Phase 10.6)
        sym!(rt_value_tag),
        sym!(rt_unbox_long),
        sym!(rt_unbox_double),
        sym!(rt_box_bool),
        sym!(rt_deopt),
        sym!(rt_kw_ic_fill),
        sym!(rt_call_ic),
        sym!(rt_load_global_versioned_ic),
    ];

    builder.symbols(symbols.iter().map(|&(n, p)| (n, p)));
}
