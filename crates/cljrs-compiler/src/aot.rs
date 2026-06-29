//! AOT compilation driver.
//!
//! Orchestrates the full pipeline from source file to native binary:
//!
//! 1. Parse source → `Vec<Form>`
//! 2. Boot a standard environment (for macro expansion)
//! 3. Macro-expand each top-level form
//! 4. ANF-lower all forms as a zero-arg `__cljrs_main` function
//! 5. Cranelift codegen → `.o` object bytes
//! 6. Generate a Cargo harness project that links the object + runtime
//! 7. `cargo build --release` the harness → standalone binary

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cljrs_reader::Parser;

use crate::codegen::Compiler;
use crate::ir::IrFunction;

// ── Error type ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum AotError {
    Io(std::io::Error),
    Parse(cljrs_types::error::CljxError),
    Codegen(crate::codegen::CodegenError),
    Eval(String),
    Link(String),
    /// The wasm backend could not lower a construct in the program.
    Wasm(crate::wasm::WasmError),
    /// One or more no-gc memory-safety violations were found by the blacklist
    /// analysis.  Only emitted when the `no-gc` Cargo feature is active.
    #[cfg(feature = "no-gc")]
    NoGcBlacklist(Vec<crate::escape::BlacklistViolation>),
}

impl std::fmt::Display for AotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AotError::Io(e) => write!(f, "I/O error: {e}"),
            AotError::Parse(e) => write!(f, "parse error: {e}"),
            AotError::Codegen(e) => write!(f, "codegen error: {e:?}"),
            AotError::Eval(e) => write!(f, "eval/lowering error: {e}"),
            AotError::Link(e) => write!(f, "link error: {e}"),
            AotError::Wasm(e) => write!(f, "wasm backend error: {e}"),
            #[cfg(feature = "no-gc")]
            AotError::NoGcBlacklist(vs) => {
                writeln!(f, "no-gc blacklist violations:")?;
                for v in vs {
                    writeln!(f, "  • {v}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for AotError {}

impl From<std::io::Error> for AotError {
    fn from(e: std::io::Error) -> Self {
        AotError::Io(e)
    }
}
impl From<cljrs_types::error::CljxError> for AotError {
    fn from(e: cljrs_types::error::CljxError) -> Self {
        AotError::Parse(e)
    }
}
impl From<crate::codegen::CodegenError> for AotError {
    fn from(e: crate::codegen::CodegenError) -> Self {
        AotError::Codegen(e)
    }
}
impl From<crate::wasm::WasmError> for AotError {
    fn from(e: crate::wasm::WasmError) -> Self {
        AotError::Wasm(e)
    }
}

pub type AotResult<T> = Result<T, AotError>;

// ── Rust-native lowering ────────────────────────────────────────────────────

/// Lower forms directly via the native Rust compiler pipeline
/// (no interpreter round-trip).
pub fn lower_via_rust(
    name: Option<&str>,
    ns: &str,
    params: &[Arc<str>],
    compilable_forms: &[cljrs_reader::Form],
    _env: &mut cljrs_eval::Env,
) -> AotResult<IrFunction> {
    let ns_arc: Arc<str> = Arc::from(ns);
    let ir = cljrs_ir::lower::lower_fn_body(name, &ns_arc, params, compilable_forms, false)
        .map_err(|e| AotError::Eval(format!("lowering: {e:?}")))?;
    let ir = cljrs_ir::lower::optimize(ir);

    #[cfg(feature = "no-gc")]
    {
        let violations = crate::escape::check(&ir);
        if !violations.is_empty() {
            return Err(AotError::NoGcBlacklist(violations));
        }
    }

    Ok(ir)
}

// ── Direct call optimization ────────────────────────────────────────────────

/// Information about a compiled function arity.
#[derive(Debug, Clone)]
struct ArityInfo {
    fn_name: Arc<str>,
    param_count: usize,
    is_variadic: bool,
}

/// Collect top-level function definitions from an IR function.
///
/// Scans for `AllocClosure(...) + DefVar(_, ns, name, closure_var)` patterns
/// and returns a map from `(ns, name)` → list of arity infos.
fn collect_defn_arities(
    ir_func: &IrFunction,
) -> std::collections::HashMap<(Arc<str>, Arc<str>), Vec<ArityInfo>> {
    use crate::ir::{ClosureTemplate, Inst, VarId};
    use std::collections::HashMap;

    let mut closure_templates: HashMap<VarId, ClosureTemplate> = HashMap::new();
    let mut defns: HashMap<(Arc<str>, Arc<str>), Vec<ArityInfo>> = HashMap::new();

    for block in &ir_func.blocks {
        for inst in &block.insts {
            match inst {
                Inst::AllocClosure(dst, template, captures) if captures.is_empty() => {
                    // Only consider zero-capture closures (top-level defns).
                    closure_templates.insert(*dst, template.clone());
                }
                Inst::DefVar(_, ns, name, val) => {
                    if let Some(template) = closure_templates.get(val) {
                        let arities: Vec<ArityInfo> = template
                            .arity_fn_names
                            .iter()
                            .zip(template.param_counts.iter())
                            .zip(template.is_variadic.iter())
                            .map(|((fn_name, &param_count), &is_variadic)| ArityInfo {
                                fn_name: fn_name.clone(),
                                param_count,
                                is_variadic,
                            })
                            .collect();
                        defns.insert((ns.clone(), name.clone()), arities);
                    }
                }
                _ => {}
            }
        }
    }

    defns
}

/// Find the arity function name that matches a given argument count.
///
/// Only matches fixed arities — variadic functions cannot be called directly
/// because the runtime needs to pack extra args into a rest list.
fn find_matching_arity(arities: &[ArityInfo], arg_count: usize) -> Option<&ArityInfo> {
    arities
        .iter()
        .find(|arity| !arity.is_variadic && arity.param_count == arg_count)
}

/// Rewrite `LoadGlobal + Call` sequences to `CallDirect` for functions
/// defined in the same compilation unit.
///
/// This is a perf optimization: instead of going through `rt_call` (which
/// looks up the var in the interpreter and dispatches dynamically), we call
/// the compiled function pointer directly.
fn optimize_direct_calls(ir_func: &mut IrFunction) {
    // Collect defn arities from this function AND all subfunctions (recursively).
    let mut all_defns = collect_defn_arities(ir_func);
    for sub in &ir_func.subfunctions {
        // Subfunctions don't typically DefVar, but recurse just in case.
        all_defns.extend(collect_defn_arities(sub));
    }

    if all_defns.is_empty() {
        return;
    }

    let rewrites = rewrite_calls_to_direct(ir_func, &all_defns);
    if rewrites > 0 {
        eprintln!("[aot] optimized {rewrites} call(s) to direct function calls");
    }

    // Recursively optimize subfunctions too.
    // Subfunctions can call top-level defns, so pass the defn map down.
    for sub in &mut ir_func.subfunctions {
        optimize_direct_calls_with_defns(sub, &all_defns);
    }
}

/// Like `optimize_direct_calls` but uses a pre-built defn map (for subfunctions).
fn optimize_direct_calls_with_defns(
    ir_func: &mut IrFunction,
    defns: &std::collections::HashMap<(Arc<str>, Arc<str>), Vec<ArityInfo>>,
) {
    // Merge parent defns with any defns from this function.
    let mut all_defns = defns.clone();
    all_defns.extend(collect_defn_arities(ir_func));

    if all_defns.is_empty() {
        return;
    }

    let rewrites = rewrite_calls_to_direct(ir_func, &all_defns);
    if rewrites > 0 {
        eprintln!("[aot] optimized {rewrites} direct call(s) in subfunction");
    }

    for sub in &mut ir_func.subfunctions {
        optimize_direct_calls_with_defns(sub, &all_defns);
    }
}

/// Rewrite `LoadGlobal + Call` → `CallDirect` in a single IR function.
/// Returns the number of rewrites performed.
fn rewrite_calls_to_direct(
    ir_func: &mut IrFunction,
    defns: &std::collections::HashMap<(Arc<str>, Arc<str>), Vec<ArityInfo>>,
) -> usize {
    use crate::ir::{Inst, VarId};
    use std::collections::HashMap;

    // Build a map of VarId → (ns, name) for LoadGlobal instructions that load known defns.
    let mut loadglobal_targets: HashMap<VarId, (Arc<str>, Arc<str>)> = HashMap::new();
    for block in &ir_func.blocks {
        for inst in &block.insts {
            if let Inst::LoadGlobal(dst, ns, name) = inst
                && defns.contains_key(&(ns.clone(), name.clone()))
            {
                loadglobal_targets.insert(*dst, (ns.clone(), name.clone()));
            }
        }
    }

    let mut rewrites = 0;
    for block in &mut ir_func.blocks {
        for inst in &mut block.insts {
            if let Inst::Call(dst, callee, args) = inst
                && let Some((ns, name)) = loadglobal_targets.get(callee)
                && let Some(arities) = defns.get(&(ns.clone(), name.clone()))
                && let Some(arity_info) = find_matching_arity(arities, args.len())
            {
                *inst = Inst::CallDirect(*dst, arity_info.fn_name.clone(), args.clone());
                rewrites += 1;
            }
        }
    }

    rewrites
}

/// Tally region vs heap allocations across an IR function tree, so the
/// AOT pipeline can report the impact of escape analysis at a glance.
#[derive(Default)]
struct AllocStats {
    region: usize,
    heap: usize,
    closures: usize,
    functions: usize,
}

fn count_alloc_stats(ir_func: &IrFunction) -> AllocStats {
    use crate::ir::Inst;
    let mut stats = AllocStats {
        functions: 1,
        ..Default::default()
    };
    for block in &ir_func.blocks {
        for inst in &block.insts {
            match inst {
                Inst::AllocVector(..)
                | Inst::AllocMap(..)
                | Inst::AllocSet(..)
                | Inst::AllocList(..)
                | Inst::AllocCons(..) => stats.heap += 1,
                Inst::AllocClosure(..) => stats.closures += 1,
                Inst::RegionAlloc(..) => stats.region += 1,
                _ => {}
            }
        }
    }
    for sub in &ir_func.subfunctions {
        let s = count_alloc_stats(sub);
        stats.region += s.region;
        stats.heap += s.heap;
        stats.closures += s.closures;
        stats.functions += s.functions;
    }
    stats
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Run the AOT pipeline up to (and including) ANF lowering + region
/// optimization, but stop before code generation.  Returns the source text
/// and the optimized `IrFunction` so tools like `cljrs-ir-viz` can inspect
/// exactly what the AOT compiler would lower.
///
/// The `silent` flag suppresses the usual `[aot] ...` progress output.
pub fn lower_file_to_ir(
    src_path: &Path,
    src_dirs: &[PathBuf],
    silent: bool,
) -> AotResult<(String, IrFunction)> {
    macro_rules! note {
        ($($arg:tt)*) => { if !silent { eprintln!($($arg)*); } };
    }

    note!("[aot] reading {}", src_path.display());
    let source = std::fs::read_to_string(src_path)?;
    let filename = src_path.display().to_string();

    let mut parser = Parser::new(source.clone(), filename);
    let forms = parser.parse_all()?;
    note!("[aot] parsed {} top-level form(s)", forms.len());

    let globals = if src_dirs.is_empty() {
        cljrs_stdlib::standard_env()
    } else {
        cljrs_stdlib::standard_env_with_paths(src_dirs.to_vec())
    };
    let mut env = cljrs_eval::Env::new(globals, "user");

    let mut expanded = Vec::with_capacity(forms.len());
    for form in &forms {
        if needs_interpreter(form) {
            match cljrs_eval::eval(form, &mut env) {
                Ok(_) => {}
                Err(e) => return Err(AotError::Eval(format!("{e:?}"))),
            }
        }
        match cljrs_interp::macros::macroexpand_all(form, &mut env) {
            Ok(f) => expanded.push(f),
            Err(e) => return Err(AotError::Eval(format!("{e:?}"))),
        }
    }
    note!("[aot] macro-expanded {} form(s)", expanded.len());

    let mut compilable = Vec::new();
    for (i, form) in expanded.iter().enumerate() {
        if needs_interpreter(&forms[i]) || expanded_needs_interpreter(form) {
            continue;
        }
        compilable.push(form.clone());
    }

    let params: Vec<Arc<str>> = vec![];
    let compilable_forms = if compilable.is_empty() {
        let nil_form = cljrs_reader::Form::new(
            cljrs_reader::form::FormKind::Nil,
            cljrs_types::span::Span::new(Arc::new("<aot>".to_string()), 0, 0, 1, 1),
        );
        vec![nil_form]
    } else {
        compilable
    };

    let current_ns = env.current_ns.to_string();
    let ir_func = lower_via_rust(
        Some("__cljrs_main"),
        &current_ns,
        &params,
        &compilable_forms,
        &mut env,
    )?;
    note!(
        "[aot] lowered to {} block(s), {} var(s)",
        ir_func.blocks.len(),
        ir_func.next_var
    );
    Ok((source, ir_func))
}

/// AOT-compile a source file to a standalone WebAssembly module.
///
/// Lowers the entry namespace to IR (parse → macro-expand → ANF/region
/// optimization, via [`lower_file_to_ir`]), rewrites same-compilation-unit calls
/// to `CallDirect` ([`optimize_direct_calls`], so they resolve to wasm function
/// indices), then drives [`crate::wasm::compile_bundle`] over the entry function
/// and its flattened [`IrFunction::subfunctions`], writing the validated module
/// bytes to `out_path`.
///
/// This produces the **AOT code-generation artifact** for the entry namespace:
/// a wasm module whose `"rt"` imports (the `rt_abi` bridges, linear memory, and
/// the shared function table) are satisfied by the runtime compiled to
/// `wasm32-unknown-unknown`.  Linking the two — and wiring the IR interpreter in
/// as the dynamic-code tier, which is where the rodata / function-table bases
/// ([`crate::wasm::abi::RODATA_BASE`], [`crate::wasm::abi::FUNC_TABLE_BASE`]) are
/// finalized against the runtime's actual memory/table layout — is the remaining
/// bundling step tracked in `docs/wasm-aot-plan.md`.  Cross-namespace
/// dependencies are not yet bundled (only the entry namespace's functions are
/// emitted); same-unit calls resolve directly, others dispatch through `rt_call`.
pub fn compile_file_to_wasm(
    src_path: &Path,
    out_path: &Path,
    src_dirs: &[PathBuf],
) -> AotResult<()> {
    let (_source, mut ir_func) = lower_file_to_ir(src_path, src_dirs, false)?;

    // Resolve same-compilation-unit calls to direct calls so they bind to wasm
    // function indices rather than dispatching dynamically through `rt_call`.
    optimize_direct_calls(&mut ir_func);

    eprintln!("[aot] emitting wasm module");
    let cfg = crate::wasm::WasmBackend::default();
    let bytes = crate::wasm::compile_bundle(&[&ir_func], &cfg)?;

    std::fs::write(out_path, &bytes)?;
    eprintln!(
        "[aot] wrote wasm module ({} bytes) to {}",
        bytes.len(),
        out_path.display()
    );
    Ok(())
}

/// Compile a `.cljrs` / `.cljc` source file to a standalone native binary.
///
/// `src_path` is the input source file.  `out_path` is the desired output
/// binary.  `src_dirs` are additional directories for `require` resolution
/// during macro expansion.  `rust_config`, when present, causes the generated
/// harness to depend on the user's Rust crate and call its `cljrs_init` hook
/// before loading any Clojure code.  `verify_commit_signatures` enables
/// `git verify-commit` on every versioned pin resolved during compilation
/// (the produced binary trusts its embedded sources, so verification happens
/// here, at compile time).
pub fn compile_file(
    src_path: &Path,
    out_path: &Path,
    src_dirs: &[PathBuf],
    rust_config: Option<&cljrs_deps::RustConfig>,
    verify_commit_signatures: bool,
) -> AotResult<()> {
    eprintln!("[aot] reading {}", src_path.display());
    let source = std::fs::read_to_string(src_path)?;
    let filename = src_path.display().to_string();

    // ── 1. Parse ────────────────────────────────────────────────────────
    let mut parser = Parser::new(source.clone(), filename);
    let forms = parser.parse_all()?;
    eprintln!("[aot] parsed {} top-level form(s)", forms.len());

    // Resolve reader conditionals (`#?(...)`) to the `:rust` branch before
    // macro-expansion — a selected branch may itself contain macros.
    let forms = expand_reader_conds_deep(&forms);

    // ── 2. Macro-expand ─────────────────────────────────────────────────
    // Boot a full environment so macros resolve correctly.
    let globals = if src_dirs.is_empty() {
        cljrs_stdlib::standard_env()
    } else {
        cljrs_stdlib::standard_env_with_paths(src_dirs.to_vec())
    };
    if verify_commit_signatures {
        globals
            .verify_commit_signatures
            .store(true, std::sync::atomic::Ordering::Relaxed);
        // Load `:trusted-signers` from cljrs.edn so compile-time signature
        // verification has keys to check pinned commits against.
        if let Some(config) = std::env::current_dir()
            .ok()
            .and_then(|cwd| cljrs_deps::load_config(&cwd).ok().flatten())
        {
            globals.load_trusted_signers(&config);
        }
    }
    // Register clojure.core.async so that (require '[clojure.core.async ...])
    // and the `go`/`alt` macros resolve during macro-expansion. The GC
    // service is silently skipped when there is no LocalSet context.
    cljrs_async::init(&globals);
    // Register I/O, networking, charset, and base64 namespaces so that require
    // forms in source files resolve correctly during macro expansion.
    cljrs_io::init(&globals);
    cljrs_net::init(&globals);
    cljrs_charset::init(&globals);
    cljrs_base64::init(&globals);
    let mut env = cljrs_eval::Env::new(globals, "user");

    // Snapshot loaded namespaces before expansion so we can detect
    // which user namespaces were pulled in by require.
    let pre_loaded: std::collections::HashSet<Arc<str>> =
        env.globals.loaded.lock().unwrap().clone();

    let mut expanded = Vec::with_capacity(forms.len());
    for form in &forms {
        // For forms that need the interpreter (ns, require, defmacro, etc.),
        // evaluate them immediately so that required namespaces get loaded
        // and macros from dependencies are available for later forms.
        if needs_interpreter(form) {
            match cljrs_eval::eval(form, &mut env) {
                Ok(_) => {}
                Err(e) => return Err(AotError::Eval(format!("{e:?}"))),
            }
        }
        match cljrs_interp::macros::macroexpand_all(form, &mut env) {
            Ok(f) => expanded.push(f),
            Err(e) => return Err(AotError::Eval(format!("{e:?}"))),
        }
    }
    eprintln!("[aot] macro-expanded {} form(s)", expanded.len());

    // ── 2a. Pin versioned references ──────────────────────────────────
    // Versioned requires already executed (and were fetched) during
    // expansion.  This pass additionally catches bare versioned symbols
    // (`mylib/foo@<sha>`) anywhere in the program, force-loading each pin
    // now so the binary is self-contained: every pinned source is recorded
    // in `versioned_sources` for embedding, and a bad pin (missing commit,
    // failed signature) fails the compile instead of the deployed binary.
    pin_versioned_references(&expanded, &mut env)?;

    // Discover user namespaces loaded during expansion (transitive deps).
    // Each is AOT-compiled into its own initializer (below) rather than
    // bundled as source and tree-walked at startup.
    let discovered = discover_bundled_sources(&env.globals, &pre_loaded, src_dirs);

    // Pinned versioned sources (e.g. "mylib@abc1234") stay interpreted: they
    // resolve through the separate versioned loader — not the plain `require`
    // path — so they are bundled as builtin source as before.
    let mut versioned_bundled: Vec<(Arc<str>, String)> = Vec::new();
    for (ns, src) in env.globals.versioned_sources_snapshot() {
        versioned_bundled.push((ns, src.to_string()));
    }

    // Lower each discovered namespace to an interpreted preamble (ns/require,
    // macros) plus an optional natively compiled initializer.
    //
    // Graceful degradation: a namespace whose body cannot be lowered or
    // compiled (an unsupported construct, or a backend/verifier edge case) is
    // not a hard error — we fall back to bundling its source for interpretation
    // at startup, exactly as before required namespaces were compiled.  This
    // keeps the program buildable while compiling every namespace that can be.
    let mut compiled_namespaces: Vec<CompiledNamespace> = Vec::new();
    let mut dep_irs: Vec<(String, IrFunction)> = Vec::new();
    let mut interpreted_bundled: Vec<(Arc<str>, String)> = Vec::new();
    for (i, (ns, src)) in discovered.iter().enumerate() {
        let init_symbol = format!("__cljrs_ns_init_{i}");
        match lower_namespace(ns, src, &init_symbol, &env.globals) {
            Ok((preamble, Some(ir))) if dep_codegen_ok(&ir, &init_symbol) => {
                dep_irs.push((init_symbol.clone(), ir));
                compiled_namespaces.push(CompiledNamespace {
                    ns: ns.clone(),
                    init_symbol: Some(init_symbol),
                    preamble,
                });
            }
            // Lowered, but the body could not be compiled by the backend.
            Ok((_, Some(_))) => {
                eprintln!("[aot] {ns}: body could not be compiled, bundling as interpreted source");
                interpreted_bundled.push((ns.clone(), src.clone()));
            }
            // No compilable body (e.g. a namespace of only macros): a
            // preamble-only loader is enough.
            Ok((preamble, None)) => {
                compiled_namespaces.push(CompiledNamespace {
                    ns: ns.clone(),
                    init_symbol: None,
                    preamble,
                });
            }
            // Lowering failed outright (an unsupported form): interpret it.
            Err(e) => {
                eprintln!("[aot] {ns}: could not be lowered ({e}), bundling as interpreted source");
                interpreted_bundled.push((ns.clone(), src.clone()));
            }
        }
    }
    if !compiled_namespaces.is_empty() {
        eprintln!(
            "[aot] compiling {} required namespace(s): {}",
            compiled_namespaces.len(),
            compiled_namespaces
                .iter()
                .map(|c| c.ns.as_ref())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    // Namespaces that fell back to interpretation join the builtin-source
    // bundle, alongside pinned versioned sources.
    versioned_bundled.extend(interpreted_bundled);

    // ── 2b. Partition: interpreted preamble vs compiled body ─────────
    // Forms that define functions (defn, defmacro) or require interpreter
    // features (closures) are evaluated at startup via the interpreter.
    // The rest is AOT-compiled.
    let mut interpreted_source = String::new();
    let mut compilable = Vec::new();
    for (i, form) in expanded.iter().enumerate() {
        if needs_interpreter(&forms[i]) || expanded_needs_interpreter(form) {
            // Extract the original source text using span byte offsets.
            let span = &forms[i].span;
            let src_text = &source[span.start..span.end];
            interpreted_source.push_str(src_text);
            interpreted_source.push('\n');
        } else {
            let form = expand_anon_fns(form);
            compilable.push(qualify_aliases(&form, &env.current_ns, &env.globals));
        }
    }
    if !interpreted_source.is_empty() {
        eprintln!(
            "[aot] {} form(s) will be interpreted at startup",
            expanded.len() - compilable.len()
        );
    }

    // ── 3. ANF-lower ────────────────────────────────────────────────────
    // Treat compilable top-level forms as the body of a zero-arg `__cljrs_main`.
    let params: Vec<Arc<str>> = vec![];
    let compilable_forms = if compilable.is_empty() {
        // If everything is interpreted, emit a simple nil-returning main.
        let nil_form = cljrs_reader::Form::new(
            cljrs_reader::form::FormKind::Nil,
            cljrs_types::span::Span::new(Arc::new("<aot>".to_string()), 0, 0, 1, 1),
        );
        vec![nil_form]
    } else {
        compilable
    };

    // Use the current namespace (may have been changed by ns form in preamble).
    let current_ns = env.current_ns.to_string();
    let mut ir_func = lower_via_rust(
        Some("__cljrs_main"),
        &current_ns,
        &params,
        &compilable_forms,
        &mut env,
    )?;
    eprintln!(
        "[aot] lowered to {} block(s), {} var(s)",
        ir_func.blocks.len(),
        ir_func.next_var
    );

    // Region allocation stats — show how many heap allocs the optimizer
    // managed to lift onto the bump arena.
    let stats = count_alloc_stats(&ir_func);
    eprintln!(
        "[aot] allocation stats: {} region-allocated, {} heap, {} closures (across {} functions)",
        stats.region, stats.heap, stats.closures, stats.functions,
    );

    // ── 3b. Direct call optimization ────────────────────────────────────
    // Rewrite `LoadGlobal + Call` → `CallDirect` for functions defined in
    // the same compilation unit. This avoids going through rt_call.
    optimize_direct_calls(&mut ir_func);

    // ── 4. Cranelift codegen → .o ───────────────────────────────────────
    let mut compiler = Compiler::new()?;

    // Declare all subfunctions first (so they can reference each other).
    declare_subfunctions(&ir_func, &mut compiler)?;

    // Compile subfunctions before the main function.
    compile_subfunctions(&ir_func, &mut compiler)?;

    let func_id = compiler.declare_function("__cljrs_main", 0)?;
    compiler.compile_function(&ir_func, func_id)?;

    // Compile each required namespace's initializer into the same module.
    // Subfunction symbols are globally unique (per `fresh_global_name_id`),
    // so initializers can share one object module without name collisions.
    for (init_symbol, ns_ir) in &dep_irs {
        declare_subfunctions(ns_ir, &mut compiler)?;
        compile_subfunctions(ns_ir, &mut compiler)?;
        let id = compiler.declare_function(init_symbol, 0)?;
        compiler.compile_function(ns_ir, id)?;
    }

    // ── 4b. Async poll functions (Phase H) ──────────────────────────────
    // Regular `defn`s are compiled into `__cljrs_main`; `^:async` ones stay
    // interpreted (their body has `await`) and so were never evaluated into the
    // compile-time `env`.  Evaluate just their *definitions* now (the original
    // form, so `^:async` metadata survives; never a side-effecting top-level
    // form) so their bodies can be introspected and compiled to poll functions.
    for (i, form) in forms.iter().enumerate() {
        if is_def_form(form) && expanded_needs_interpreter(&expanded[i]) {
            let _ = cljrs_eval::eval(form, &mut env);
        }
    }
    // Compile a native state machine for each `^:async` fn the program
    // defined, into the same object module.  The harness registers them so
    // dispatch runs native instead of the `eval_async` tree-walker.
    let async_polls = compile_async_poll_fns(&mut compiler, &mut env)?;
    if !async_polls.is_empty() {
        eprintln!(
            "[aot] compiled {} async poll function(s)",
            async_polls.len()
        );
    }

    let obj_bytes = compiler.finish();
    eprintln!("[aot] generated {} bytes of object code", obj_bytes.len());

    // ── 5. Generate harness project & build ─────────────────────────────
    let (harness_dir, offline) = build_harness(
        out_path,
        &obj_bytes,
        &interpreted_source,
        &compiled_namespaces,
        &versioned_bundled,
        &async_polls,
        rust_config,
    )?;
    link_with_cargo(&harness_dir, out_path, offline)?;

    eprintln!("[aot] wrote {}", out_path.display());
    Ok(())
}

/// Check if a top-level form needs the interpreter (can't be AOT-compiled yet).
/// Whether `form` is a top-level `def`/`defn`/`defn-` (a binding definition,
/// safe to evaluate at compile time — no program side effects).
fn is_def_form(form: &cljrs_reader::Form) -> bool {
    use cljrs_reader::form::FormKind;
    if let FormKind::List(parts) = &form.kind
        && let Some(head) = parts.first()
        && let FormKind::Symbol(s) = &head.kind
    {
        return matches!(s.as_str(), "def" | "defn" | "defn-");
    }
    false
}

fn needs_interpreter(form: &cljrs_reader::Form) -> bool {
    use cljrs_reader::form::FormKind;
    match &form.kind {
        FormKind::List(parts) => {
            if let Some(head) = parts.first()
                && let FormKind::Symbol(s) = &head.kind
            {
                // defmacro/defonce need the interpreter (macros must be
                // available at compile time). ns/require are module-level.
                // Protocol/multimethod forms modify global dispatch tables
                // and are best handled by the interpreter at startup.
                return matches!(
                    s.as_str(),
                    "defmacro"
                        | "defonce"
                        | "ns"
                        | "require"
                        | "defprotocol"
                        | "extend-type"
                        | "extend-protocol"
                        | "defmulti"
                        | "defmethod"
                        | "defrecord"
                );
            }
            false
        }
        _ => false,
    }
}

/// Check if a symbol name (possibly namespace-qualified) refers to an
/// interpreter-only function.
fn is_interpreter_only_sym(s: &str) -> bool {
    // Strip namespace prefix if present (e.g. "clojure.core/alter-meta!" → "alter-meta!")
    let base = s.rsplit('/').next().unwrap_or(s);
    matches!(
        base,
        "alter-meta!" | "vary-meta" | "reset-meta!" | "with-meta"
    )
}

/// Check the macro-expanded form for constructs that the AOT compiler
/// cannot handle (e.g. alter-meta!, vary-meta, await). This recurses
/// into the form tree so that e.g. `(do (def x ...) (alter-meta! ...))` is caught.
/// `await` and `async-spawn` are async special forms only the interpreter understands;
/// any top-level form whose expansion tree contains them must stay interpreted.
fn expanded_needs_interpreter(form: &cljrs_reader::Form) -> bool {
    use cljrs_reader::form::FormKind;
    match &form.kind {
        FormKind::List(parts) => {
            if let Some(head) = parts.first()
                && let FormKind::Symbol(s) = &head.kind
            {
                let base = s.rsplit('/').next().unwrap_or(s.as_str());
                if is_interpreter_only_sym(s.as_str()) || base == "await" {
                    return true;
                }
            }
            parts.iter().any(expanded_needs_interpreter)
        }
        FormKind::Vector(elems) | FormKind::Set(elems) => {
            elems.iter().any(expanded_needs_interpreter)
        }
        FormKind::Map(elems) => elems.iter().any(expanded_needs_interpreter),
        _ => false,
    }
}

// ── Subfunction compilation ─────────────────────────────────────────────────

/// Recursively declare all subfunctions in the compiler module.
fn declare_subfunctions(ir_func: &IrFunction, compiler: &mut Compiler) -> AotResult<()> {
    for sub in &ir_func.subfunctions {
        let name = sub.name.as_deref().unwrap_or("__cljrs_anon");
        // `abi_param_count` adds the hidden trailing region parameter for
        // region-parameterised (`__rg`) variants.
        compiler.declare_function(name, sub.abi_param_count())?;
        declare_subfunctions(sub, compiler)?;
    }
    Ok(())
}

/// Recursively compile all subfunctions.
fn compile_subfunctions(ir_func: &IrFunction, compiler: &mut Compiler) -> AotResult<()> {
    for sub in &ir_func.subfunctions {
        compile_subfunctions(sub, compiler)?;
        let name = sub.name.as_deref().unwrap_or("__cljrs_anon");
        let func_id = compiler.declare_function(name, sub.abi_param_count())?;
        compiler.compile_function(sub, func_id)?;
    }
    Ok(())
}

// ── Versioned pin discovery ──────────────────────────────────────────────────

/// Walk the macro-expanded program for versioned symbols (`name@<sha>`,
/// `ns/name@<sha>`) and force-load each pin at compile time.
///
/// Loading records the pinned source in `GlobalEnv::versioned_sources` for
/// embedding.  Pins whose namespace has no locatable Clojure source are
/// skipped (pure-Rust packages resolve through the native HEAD fallback at
/// runtime; quoted symbols that merely look versioned stay inert).  Genuine
/// load failures abort the compile.
fn pin_versioned_references(
    forms: &[cljrs_reader::Form],
    env: &mut cljrs_eval::Env,
) -> AotResult<()> {
    let mut pins: Vec<(Option<String>, String)> = Vec::new();
    for form in forms {
        collect_versioned_syms(form, &mut pins);
    }
    pins.sort();
    pins.dedup();

    for (ns_part, commit) in pins {
        let base: Arc<str> = match &ns_part {
            Some(p) => {
                let resolved = env
                    .globals
                    .resolve_alias(&env.current_ns, p)
                    .unwrap_or_else(|| Arc::from(p.as_str()));
                Arc::from(cljrs_env::versioned::base_ns_name(&resolved))
            }
            None => Arc::from(cljrs_env::versioned::base_ns_name(&env.current_ns)),
        };
        match cljrs_env::versioned::pin_if_available(&env.globals, &base, &commit) {
            Ok(true) => eprintln!("[aot] pinned {base}@{commit}"),
            Ok(false) => {}
            Err(e) => {
                return Err(AotError::Eval(format!(
                    "failed to pin {base}@{commit}: {e:?}"
                )));
            }
        }
    }
    Ok(())
}

/// Collect `(namespace-part, commit)` for every versioned symbol in the form
/// tree (including quoted/metadata-wrapped positions).
fn collect_versioned_syms(form: &cljrs_reader::Form, out: &mut Vec<(Option<String>, String)>) {
    use cljrs_reader::form::FormKind;
    match &form.kind {
        FormKind::Symbol(s) => {
            let sym = cljrs_value::Symbol::parse(s);
            if let Some(v) = sym.version {
                out.push((
                    sym.namespace.as_deref().map(str::to_string),
                    v.as_ref().to_string(),
                ));
            }
        }
        FormKind::List(items)
        | FormKind::Vector(items)
        | FormKind::Map(items)
        | FormKind::Set(items)
        | FormKind::AnonFn(items) => {
            for item in items {
                collect_versioned_syms(item, out);
            }
        }
        FormKind::Quote(inner)
        | FormKind::SyntaxQuote(inner)
        | FormKind::Unquote(inner)
        | FormKind::UnquoteSplice(inner)
        | FormKind::Deref(inner)
        | FormKind::Var(inner)
        | FormKind::TaggedLiteral(_, inner) => collect_versioned_syms(inner, out),
        FormKind::Meta(meta, inner) => {
            collect_versioned_syms(meta, out);
            collect_versioned_syms(inner, out);
        }
        FormKind::ReaderCond { clauses, .. } => {
            for clause in clauses {
                collect_versioned_syms(clause, out);
            }
        }
        _ => {}
    }
}

// ── Bundled source discovery ─────────────────────────────────────────────────

/// Discover user namespaces loaded during macro expansion that need to be
/// bundled into the compiled binary.
///
/// Compares the set of loaded namespaces before and after expansion. For each
/// newly loaded namespace that isn't a builtin source, resolves and reads
/// its source file from `src_dirs`.
fn discover_bundled_sources(
    globals: &Arc<cljrs_env::env::GlobalEnv>,
    pre_loaded: &std::collections::HashSet<Arc<str>>,
    src_dirs: &[PathBuf],
) -> Vec<(Arc<str>, String)> {
    let post_loaded = globals.loaded.lock().unwrap().clone();
    let mut bundled = Vec::new();

    for ns in post_loaded.difference(pre_loaded) {
        // Skip namespaces that are already available as builtins at runtime.
        if globals.builtin_source(ns).is_some() {
            continue;
        }
        // Resolve the source file from src_dirs.
        let rel_path = ns.replace('.', "/").replace('-', "_");
        if let Some(src) = find_user_source(&rel_path, src_dirs) {
            bundled.push((ns.clone(), src));
        }
    }

    bundled
}

/// Find and read a user source file from the given directories.
fn find_user_source(rel: &str, src_dirs: &[PathBuf]) -> Option<String> {
    for dir in src_dirs {
        for ext in &[".cljrs", ".cljc"] {
            let path = dir.join(format!("{rel}{ext}"));
            if path.exists() {
                return std::fs::read_to_string(&path).ok();
            }
        }
    }
    None
}

/// One required namespace compiled into the AOT object module: its interpreted
/// preamble (the `ns`/`require` form and any `defmacro`/protocol/multimethod
/// definitions that must run through the interpreter) and the symbol of its
/// natively compiled initializer (`None` when the namespace has no compilable
/// top-level forms — e.g. a namespace consisting only of `defmacro`s).
struct CompiledNamespace {
    ns: Arc<str>,
    init_symbol: Option<String>,
    preamble: String,
}

/// Recursively resolve reader conditionals (`#?(...)` / `#?@(...)`) to their
/// `:rust` (or `:default`) branch throughout a form tree.
///
/// `macroexpand_all` leaves reader conditionals as `ReaderCond` nodes — the
/// tree-walking interpreter resolves them at eval time — but the AOT lowerer
/// rejects an un-expanded `ReaderCond`.  This runs over the whole program before
/// macro-expansion (a selected branch may itself contain macros), mirroring the
/// reader→macro pipeline.  Splicing `#?@(...)` is inlined into its enclosing
/// sequence via `expand_reader_conds`; non-matching conditionals are dropped
/// (or become `nil` in a value position, matching the interpreter).
fn expand_reader_conds_deep(forms: &[cljrs_reader::Form]) -> Vec<cljrs_reader::Form> {
    // Resolve conditionals at this level first (this is what inlines `#?@`),
    // then recurse into each resulting form's children.
    cljrs_builtins::form::expand_reader_conds(forms)
        .iter()
        .map(expand_reader_cond_form)
        .collect()
}

fn expand_reader_cond_form(form: &cljrs_reader::Form) -> cljrs_reader::Form {
    use cljrs_reader::form::FormKind;
    let kind = match &form.kind {
        FormKind::List(v) => FormKind::List(expand_reader_conds_deep(v)),
        FormKind::Vector(v) => FormKind::Vector(expand_reader_conds_deep(v)),
        FormKind::Set(v) => FormKind::Set(expand_reader_conds_deep(v)),
        FormKind::Map(v) => FormKind::Map(expand_reader_conds_deep(v)),
        FormKind::AnonFn(v) => FormKind::AnonFn(expand_reader_conds_deep(v)),
        FormKind::Quote(f) => FormKind::Quote(Box::new(expand_reader_cond_form(f))),
        FormKind::SyntaxQuote(f) => FormKind::SyntaxQuote(Box::new(expand_reader_cond_form(f))),
        FormKind::Unquote(f) => FormKind::Unquote(Box::new(expand_reader_cond_form(f))),
        FormKind::UnquoteSplice(f) => FormKind::UnquoteSplice(Box::new(expand_reader_cond_form(f))),
        FormKind::Deref(f) => FormKind::Deref(Box::new(expand_reader_cond_form(f))),
        FormKind::Var(f) => FormKind::Var(Box::new(expand_reader_cond_form(f))),
        FormKind::Meta(m, f) => FormKind::Meta(
            Box::new(expand_reader_cond_form(m)),
            Box::new(expand_reader_cond_form(f)),
        ),
        FormKind::TaggedLiteral(t, f) => {
            FormKind::TaggedLiteral(t.clone(), Box::new(expand_reader_cond_form(f)))
        }
        // A conditional reached outside a sequence (e.g. wrapped in `quote`):
        // resolve it directly; an unconsumed splice or no-match becomes `nil`.
        FormKind::ReaderCond {
            splicing: false,
            clauses,
        } => match cljrs_builtins::form::select_reader_cond(clauses) {
            Some(sel) => return expand_reader_cond_form(sel),
            None => FormKind::Nil,
        },
        FormKind::ReaderCond { splicing: true, .. } => FormKind::Nil,
        _ => return form.clone(),
    };
    cljrs_reader::Form::new(kind, form.span.clone())
}

/// Recursively expand anonymous-function reader macros (`#(...)`) into `(fn*
/// [...] ...)` forms.
///
/// `macroexpand_all` deliberately leaves `#(...)` as an `AnonFn` node — the
/// tree-walking interpreter expands it lazily at eval time — but the AOT lowerer
/// has no such fallback and rejects an un-expanded `AnonFn`.  This pass runs over
/// the compilable forms before lowering so any `#(...)` (in the entry namespace
/// or in a compiled required namespace such as `clojure.tools.cli`) is turned
/// into a lowerable `fn*`.
fn expand_anon_fns(form: &cljrs_reader::Form) -> cljrs_reader::Form {
    use cljrs_reader::form::FormKind;
    let map_vec = |v: &[cljrs_reader::Form]| v.iter().map(expand_anon_fns).collect::<Vec<_>>();
    let kind = match &form.kind {
        FormKind::AnonFn(body) => {
            // Expand this `#(...)`, then recurse so nested forms are handled too.
            let expanded = cljrs_builtins::form::expand_anon_fn(body, form.span.clone());
            return expand_anon_fns(&expanded);
        }
        FormKind::List(v) => FormKind::List(map_vec(v)),
        FormKind::Vector(v) => FormKind::Vector(map_vec(v)),
        FormKind::Map(v) => FormKind::Map(map_vec(v)),
        FormKind::Set(v) => FormKind::Set(map_vec(v)),
        FormKind::Quote(f) => FormKind::Quote(Box::new(expand_anon_fns(f))),
        FormKind::SyntaxQuote(f) => FormKind::SyntaxQuote(Box::new(expand_anon_fns(f))),
        FormKind::Unquote(f) => FormKind::Unquote(Box::new(expand_anon_fns(f))),
        FormKind::UnquoteSplice(f) => FormKind::UnquoteSplice(Box::new(expand_anon_fns(f))),
        FormKind::Deref(f) => FormKind::Deref(Box::new(expand_anon_fns(f))),
        FormKind::Var(f) => FormKind::Var(Box::new(expand_anon_fns(f))),
        FormKind::Meta(m, f) => {
            FormKind::Meta(Box::new(expand_anon_fns(m)), Box::new(expand_anon_fns(f)))
        }
        FormKind::TaggedLiteral(t, f) => {
            FormKind::TaggedLiteral(t.clone(), Box::new(expand_anon_fns(f)))
        }
        _ => return form.clone(),
    };
    cljrs_reader::Form::new(kind, form.span.clone())
}

/// Rewrite namespace-alias-qualified symbols (`alias/name`) to their fully
/// qualified form (`real-ns/name`) using `ns`'s alias table.
///
/// Compiled `LoadGlobal` bakes in the namespace part of a symbol at lower time
/// and resolves it at runtime; for a plain alias (`u/wrap`) that resolution
/// otherwise depends on the namespace *active when the compiled function runs*,
/// which is the caller's — not the function's defining namespace.  Resolving the
/// alias here makes the symbol carry its real namespace (`utils/wrap`), so it
/// resolves correctly no matter where the compiled function is invoked from.
///
/// Only var references are rewritten: in Clojure bare symbols inside collection
/// literals are evaluated, so they are rewritten too, but `quote`d data is left
/// untouched.
fn qualify_aliases(
    form: &cljrs_reader::Form,
    ns: &str,
    globals: &Arc<cljrs_env::env::GlobalEnv>,
) -> cljrs_reader::Form {
    use cljrs_reader::form::FormKind;
    let recur = |f: &cljrs_reader::Form| qualify_aliases(f, ns, globals);
    let map_vec = |v: &[cljrs_reader::Form]| v.iter().map(&recur).collect::<Vec<_>>();
    let kind = match &form.kind {
        FormKind::Symbol(s) => match s.find('/') {
            Some(slash) if s != "/" => {
                let (alias, rest) = (&s[..slash], &s[slash + 1..]);
                if !alias.is_empty()
                    && !rest.is_empty()
                    && let Some(real) = globals.resolve_alias(ns, alias)
                    && real.as_ref() != alias
                {
                    FormKind::Symbol(format!("{real}/{rest}"))
                } else {
                    return form.clone();
                }
            }
            _ => return form.clone(),
        },
        FormKind::List(v) => FormKind::List(map_vec(v)),
        FormKind::Vector(v) => FormKind::Vector(map_vec(v)),
        FormKind::Map(v) => FormKind::Map(map_vec(v)),
        FormKind::Set(v) => FormKind::Set(map_vec(v)),
        FormKind::AnonFn(v) => FormKind::AnonFn(map_vec(v)),
        FormKind::SyntaxQuote(f) => FormKind::SyntaxQuote(Box::new(recur(f))),
        FormKind::Unquote(f) => FormKind::Unquote(Box::new(recur(f))),
        FormKind::UnquoteSplice(f) => FormKind::UnquoteSplice(Box::new(recur(f))),
        FormKind::Deref(f) => FormKind::Deref(Box::new(recur(f))),
        FormKind::Var(f) => FormKind::Var(Box::new(recur(f))),
        FormKind::Meta(m, f) => FormKind::Meta(Box::new(recur(m)), Box::new(recur(f))),
        FormKind::TaggedLiteral(t, f) => FormKind::TaggedLiteral(t.clone(), Box::new(recur(f))),
        // `quote`d data and scalars (and reader conditionals, already resolved
        // for this platform at parse time) are left untouched.
        _ => return form.clone(),
    };
    cljrs_reader::Form::new(kind, form.span.clone())
}

/// Trial-compile a required namespace's initializer (and its subfunctions) in a
/// throwaway object module to check the backend can actually compile it.
///
/// Lowering can succeed yet still produce IR the Cranelift backend rejects
/// (e.g. a region-specialization edge case that fails verification).  Probing
/// here lets `compile_file` fall back to interpreting such a namespace instead
/// of aborting the whole build.  The probe is discarded; the namespace is
/// compiled again into the real shared module only if this returns `true`.
fn dep_codegen_ok(ir: &IrFunction, init_symbol: &str) -> bool {
    let Ok(mut trial) = Compiler::new() else {
        return false;
    };
    let result = (|| -> AotResult<()> {
        declare_subfunctions(ir, &mut trial)?;
        compile_subfunctions(ir, &mut trial)?;
        let id = trial.declare_function(init_symbol, 0)?;
        trial.compile_function(ir, id)?;
        Ok(())
    })();
    result.is_ok()
}

/// Lower one required namespace to an interpreted preamble plus an optional
/// natively compiled initializer.
///
/// `source` is the namespace's Clojure source.  The namespace has already been
/// loaded into `globals` during the entry file's macro-expansion, so its own
/// macros and requires are available; here we only macro-expand (never
/// re-evaluate) in order to partition top-level forms into an interpreted
/// preamble (ns/require, defmacro, protocols, …) and a compilable body which is
/// lowered to an `__cljrs_ns_init_*` IR function.  Returns `(preamble, ir)`
/// where `ir` is `None` if the namespace has no compilable top-level forms.
fn lower_namespace(
    ns_name: &str,
    source: &str,
    init_symbol: &str,
    globals: &Arc<cljrs_env::env::GlobalEnv>,
) -> AotResult<(String, Option<IrFunction>)> {
    let mut parser = Parser::new(source.to_string(), format!("<{ns_name}>"));
    let forms = parser.parse_all()?;

    // Resolve reader conditionals to the `:rust` branch before macro-expansion.
    let forms = expand_reader_conds_deep(&forms);

    // Macro-expand each form in an env rooted at this namespace so symbol and
    // alias resolution match how the namespace's own code sees the world.
    let mut env = cljrs_eval::Env::new(globals.clone(), ns_name);
    let mut expanded = Vec::with_capacity(forms.len());
    for form in &forms {
        match cljrs_interp::macros::macroexpand_all(form, &mut env) {
            Ok(f) => expanded.push(f),
            Err(e) => return Err(AotError::Eval(format!("{e:?}"))),
        }
    }

    // Partition: interpreted preamble vs compiled body (same rule as the entry
    // file in `compile_file`).
    let mut preamble = String::new();
    let mut compilable = Vec::new();
    for (i, form) in expanded.iter().enumerate() {
        if needs_interpreter(&forms[i]) || expanded_needs_interpreter(form) {
            let span = &forms[i].span;
            preamble.push_str(&source[span.start..span.end]);
            preamble.push('\n');
        } else {
            let form = expand_anon_fns(form);
            compilable.push(qualify_aliases(&form, ns_name, globals));
        }
    }

    if compilable.is_empty() {
        return Ok((preamble, None));
    }

    let mut ir = lower_via_rust(Some(init_symbol), ns_name, &[], &compilable, &mut env)?;
    optimize_direct_calls(&mut ir);
    Ok((preamble, Some(ir)))
}

// ── Harness generation ──────────────────────────────────────────────────────

/// Create a temporary Cargo project that links the compiled object code with
/// the clojurust runtime and produces a binary.
/// A `^:async` arity compiled to a native poll function, to be registered by
/// the generated harness via `cljrs_async::state_machine::register_poll_fn_named`.
struct AsyncPollEntry {
    ns: String,
    name: String,
    arity: usize,
    symbol: String,
    n_slots: usize,
}

/// Compile a native poll function for each `^:async` fn the program defined
/// (introspected from `env`, which already evaluated every `defn` at compile
/// time), into the AOT object module.  Functions whose body uses an unsupported
/// construct (channels, spawn, `throw`, regions) are skipped — they keep the
/// `eval_async` tree-walker at runtime.  Capturing closures and fns with inner
/// closures are also skipped (standalone lowering can't see captures, and inner
/// closures would need separate subfunction-symbol management).
fn compile_async_poll_fns(
    compiler: &mut Compiler,
    env: &mut cljrs_eval::Env,
) -> AotResult<Vec<AsyncPollEntry>> {
    // Snapshot the async fns first so no namespace lock is held across lowering
    // (which re-enters the env).
    struct AritySnap {
        params: Vec<Arc<str>>,
        rest: Option<Arc<str>>,
        dparams: Vec<(usize, cljrs_reader::Form)>,
        drest: Option<cljrs_reader::Form>,
        body: Vec<cljrs_reader::Form>,
    }
    struct FnSnap {
        ns: Arc<str>,
        name: Arc<str>,
        arities: Vec<AritySnap>,
    }

    let snaps: Vec<FnSnap> = {
        let ns_map = env.globals.namespaces.read().unwrap();
        let mut out = Vec::new();
        for (ns_name, ns_ptr) in ns_map.iter() {
            // User namespaces only; the stdlib defines no `^:async` fns and
            // scanning it would just waste work.
            if ns_name.starts_with("clojure.") {
                continue;
            }
            let interns = ns_ptr.get().interns.lock().unwrap();
            for (vname, var) in interns.iter() {
                let Some(cljrs_value::Value::Fn(f)) = var.get().deref() else {
                    continue;
                };
                let fr = f.get();
                if !fr.is_async || !fr.closed_over_names.is_empty() {
                    continue;
                }
                let arities = fr
                    .arities
                    .iter()
                    .map(|a| AritySnap {
                        params: a.params.clone(),
                        rest: a.rest_param.clone(),
                        dparams: a.destructure_params.clone(),
                        drest: a.destructure_rest.clone(),
                        body: a.body.clone(),
                    })
                    .collect();
                out.push(FnSnap {
                    ns: fr.defining_ns.clone(),
                    name: fr.name.clone().unwrap_or_else(|| vname.clone()),
                    arities,
                });
            }
        }
        out
    };

    let mut entries = Vec::new();
    for fnsnap in &snaps {
        for arity in &fnsnap.arities {
            // Channel / concurrency ops (Phase H4) aren't modeled by the poll
            // machine; keep such fns on the tree-walker.
            if cljrs_ir::lower::async_lower::body_uses_unsupported_async(&arity.body) {
                continue;
            }
            let ir = match cljrs_eval::lower::lower_arity(
                Some(&fnsnap.name),
                &arity.params,
                arity.rest.as_ref(),
                &arity.dparams,
                arity.drest.as_ref(),
                &arity.body,
                &fnsnap.ns,
                env,
                true,
            ) {
                Ok(ir) => ir,
                Err(_) => continue,
            };
            // Inner closures would need separate subfunction symbols; defer.
            if !ir.subfunctions.is_empty() {
                continue;
            }
            let low = match cljrs_ir::lower::lower_async(&ir) {
                Ok(low) => low,
                Err(_) => continue,
            };
            let symbol = format!("__cljrs_async_poll_{}", entries.len());
            let func_id = compiler.declare_poll_function(&symbol)?;
            compiler.compile_function(&low.poll_fn, func_id)?;
            entries.push(AsyncPollEntry {
                ns: fnsnap.ns.to_string(),
                name: fnsnap.name.to_string(),
                arity: arity.params.len(),
                symbol,
                n_slots: low.n_slots,
            });
        }
    }
    Ok(entries)
}

fn build_harness(
    out_path: &Path,
    obj_bytes: &[u8],
    interpreted_source: &str,
    compiled_namespaces: &[CompiledNamespace],
    versioned_bundled: &[(Arc<str>, String)],
    async_polls: &[AsyncPollEntry],
    rust_config: Option<&cljrs_deps::RustConfig>,
) -> AotResult<(PathBuf, bool)> {
    // Place the harness in a temp dir next to the output.
    let harness_dir = out_path
        .parent()
        .unwrap_or(Path::new("."))
        .join(".cljrs-aot-harness");

    // Clean any previous harness.
    if harness_dir.exists() {
        std::fs::remove_dir_all(&harness_dir)?;
    }
    std::fs::create_dir_all(harness_dir.join("src"))?;

    // Write the object file.
    let obj_path = harness_dir.join("__cljrs_main.o");
    std::fs::write(&obj_path, obj_bytes)?;

    // Resolve how to depend on the runtime crates: a local checkout (path
    // deps) when one is found, otherwise the published versions (so an
    // installed `cargo install cljrs` binary can still compile).
    let deps = resolve_harness_deps()?;

    // Write Cargo.toml.
    // The empty [workspace] table prevents Cargo from thinking this is
    // part of a parent workspace.
    let mut native_deps = String::new();
    if let Some(rc) = rust_config {
        native_deps.push_str(&deps.dep_line("cljrs-interop"));
        if let Some(crate_name) = rc.crate_name() {
            let crate_dir = rc.crate_dir.display();
            native_deps.push_str(&format!("{crate_name} = {{ path = \"{crate_dir}\" }}\n"));
        }
    }
    let runtime_deps: String = HARNESS_RUNTIME_CRATES
        .iter()
        .map(|c| deps.dep_line(c))
        .collect();
    let cargo_toml = format!(
        r#"[package]
name = "cljrs-aot-harness"
version = "0.1.0"
edition = "2024"

[workspace]

[dependencies]
{runtime_deps}tokio = {{ version = "1", features = ["rt", "time", "net", "io-util"] }}
{native_deps}"#,
    );
    std::fs::write(harness_dir.join("Cargo.toml"), cargo_toml)?;

    // Write build.rs — tells Cargo to link our object file.
    let obj_abs = std::fs::canonicalize(&obj_path)?;
    let build_rs = format!(
        r#"fn main() {{
    // Link the AOT-compiled object file.
    println!("cargo:rustc-link-arg={obj}");
    println!("cargo:rerun-if-changed={obj}");
}}"#,
        obj = obj_abs.display()
    );
    std::fs::write(harness_dir.join("build.rs"), build_rs)?;

    // Write the interpreted preamble source (if any).
    let has_preamble = !interpreted_source.is_empty();
    if has_preamble {
        std::fs::write(harness_dir.join("src/preamble.cljrs"), interpreted_source)?;
    }

    // Write pinned versioned dependency sources (still interpreted at startup —
    // they resolve through the versioned loader, not the plain require path).
    let mut bundled_registration = String::new();
    for (i, (ns, src)) in versioned_bundled.iter().enumerate() {
        let filename = format!("versioned_{i}.cljrs");
        std::fs::write(harness_dir.join("src").join(&filename), src)?;
        eprintln!("[aot] bundled (interpreted) {ns} → src/{filename}");
        bundled_registration.push_str(&format!(
            "    globals.register_builtin_source({ns:?}, include_str!({filename:?}));\n"
        ));
    }

    // Write the interpreted preamble for each AOT-compiled namespace, and build
    // the extern declarations + loader registrations that wire each required
    // namespace's compiled initializer into `require`.
    let mut ns_init_externs = String::new();
    let mut compiled_ns_registration = String::new();
    for (i, cns) in compiled_namespaces.iter().enumerate() {
        let preamble_file = format!("ns_{i}_preamble.cljrs");
        std::fs::write(harness_dir.join("src").join(&preamble_file), &cns.preamble)?;
        let ns = cns.ns.as_ref();
        let ns_label = format!("<{ns}>");
        eprintln!(
            "[aot] compiled namespace {ns} → src/{preamble_file} + {}",
            cns.init_symbol.as_deref().unwrap_or("(preamble only)")
        );

        let init_call = match &cns.init_symbol {
            Some(sym) => {
                ns_init_externs.push_str(&format!("    fn {sym}() -> *const Value;\n"));
                format!("        unsafe {{ {sym}(); }}\n")
            }
            None => String::new(),
        };

        compiled_ns_registration.push_str(&format!(
            r#"    globals.register_compiled_ns_loader({ns:?}, std::sync::Arc::new(
        |globals: &std::sync::Arc<cljrs_env::env::GlobalEnv>| -> cljrs_env::error::EvalResult<()> {{
        let mut env = cljrs_eval::Env::new(globals.clone(), {ns:?});
        cljrs_env::callback::push_eval_context(&env);
        let preamble = include_str!({preamble_file:?});
        if !preamble.is_empty() {{
            let mut parser = cljrs_reader::Parser::new(preamble.to_string(), {ns_label:?}.to_string());
            let forms = parser.parse_all().map_err(cljrs_env::error::EvalError::Read)?;
            for form in &forms {{
                cljrs_eval::eval(form, &mut env)?;
            }}
        }}
        // Re-push the eval context with the (possibly updated) namespace before
        // running the compiled initializer.
        cljrs_env::callback::pop_eval_context();
        cljrs_env::callback::push_eval_context(&env);
{init_call}        cljrs_env::callback::pop_eval_context();
        Ok(())
    }}));
"#,
        ));
    }

    // Write main.rs — calls into the compiled __cljrs_main.

    // Generated Rust block that calls -main after the compiled body runs.
    // Uses resolve to check existence, then builds and evals the call expression.
    let main_call_code = r#"
    // Call -main if defined, forwarding command-line arguments (skip program name).
    {
        let __argv: Vec<String> = std::env::args().skip(1).collect();
        let __check = cljrs_reader::Parser::new(
            "(resolve '-main)".to_string(),
            "<main-check>".to_string(),
        )
        .parse_all()
        .ok()
        .and_then(|fs| cljrs_eval::eval(&fs[0], &mut env).ok());
        if let Some(r) = __check {
            if r != cljrs_value::Value::Nil {
                let escaped: Vec<String> = __argv
                    .iter()
                    .map(|a| {
                        let mut s = String::with_capacity(a.len() + 2);
                        s.push('"');
                        for ch in a.chars() {
                            match ch {
                                '"' => s.push_str("\\\""),
                                '\\' => s.push_str("\\\\"),
                                '\n' => s.push_str("\\n"),
                                '\r' => s.push_str("\\r"),
                                '\t' => s.push_str("\\t"),
                                c => s.push(c),
                            }
                        }
                        s.push('"');
                        s
                    })
                    .collect();
                let call = format!("(-main {})", escaped.join(" "));
                if let Ok(fs) = cljrs_reader::Parser::new(call, "<main>".to_string()).parse_all() {
                    match cljrs_eval::eval(&fs[0], &mut env) {
                        Ok(main_result) => {
                            // If -main is ^:async it returns a Future; await it on the
                            // current LocalSet so all spawned go-blocks drain to completion.
                            if let Err(e) = cljrs_async::eval_async::await_value(main_result).await {
                                eprintln!("cljrs: error in -main: {e:?}");
                                std::process::exit(1);
                            }
                        }
                        Err(e) => {
                            eprintln!("cljrs: error in -main: {e:?}");
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
    }
"#;

    let preamble_code = if has_preamble {
        r#"
    // Evaluate interpreted preamble (ns, require, defn, defmacro, etc.).
    let preamble = include_str!("preamble.cljrs");
    let mut parser = cljrs_reader::Parser::new(preamble.to_string(), "<preamble>".to_string());
    let forms = parser.parse_all().expect("preamble parse error");
    for form in &forms {
        cljrs_eval::eval(form, &mut env).expect("preamble eval error");
    }
    // Re-push eval context with updated namespace (ns form may have changed it).
    cljrs_env::callback::pop_eval_context();
    cljrs_env::callback::push_eval_context(&env);
"#
    } else {
        ""
    };

    // Emit the native init call when :rust :init is configured.  The init
    // function has the signature `fn cljrs_init(registry: &mut Registry)`
    // and is called before the preamble so native functions are visible to
    // macro-expanded code at startup.
    let native_init_code = match rust_config.and_then(|rc| rc.init_fn.as_deref()) {
        Some(init_fn) => format!(
            "\n    // Register native Rust functions via the user crate's init hook.\n    \
             let mut __registry = cljrs_interop::Registry::new(globals.clone());\n    \
             {init_fn}(&mut __registry);\n"
        ),
        None => String::new(),
    };

    // Register AOT-compiled async poll functions: declare each as an external
    // symbol (defined in the linked object), then register it by ns/name/arity
    // so `^:async` dispatch runs native instead of the tree-walker.
    let async_poll_registration = if async_polls.is_empty() {
        String::new()
    } else {
        let mut s = String::from(
            "\n    // Register AOT-compiled async poll functions.\n    unsafe extern \"C\" {\n",
        );
        for e in async_polls {
            s.push_str(&format!(
                "        fn {sym}(sm: *mut cljrs_async::state_machine::CljxStateMachine) -> i32;\n",
                sym = e.symbol,
            ));
        }
        s.push_str("    }\n    unsafe {\n");
        for e in async_polls {
            s.push_str(&format!(
                "        cljrs_async::state_machine::register_poll_fn_named({ns:?}, {name:?}, {arity}, \
                 std::mem::transmute::<unsafe extern \"C\" fn(*mut cljrs_async::state_machine::CljxStateMachine) -> i32, \
                 cljrs_async::state_machine::PollFn>({sym}), {nslots});\n",
                ns = e.ns,
                name = e.name,
                arity = e.arity,
                sym = e.symbol,
                nslots = e.n_slots,
            ));
        }
        s.push_str("    }\n");
        s
    };

    let main_rs = format!(
        r#"//! Auto-generated AOT harness for clojurust.
//!
//! Initializes the runtime, then calls the compiled `__cljrs_main`.

#![allow(improper_ctypes)]

use cljrs_value::Value;

unsafe extern "C" {{
    fn __cljrs_main() -> *const Value;
{ns_init_externs}}}

async fn run() {{
    // Parse environment -X flags.
    cljrs_logging::set_feature_levels_from_env().unwrap();

    // Ensure all rt_* symbols are linked into the binary.
    cljrs_compiler::rt_abi::anchor_rt_symbols();

    // Initialize the standard environment so that rt_call and other
    // runtime bridge functions can look up builtins.
    let globals = cljrs_stdlib::standard_env();

    // Versioned namespaces resolve only from sources embedded at compile
    // time — an AOT binary never fetches from git at runtime.
    globals.set_versioned_offline(true);

    // Register the async runtime (clojure.core.async, ^:async dispatch, await).
    cljrs_async::init(&globals);
{async_polls}
    // Register I/O, networking, charset, and base64 namespaces.
    cljrs_io::init(&globals);
    cljrs_net::init(&globals);
    cljrs_charset::init(&globals);
    cljrs_base64::init(&globals);

    // Register bundled dependency sources so require can find them
    // without needing source files on disk.
{bundled}
    // Register AOT-compiled namespace loaders so `require` runs their native
    // initializers instead of interpreting source.
{compiled_ns}{native_init}
    let mut env = cljrs_eval::Env::new(globals, "user");

    // Push an eval context so rt_call can dispatch through the interpreter.
    cljrs_env::callback::push_eval_context(&env);
{preamble}
    // Call the compiled code.
    let _result = unsafe {{ __cljrs_main() }};
{main_call}
    // Pop the eval context.
    cljrs_env::callback::pop_eval_context();

    // If CLJRS_GC_STATS is set, dump GC stats to its target (stdout/file).
    cljrs_gc::dump_stats_from_env();
}}

fn main() {{
    // Run on a large-stack thread to avoid stack overflows in deeply recursive
    // Clojure code (macros, lazy sequences, recursive interpreting).
    const STACK_SIZE: usize = 64 * 1024 * 1024; // 64 MiB
    let thread = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(|| {{
            // Drive all async tasks (go blocks, ^:async fns, channels) on a
            // single-threaded Tokio LocalSet so GcPtr<!Send> values stay on
            // one OS thread throughout execution.
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build Tokio runtime");
            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, run());
        }})
        .expect("failed to spawn main thread");
    if let Err(e) = thread.join() {{
        std::panic::resume_unwind(e);
    }}
}}
"#,
        preamble = preamble_code,
        bundled = bundled_registration,
        compiled_ns = compiled_ns_registration,
        ns_init_externs = ns_init_externs,
        native_init = native_init_code,
        main_call = main_call_code,
        async_polls = async_poll_registration,
    );
    std::fs::write(harness_dir.join("src/main.rs"), main_rs)?;

    Ok((harness_dir, deps.offline()))
}

/// Runtime crates the AOT harness `main()` links against, in dependency order.
const HARNESS_RUNTIME_CRATES: &[&str] = &[
    "cljrs-logging",
    "cljrs-types",
    "cljrs-gc",
    "cljrs-value",
    "cljrs-reader",
    "cljrs-env",
    "cljrs-eval",
    "cljrs-stdlib",
    "cljrs-compiler",
    "cljrs-async",
    "cljrs-io",
    "cljrs-net",
    "cljrs-charset",
    "cljrs-base64",
];

/// Runtime crates the AOT *test* harness links against.  The test runner
/// interprets Clojure at runtime, so it needs neither the async/IO/net/charset
/// stacks nor object-file linking.
const TEST_HARNESS_RUNTIME_CRATES: &[&str] = &[
    "cljrs-logging",
    "cljrs-types",
    "cljrs-gc",
    "cljrs-value",
    "cljrs-reader",
    "cljrs-env",
    "cljrs-eval",
    "cljrs-stdlib",
    "cljrs-compiler",
];

/// Build the harness with Cargo and copy the resulting binary to `out_path`.
///
/// `offline` passes `--offline`; it is set only for path-dep (local checkout)
/// harnesses.  Versioned (published) deps may need to be fetched from
/// crates.io, so the build must be allowed network access.
fn link_with_cargo(harness_dir: &Path, out_path: &Path, offline: bool) -> AotResult<()> {
    eprintln!("[aot] building harness with cargo...");

    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("build").arg("--release");
    if offline {
        cmd.arg("--offline");
    }
    let output = cmd.current_dir(harness_dir).output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AotError::Link(format!("cargo build failed:\n{stderr}")));
    }

    // The binary is at target/release/cljrs-aot-harness.
    let bin_name = if cfg!(target_os = "windows") {
        "cljrs-aot-harness.exe"
    } else {
        "cljrs-aot-harness"
    };
    let built = harness_dir.join("target/release").join(bin_name);
    std::fs::copy(&built, out_path)?;

    // Clean up the harness directory.
    let _ = std::fs::remove_dir_all(harness_dir);

    Ok(())
}

/// Build the harness with Cargo and copy the resulting binary to `out_path`.
/// Keeps the harness directory for debugging test harnesses.
fn link_with_cargo_test_harness(
    harness_dir: &Path,
    out_path: &Path,
    offline: bool,
) -> AotResult<()> {
    eprintln!("[aot] building harness with cargo...");

    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("build").arg("--release");
    if offline {
        cmd.arg("--offline");
    }
    let output = cmd.current_dir(harness_dir).output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AotError::Link(format!("cargo build failed:\n{stderr}")));
    }

    // The binary is at target/release/cljrs-aot-harness.
    let bin_name = if cfg!(target_os = "windows") {
        "cljrs-aot-harness.exe"
    } else {
        "cljrs-aot-harness"
    };
    let built = harness_dir.join("target/release").join(bin_name);
    std::fs::copy(&built, out_path)?;

    // Keep the harness directory for debugging
    eprintln!("[aot] harness directory kept at {}", harness_dir.display());

    Ok(())
}

/// How the generated harness depends on the clojurust runtime crates.
enum HarnessDeps {
    /// A local clojurust checkout was found — depend on the crates by path.
    Workspace(PathBuf),
    /// No checkout is available (e.g. a `cargo install cljrs` binary) — depend
    /// on the published crates from crates.io by exact version.
    Published(String),
}

impl HarnessDeps {
    /// Whether the harness build may pass `--offline`.  Path deps resolve
    /// against the local checkout (already built/cached), so offline is safe;
    /// versioned deps may still need to be fetched from crates.io.
    fn offline(&self) -> bool {
        matches!(self, HarnessDeps::Workspace(_))
    }

    /// Emit a `[dependencies]` line for one runtime crate.
    fn dep_line(&self, name: &str) -> String {
        match self {
            HarnessDeps::Workspace(root) => {
                format!(
                    "{name} = {{ path = \"{}/crates/{name}\" }}\n",
                    root.display()
                )
            }
            // `=` pins the exact version this `cljrs` was built against, so the
            // harness can never silently link a mismatched runtime.
            HarnessDeps::Published(version) => format!("{name} = \"={version}\"\n"),
        }
    }
}

/// Decide how the harness should depend on the runtime crates: against a local
/// checkout when one is found, otherwise against the published versions.
fn resolve_harness_deps() -> AotResult<HarnessDeps> {
    match find_workspace_root()? {
        Some(root) => Ok(HarnessDeps::Workspace(root)),
        // The runtime crates share the workspace version, which is this crate's
        // `CARGO_PKG_VERSION`, so they are all published in lock-step.
        None => Ok(HarnessDeps::Published(
            env!("CARGO_PKG_VERSION").to_string(),
        )),
    }
}

/// Locate the clojurust workspace root — the directory whose `Cargo.toml`
/// declares the `[workspace]` that owns the `cljrs-*` crates the generated
/// harness depends on.
///
/// Returns `Ok(None)` when no checkout is found (the caller then falls back to
/// versioned crates.io deps).  Only errors when `CLJRS_WORKSPACE_ROOT` is set
/// but invalid — an explicit override that doesn't resolve is a user mistake,
/// not a cue to silently switch to published deps.  Resolution order:
///
/// 1. `CLJRS_WORKSPACE_ROOT` env var — an explicit override for unusual
///    layouts (e.g. an installed `cljrs` pointed at a relocated source tree).
/// 2. The compiler crate's own compile-time location.  `cljrs-compiler` lives
///    at `<workspace>/crates/cljrs-compiler`, so its `CARGO_MANIFEST_DIR` is
///    two levels below the root.  This works no matter the current directory,
///    which is what lets `cljrs compile` run on a *bare* `.cljrs` file with no
///    surrounding Cargo workspace.
/// 3. As a last resort, walk up from the current directory.  This keeps
///    working in exotic setups where the source tree was moved after build
///    but the user runs `cljrs` from inside the checkout.
fn find_workspace_root() -> AotResult<Option<PathBuf>> {
    // 1. Explicit override.
    if let Some(root) = std::env::var_os("CLJRS_WORKSPACE_ROOT") {
        let dir = PathBuf::from(root);
        if is_workspace_manifest(&dir.join("Cargo.toml")) {
            return Ok(Some(dir));
        }
        return Err(AotError::Link(format!(
            "CLJRS_WORKSPACE_ROOT={} does not contain a Cargo.toml with [workspace]",
            dir.display()
        )));
    }

    // 2. Compile-time location of this crate: <workspace>/crates/cljrs-compiler.
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    if let Some(root) = manifest_dir.parent().and_then(Path::parent)
        && is_workspace_manifest(&root.join("Cargo.toml"))
    {
        return Ok(Some(root.to_path_buf()));
    }

    // 3. Walk up from the current directory.
    let mut dir = std::env::current_dir()?;
    loop {
        if is_workspace_manifest(&dir.join("Cargo.toml")) {
            return Ok(Some(dir));
        }
        if !dir.pop() {
            return Ok(None);
        }
    }
}

/// Returns true when `cargo_toml` exists and declares a `[workspace]` table.
fn is_workspace_manifest(cargo_toml: &Path) -> bool {
    std::fs::read_to_string(cargo_toml)
        .map(|contents| contents.contains("[workspace"))
        .unwrap_or(false)
}

// ── Test harness generation ─────────────────────────────────────────────────

/// Discover test namespaces from a directory of test files.
/// Returns a sorted list of namespace names.
fn discover_test_namespaces(test_dir: &Path, src_dirs: &[PathBuf]) -> AotResult<Vec<String>> {
    let mut namespaces = Vec::new();

    // First, try to discover from the test directory directly
    if test_dir.is_dir() {
        discover_in_dir(test_dir, test_dir, &mut namespaces);
    }

    // If no tests found in test_dir, also search src_dirs
    if namespaces.is_empty() {
        for dir in src_dirs {
            if dir.is_dir() {
                discover_in_dir(dir, dir, &mut namespaces);
            }
        }
    }

    namespaces.sort();
    Ok(namespaces)
}

/// Discover all namespace names from `.cljc` / `.cljrs` files in the given source paths.
fn discover_in_dir(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            discover_in_dir(root, &path, out);
        } else if let Some(ext) = path.extension()
            && (ext == "cljc" || ext == "cljrs")
            && let Some(ns) = file_to_namespace(root, &path)
        {
            out.push(ns);
        }
    }
}

