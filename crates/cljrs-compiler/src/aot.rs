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

pub type AotResult<T> = Result<T, AotError>;

// ── Rust-native lowering ────────────────────────────────────────────────────

/// Lower forms directly via the native Rust compiler pipeline.
///
/// Replaces the old `lower_via_clojure` path: no interpreter round-trip,
/// no `callback::invoke`, no `ir_convert`.
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

/// Compile a `.cljrs` / `.cljc` source file to a standalone native binary.
///
/// `src_path` is the input source file.  `out_path` is the desired output
/// binary.  `src_dirs` are additional directories for `require` resolution
/// during macro expansion.  `rust_config`, when present, causes the generated
/// harness to depend on the user's Rust crate and call its `cljrs_init` hook
/// before loading any Clojure code.
pub fn compile_file(
    src_path: &Path,
    out_path: &Path,
    src_dirs: &[PathBuf],
    rust_config: Option<&cljrs_deps::RustConfig>,
) -> AotResult<()> {
    eprintln!("[aot] reading {}", src_path.display());
    let source = std::fs::read_to_string(src_path)?;
    let filename = src_path.display().to_string();

    // ── 1. Parse ────────────────────────────────────────────────────────
    let mut parser = Parser::new(source.clone(), filename);
    let forms = parser.parse_all()?;
    eprintln!("[aot] parsed {} top-level form(s)", forms.len());

    // ── 2. Macro-expand ─────────────────────────────────────────────────
    // Boot a full environment so macros resolve correctly.
    let globals = if src_dirs.is_empty() {
        cljrs_stdlib::standard_env()
    } else {
        cljrs_stdlib::standard_env_with_paths(src_dirs.to_vec())
    };
    // Register clojure.core.async so that (require '[clojure.core.async ...])
    // and the `go`/`alt` macros resolve during macro-expansion. The GC
    // service is silently skipped when there is no LocalSet context.
    cljrs_async::init(&globals);
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

    // Discover user namespaces loaded during expansion (transitive deps).
    let bundled_sources = discover_bundled_sources(&env.globals, &pre_loaded, src_dirs);
    if !bundled_sources.is_empty() {
        eprintln!(
            "[aot] bundling {} required namespace(s): {}",
            bundled_sources.len(),
            bundled_sources
                .iter()
                .map(|(ns, _)| ns.as_ref())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

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
            compilable.push(form.clone());
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
    let obj_bytes = compiler.finish();
    eprintln!("[aot] generated {} bytes of object code", obj_bytes.len());

    // ── 5. Generate harness project & build ─────────────────────────────
    let harness_dir = build_harness(
        out_path,
        &obj_bytes,
        &interpreted_source,
        &bundled_sources,
        rust_config,
    )?;
    link_with_cargo(&harness_dir, out_path)?;

    eprintln!("[aot] wrote {}", out_path.display());
    Ok(())
}

/// Check if a top-level form needs the interpreter (can't be AOT-compiled yet).
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
        compiler.declare_function(name, sub.params.len())?;
        declare_subfunctions(sub, compiler)?;
    }
    Ok(())
}

/// Recursively compile all subfunctions.
fn compile_subfunctions(ir_func: &IrFunction, compiler: &mut Compiler) -> AotResult<()> {
    for sub in &ir_func.subfunctions {
        compile_subfunctions(sub, compiler)?;
        let name = sub.name.as_deref().unwrap_or("__cljrs_anon");
        let func_id = compiler.declare_function(name, sub.params.len())?;
        compiler.compile_function(sub, func_id)?;
    }
    Ok(())
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
        // Skip compiler-internal namespaces.
        if ns.starts_with("cljrs.compiler.") {
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

// ── Harness generation ──────────────────────────────────────────────────────

/// Create a temporary Cargo project that links the compiled object code with
/// the clojurust runtime and produces a binary.
fn build_harness(
    out_path: &Path,
    obj_bytes: &[u8],
    interpreted_source: &str,
    bundled_sources: &[(Arc<str>, String)],
    rust_config: Option<&cljrs_deps::RustConfig>,
) -> AotResult<PathBuf> {
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

    // Find the workspace root (where the top-level Cargo.toml lives).
    let workspace_root = find_workspace_root()?;

    // Write Cargo.toml.
    // The empty [workspace] table prevents Cargo from thinking this is
    // part of a parent workspace.
    let ws = workspace_root.display();
    let mut native_deps = String::new();
    if let Some(rc) = rust_config {
        native_deps.push_str(&format!(
            "cljrs-interop  = {{ path = \"{ws}/crates/cljrs-interop\" }}\n"
        ));
        if let Some(crate_name) = rc.crate_name() {
            let crate_dir = rc.crate_dir.display();
            native_deps.push_str(&format!("{crate_name} = {{ path = \"{crate_dir}\" }}\n"));
        }
    }
    let cargo_toml = format!(
        r#"[package]
name = "cljrs-aot-harness"
version = "0.1.0"
edition = "2024"

[workspace]

[dependencies]
cljrs-logging  = {{ path = "{ws}/crates/cljrs-logging" }}
cljrs-types    = {{ path = "{ws}/crates/cljrs-types" }}
cljrs-gc       = {{ path = "{ws}/crates/cljrs-gc" }}
cljrs-value    = {{ path = "{ws}/crates/cljrs-value" }}
cljrs-reader   = {{ path = "{ws}/crates/cljrs-reader" }}
cljrs-env      = {{ path = "{ws}/crates/cljrs-env" }}
cljrs-eval     = {{ path = "{ws}/crates/cljrs-eval" }}
cljrs-stdlib   = {{ path = "{ws}/crates/cljrs-stdlib" }}
cljrs-compiler = {{ path = "{ws}/crates/cljrs-compiler" }}
cljrs-async    = {{ path = "{ws}/crates/cljrs-async" }}
tokio          = {{ version = "1", features = ["rt", "time"] }}
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

    // Write bundled dependency sources.
    for (i, (ns, src)) in bundled_sources.iter().enumerate() {
        let filename = format!("bundled_{i}.cljrs");
        std::fs::write(harness_dir.join("src").join(&filename), src)?;
        eprintln!("[aot] bundled {ns} → src/{filename}");
    }

    // Generate registration code for bundled sources.
    let mut bundled_registration = String::new();
    for (i, (ns, _)) in bundled_sources.iter().enumerate() {
        bundled_registration.push_str(&format!(
            "    globals.register_builtin_source(\"{ns}\", \
             include_str!(\"bundled_{i}.cljrs\"));\n"
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

    let main_rs = format!(
        r#"//! Auto-generated AOT harness for clojurust.
//!
//! Initializes the runtime, then calls the compiled `__cljrs_main`.

#![allow(improper_ctypes)]

use cljrs_value::Value;

unsafe extern "C" {{
    fn __cljrs_main() -> *const Value;
}}

async fn run() {{
    // Parse environment -X flags.
    cljrs_logging::set_feature_levels_from_env().unwrap();

    // Ensure all rt_* symbols are linked into the binary.
    cljrs_compiler::rt_abi::anchor_rt_symbols();

    // Initialize the standard environment so that rt_call and other
    // runtime bridge functions can look up builtins.
    let globals = cljrs_stdlib::standard_env();

    // Register the async runtime (clojure.core.async, ^:async dispatch, await).
    cljrs_async::init(&globals);

    // Register bundled dependency sources so require can find them
    // without needing source files on disk.
{bundled}{native_init}
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
                .enable_time()
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
        native_init = native_init_code,
        main_call = main_call_code,
    );
    std::fs::write(harness_dir.join("src/main.rs"), main_rs)?;

    Ok(harness_dir)
}

/// Build the harness with Cargo and copy the resulting binary to `out_path`.
fn link_with_cargo(harness_dir: &Path, out_path: &Path) -> AotResult<()> {
    eprintln!("[aot] building harness with cargo...");

    let output = std::process::Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--offline")
        .current_dir(harness_dir)
        .output()?;

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
fn link_with_cargo_test_harness(harness_dir: &Path, out_path: &Path) -> AotResult<()> {
    eprintln!("[aot] building harness with cargo...");

    let output = std::process::Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--offline")
        .current_dir(harness_dir)
        .output()?;

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

/// Walk up from the current directory to find the workspace root
/// (the directory containing Cargo.toml with [workspace]).
fn find_workspace_root() -> AotResult<PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists() {
            let contents = std::fs::read_to_string(&cargo_toml)?;
            if contents.contains("[workspace") {
                return Ok(dir);
            }
        }
        if !dir.pop() {
            return Err(AotError::Link(
                "could not find workspace root (no Cargo.toml with [workspace])".to_string(),
            ));
        }
    }
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
    // standard_env_no_ir() also skips loading the cljrs.compiler.* namespaces.
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

    // Write Cargo.toml
    let workspace_root = find_workspace_root()?;
    let cargo_toml = format!(
        r#"[package]
name = "cljrs-aot-harness"
version = "0.1.0"
edition = "2021"

[workspace]

[dependencies]
cljrs-logging  = {{ path = "{ws}/crates/cljrs-logging" }}
cljrs-types    = {{ path = "{ws}/crates/cljrs-types" }}
cljrs-gc       = {{ path = "{ws}/crates/cljrs-gc" }}
cljrs-value    = {{ path = "{ws}/crates/cljrs-value" }}
cljrs-reader   = {{ path = "{ws}/crates/cljrs-reader" }}
cljrs-env      = {{ path = "{ws}/crates/cljrs-env" }}
cljrs-eval     = {{ path = "{ws}/crates/cljrs-eval" }}
cljrs-stdlib   = {{ path = "{ws}/crates/cljrs-stdlib" }}
cljrs-compiler = {{ path = "{ws}/crates/cljrs-compiler" }}
"#,
        ws = workspace_root.display()
    );
    std::fs::write(harness_dir.join("Cargo.toml"), cargo_toml)?;

    // Write build.rs - minimal, no object file linking needed
    let build_rs = r#"fn main() {
    // No special linking needed for test harness
}"#;
    std::fs::write(harness_dir.join("build.rs"), build_rs)?;

    // Build with cargo
    link_with_cargo_test_harness(&harness_dir, out_path)?;

    eprintln!("[aot] wrote {}", out_path.display());
    Ok(())
}