/// Convert a file path relative to the source root into a Clojure namespace name.
/// e.g. `test/clojure/core_test/juxt.cljc` relative to `test/` → `clojure.core-test.juxt`
fn file_to_namespace(root: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(root).ok()?;
    let stem = rel.with_extension(""); // remove .cljc / .cljrs
    let ns = stem
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, ".")
        .replace('_', "-");
    Some(ns)
}

/// Generate the Rust test harness code.
fn generate_test_harness_code(namespaces: &[String], bundled_registration: &str) -> String {
    let mut code = String::new();

    code.push_str(
        r#"//! Auto-generated AOT test harness for clojurust.
//!
//! Discovers and runs all clojure.test tests in the bundled namespaces.

use cljrs_value::Value;

fn run() {
    // Set -X flags from environment.
    cljrs_logging::set_feature_levels_from_env().unwrap();

    // Initialize the standard environment without IR lowering.
    // The test harness interprets Clojure at runtime; there is no benefit to
    // eagerly compiling test functions to IR, and doing so fills IR_CACHE with
    // entries that are never evicted (non-GC memory, leaks across all 233 namespaces).
    let globals = cljrs_stdlib::standard_env_no_ir();

    // Override GC soft limit to a small value so the collector fires during
    // test execution.  standard_env_no_ir() calls set_config_from_env() which
    // defaults to system_memory/3 (often 5+ GB); at that level memory_in_use
    // (which only tracks GcBox sizes) never reaches the threshold, GC never
    // runs, and all temporary Values + freed namespace closures accumulate.
    // 64 MB is enough to trigger multiple collections per namespace test while
    // adding negligible overhead (<1%).  CLJRS_GC_SOFT_LIMIT_MB overrides the
    // 64 MB default for debugging/profiling.
    {
        let soft_mb: usize = std::env::var("CLJRS_GC_SOFT_LIMIT_MB")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(64);
        cljrs_gc::HEAP.set_config(std::sync::Arc::new(
            cljrs_gc::GcConfig::with_limits(
                soft_mb * 1024 * 1024,
                soft_mb * 2 * 1024 * 1024,
            ),
        ));
    }

    // Register bundled dependency sources so require can find them
    // without needing source files on disk.
"#,
    );

    code.push_str(bundled_registration);
    code.push_str(
        r#"    let mut env = cljrs_eval::Env::new(globals, "user");

    // Push an eval context so rt_call can dispatch through the interpreter.
    cljrs_env::callback::push_eval_context(&env);

    // Load clojure.test once; it stays loaded for the entire run.
    // push_alloc_frame() scopes transient eval allocations: after the frame
    // drops, those GcBoxes are removed from ALLOC_ROOTS.  The namespace objects
    // themselves stay alive via globals.namespaces, so GC can still trace them.
    {
        let _frame = cljrs_gc::push_alloc_frame();
        let _ = cljrs_eval::eval(
            &cljrs_reader::Parser::new(
                "(require 'clojure.test)".to_string(),
                "<test-harness>".to_string()
            ).parse_all().unwrap()[0],
            &mut env
        );
    }

    // Load, test, and unload each test namespace one at a time.
    //
    // Memory strategy: each test namespace and all its vars/closures are
    // removed from GlobalEnv after its tests finish, then two explicit GC
    // cycles free the now-unreachable objects.  Two cycles are required
    // because GC_INITIAL_LIVES=2 gives objects one grace cycle before they
    // are actually freed.  This keeps peak RSS proportional to one namespace
    // at a time rather than all 233 simultaneously.
    let mut total_pass = 0i64;
    let mut total_fail = 0i64;
    let mut total_error = 0i64;
    let mut total_test_count = 0i64;

    for ns_str in &[
"#,
    );

    for ns in namespaces.iter() {
        code.push_str(&format!("        \"{}\",\n", ns));
    }

    code.push_str(
        r#"    ] {
        // Load this test namespace.  push_alloc_frame() scopes all GcBox
        // allocations made during require: when the frame drops, those entries
        // are removed from ALLOC_ROOTS so GC treats them as non-roots.  The
        // namespace vars/closures remain reachable via globals.namespaces and
        // are kept alive by GC tracing through that reference; purely transient
        // allocations from eval become eligible for collection immediately.
        {
            let _frame = cljrs_gc::push_alloc_frame();
            let _ = cljrs_eval::eval(
                &cljrs_reader::Parser::new(
                    format!("(require '{})", ns_str).to_string(),
                    "<test-harness>".to_string()
                ).parse_all().unwrap()[0],
                &mut env
            );
        }

        // Run its tests.  Same alloc-frame scoping so transient test
        // infrastructure objects are freed after each namespace.
        let run_result = {
            let _frame = cljrs_gc::push_alloc_frame();
            cljrs_eval::eval(
                &cljrs_reader::Parser::new(
                    format!("(clojure.test/run-tests '{})", ns_str).to_string(),
                    "<run-tests>".to_string()
                ).parse_all().unwrap()[0],
                &mut env
            )
        };
        if let Ok(Value::Map(m)) = run_result {
            let mut pass = 0i64;
            let mut fail = 0i64;
            let mut error = 0i64;
            let mut test_count = 0i64;
            m.for_each(|k, v| {
                if let (Value::Keyword(kw), Value::Long(count)) = (k, v) {
                    match kw.get().name.as_ref() {
                        "pass" => pass = *count,
                        "fail" => fail = *count,
                        "error" => error = *count,
                        "test" => test_count = *count,
                        _ => {}
                    }
                }
            });
            total_pass += pass;
            total_fail += fail;
            total_error += error;
            total_test_count += test_count;
        }

        // Unload the test namespace so the GC can reclaim its vars,
        // closures, and form trees.  GcPtr::Drop is a no-op, so we must
        // remove the namespace from GlobalEnv before collecting.
        // The pressure-based GC (triggered by gc_safepoint during the next
        // namespace's test execution) will naturally collect the freed objects:
        // they are no longer reachable from GlobalEnv::namespaces and will be
        // swept in the next collection cycle.
        {
            let ns_key: std::sync::Arc<str> = std::sync::Arc::from(*ns_str);
            env.globals.namespaces.write().unwrap().remove(&*ns_key);
            env.globals.loaded.lock().unwrap().remove(&*ns_key);
        }
    }

    // Flush output before exiting
    std::io::Write::flush(&mut std::io::stdout()).unwrap();
    println!("Ran {} tests containing {} assertions.", total_test_count, total_pass + total_fail + total_error);
    std::io::Write::flush(&mut std::io::stdout()).unwrap();
    println!("{} passed, {} failed, {} errors.", total_pass, total_fail, total_error);
    std::io::Write::flush(&mut std::io::stdout()).unwrap();

    // Pop the eval context.
    cljrs_env::callback::pop_eval_context();

    // If CLJRS_GC_STATS is set, dump GC stats to its target (stdout/file).
    cljrs_gc::dump_stats_from_env();

    if total_fail > 0 || total_error > 0 {
        std::process::exit(1);
    }
}

fn main() {
    // Run on a large-stack thread to avoid stack overflows in deeply recursive
    // Clojure code (macros, lazy sequences, recursive interpreting).
    const STACK_SIZE: usize = 64 * 1024 * 1024; // 64 MiB
    let thread = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(run)
        .expect("failed to spawn main thread");
    if let Err(e) = thread.join() {
        std::panic::resume_unwind(e);
    }
}"#,
    );

    code
}

/// Compile a directory of test files to a standalone native binary.
/// The resulting binary will discover and run all clojure.test tests found.
pub fn compile_test_harness(
    test_dir: &Path,
    out_path: &Path,
    src_dirs: &[PathBuf],
) -> AotResult<()> {
    eprintln!("[aot] discovering tests in {}", test_dir.display());

    // Discover test namespaces
    let test_namespaces = discover_test_namespaces(test_dir, src_dirs)?;
    if test_namespaces.is_empty() {
        return Err(AotError::Eval(format!(
            "No test files found in {}",
            test_dir.display()
        )));
    }
    eprintln!(
        "[aot] discovered {} test namespace(s)",
        test_namespaces.len()
    );

    // Also discover source namespaces from src_dirs so they get bundled
    let mut src_namespaces = Vec::new();
    for dir in src_dirs {
        if dir.is_dir() {
            discover_in_dir(dir, dir, &mut src_namespaces);
        }
    }
    src_namespaces.sort();
    eprintln!(
        "[aot] discovered {} source namespace(s)",
        src_namespaces.len()
    );

    // Combine: source namespaces first (so they're registered before tests require them),
    // then test namespaces. Deduplicate in case of overlap.
    let mut all_namespaces = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for ns in src_namespaces.iter().chain(test_namespaces.iter()) {
        if seen.insert(ns.clone()) {
            all_namespaces.push(ns.clone());
        }
    }

    // Generate registration code for bundled sources
    let mut bundled_registration = String::new();
    for (i, ns) in all_namespaces.iter().enumerate() {
        bundled_registration.push_str(&format!(
            "    globals.register_builtin_source(\"{ns}\", include_str!(\"bundled_{i}.cljrs\"));\n"
        ));
    }

    // Create the harness directory
    let harness_dir = out_path
        .parent()
        .unwrap_or(Path::new("."))
        .join(".cljrs-aot-test-harness");

    // Clean any previous harness.
    if harness_dir.exists() {
        std::fs::remove_dir_all(&harness_dir)?;
    }
    std::fs::create_dir_all(harness_dir.join("src"))?;

    // Generate the main.rs file (only test namespaces get run-tests called)
    let main_rs = generate_test_harness_code(&test_namespaces, &bundled_registration);
    std::fs::write(harness_dir.join("src/main.rs"), &main_rs)?;

    // Write all namespace sources for bundling
    // Include test_dir as a search path for test sources
    let mut search_dirs = src_dirs.to_vec();
    search_dirs.push(test_dir.to_path_buf());

    for (i, ns) in all_namespaces.iter().enumerate() {
        let rel_path = ns.replace('.', "/").replace('-', "_");
        if let Some(src) = find_user_source(&rel_path, &search_dirs) {
            std::fs::write(
                harness_dir.join("src").join(format!("bundled_{i}.cljrs")),
                &src,
            )?;
            eprintln!("[aot] bundled {ns} → src/bundled_{i}.cljrs");
        } else {
            return Err(AotError::Eval(format!(
                "Could not find source for namespace {ns}"
            )));
        }
    }

    // Write Cargo.toml.  Resolve deps the same way as the run harness: a local
    // checkout (path deps) when found, otherwise the published versions.
    let deps = resolve_harness_deps()?;
    let runtime_deps: String = TEST_HARNESS_RUNTIME_CRATES
        .iter()
        .map(|c| deps.dep_line(c))
        .collect();
    let cargo_toml = format!(
        r#"[package]
name = "cljrs-aot-harness"
version = "0.1.0"
edition = "2021"

[workspace]

[dependencies]
{runtime_deps}"#,
    );
    std::fs::write(harness_dir.join("Cargo.toml"), cargo_toml)?;

    // Write build.rs - minimal, no object file linking needed
    let build_rs = r#"fn main() {
    // No special linking needed for test harness
}"#;
    std::fs::write(harness_dir.join("build.rs"), build_rs)?;

    // Build with cargo
    link_with_cargo_test_harness(&harness_dir, out_path, deps.offline())?;

    eprintln!("[aot] wrote {}", out_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_deps_use_path() {
        let deps = HarnessDeps::Workspace(PathBuf::from("/checkout/clojurust"));
        assert_eq!(
            deps.dep_line("cljrs-stdlib"),
            "cljrs-stdlib = { path = \"/checkout/clojurust/crates/cljrs-stdlib\" }\n"
        );
        // Path deps come from a built checkout, so the build may stay offline.
        assert!(deps.offline());
    }

    #[test]
    fn published_deps_pin_exact_version() {
        let deps = HarnessDeps::Published("0.1.0".to_string());
        // `=` pins the exact version this cljrs was built against so the
        // harness can never link a mismatched runtime.
        assert_eq!(deps.dep_line("cljrs-stdlib"), "cljrs-stdlib = \"=0.1.0\"\n");
        // Published deps may need fetching, so the build must allow network.
        assert!(!deps.offline());
    }

    #[test]
    fn published_version_matches_this_crate() {
        // resolve_harness_deps() falls back to this crate's version, which the
        // workspace shares across every runtime crate, so they publish in
        // lock-step.
        if let HarnessDeps::Published(v) = HarnessDeps::Published(env!("CARGO_PKG_VERSION").into())
        {
            assert_eq!(v, env!("CARGO_PKG_VERSION"));
        }
    }
}
