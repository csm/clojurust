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
use cljrs_eval::ir_convert;

// ── Error type ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum AotError {
    Io(std::io::Error),
    Parse(cljrs_types::error::CljxError),
    Codegen(crate::codegen::CodegenError),
    Eval(String),
    Link(String),
}

impl std::fmt::Display for AotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AotError::Io(e) => write!(f, "I/O error: {e}"),
            AotError::Parse(e) => write!(f, "parse error: {e}"),
            AotError::Codegen(e) => write!(f, "codegen error: {e:?}"),
            AotError::Eval(e) => write!(f, "eval/lowering error: {e}"),
            AotError::Link(e) => write!(f, "link error: {e}"),
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

// ── Clojure-based lowering ──────────────────────────────────────────────────

/// Lower forms via the Clojure compiler front-end.
///
/// 1. Ensures compiler namespaces are loaded
/// 2. Converts Forms → Values via `form_to_value`
/// 3. Calls `cljrs.compiler.anf/lower-fn-body` via `callback::invoke`
/// 4. Converts returned Value → `IrFunction` via `ir_convert`
pub fn lower_via_clojure(
    name: Option<&str>,
    ns: &str,
    params: &[Arc<str>],
    compilable_forms: &[cljrs_reader::Form],
    env: &mut cljrs_eval::Env,
) -> AotResult<IrFunction> {
    // Register compiler sources so require can find them.
    crate::register_compiler_sources(&env.globals);

    // Push eval context so callback::invoke can work.
    cljrs_env::callback::push_eval_context(env);

    // Set IR_LOWERING_ACTIVE to prevent eager lowering of closures
    // created inside the Clojure compiler during this lowering call.
    use cljrs_eval::apply::IR_LOWERING_ACTIVE;
    let was_active = IR_LOWERING_ACTIVE.with(|c| c.get());
    IR_LOWERING_ACTIVE.with(|c| c.set(true));

    let result = lower_via_clojure_inner(name, ns, params, compilable_forms, env);

    // Restore lowering flag and pop eval context.
    IR_LOWERING_ACTIVE.with(|c| c.set(was_active));
    cljrs_env::callback::pop_eval_context();

    result
}

fn lower_via_clojure_inner(
    name: Option<&str>,
    ns: &str,
    params: &[Arc<str>],
    compilable_forms: &[cljrs_reader::Form],
    env: &mut cljrs_eval::Env,
) -> AotResult<IrFunction> {
    use cljrs_gc::GcPtr;
    use cljrs_value::Value;
    use cljrs_value::collections::vector::PersistentVector;

    // Ensure the ANF and optimize namespaces are loaded.
    let span = || cljrs_types::span::Span::new(Arc::new("<aot>".to_string()), 0, 0, 1, 1);
    for ns_name in &["cljrs.compiler.anf", "cljrs.compiler.optimize"] {
        let require_form = cljrs_reader::Form::new(
            cljrs_reader::form::FormKind::List(vec![
                cljrs_reader::Form::new(
                    cljrs_reader::form::FormKind::Symbol("require".into()),
                    span(),
                ),
                cljrs_reader::Form::new(
                    cljrs_reader::form::FormKind::Quote(Box::new(cljrs_reader::Form::new(
                        cljrs_reader::form::FormKind::Symbol((*ns_name).into()),
                        span(),
                    ))),
                    span(),
                ),
            ]),
            span(),
        );
        cljrs_eval::eval(&require_form, env).map_err(|e| AotError::Eval(format!("{e:?}")))?;
    }

    // Look up the lower-fn-body function.
    let lower_fn = env
        .globals
        .lookup_var_in_ns("cljrs.compiler.anf", "lower-fn-body")
        .ok_or_else(|| AotError::Eval("cljrs.compiler.anf/lower-fn-body not found".to_string()))?;
    let lower_fn_val = lower_fn.get().deref().unwrap_or(Value::Nil);

    // Build arguments:
    // 1. fname (string or nil)
    let fname_val = match name {
        Some(n) => Value::string(n.to_string()),
        None => Value::Nil,
    };

    // 2. ns (string)
    let ns_val = Value::string(ns.to_string());

    // 3. params (vector of strings)
    let params_val = Value::Vector(GcPtr::new(PersistentVector::from_iter(
        params.iter().map(|p| Value::string(p.to_string())),
    )));

    // 4. body-forms (vector of form values)
    let body_forms_val = Value::Vector(GcPtr::new(PersistentVector::from_iter(
        compilable_forms
            .iter()
            .map(cljrs_builtins::form::form_to_value),
    )));

    // Call the Clojure lowering function.
    let ir_data = cljrs_env::callback::invoke(
        &lower_fn_val,
        vec![fname_val, ns_val, params_val, body_forms_val],
    )
    .map_err(|e| AotError::Eval(format!("Clojure lowering failed: {e:?}")))?;

    // Run the optimization pass (escape analysis → region allocation).
    let optimize_fn = env
        .globals
        .lookup_var_in_ns("cljrs.compiler.optimize", "optimize")
        .ok_or_else(|| AotError::Eval("cljrs.compiler.optimize/optimize not found".to_string()))?;
    let optimize_fn_val = optimize_fn.get().deref().unwrap_or(Value::Nil);

    let optimized = cljrs_env::callback::invoke(&optimize_fn_val, vec![ir_data])
        .map_err(|e| AotError::Eval(format!("Optimization failed: {e:?}")))?;

    // Convert the result Value → IrFunction.
    ir_convert::value_to_ir_function(&optimized)
        .map_err(|e| AotError::Eval(format!("IR conversion failed: {e}")))
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

// ── Public API ──────────────────────────────────────────────────────────────

/// Compile a `.cljrs` / `.cljc` source file to a standalone native binary.
///
/// `src_path` is the input source file.  `out_path` is the desired output
/// binary.  `src_dirs` are additional directories for `require` resolution
/// during macro expansion.
pub fn compile_file(src_path: &Path, out_path: &Path, src_dirs: &[PathBuf]) -> AotResult<()> {
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
    let mut ir_func = lower_via_clojure(
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
    let harness_dir = build_harness(out_path, &obj_bytes, &interpreted_source, &bundled_sources)?;
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
/// cannot handle (e.g. alter-meta!, vary-meta). This recurses
/// into the form tree so that e.g. `(do (def x ...) (alter-meta! ...))` is caught.
fn expanded_needs_interpreter(form: &cljrs_reader::Form) -> bool {
    use cljrs_reader::form::FormKind;
    match &form.kind {
        FormKind::List(parts) => {
            if let Some(head) = parts.first()
                && let FormKind::Symbol(s) = &head.kind
                && is_interpreter_only_sym(s)
            {
                return true;
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
    let cargo_toml = format!(
        r#"[package]
name = "cljrs-aot-harness"
version = "0.1.0"
edition = "2024"

[workspace]

[dependencies]
cljrs-types    = {{ path = "{ws}/crates/cljrs-types" }}
cljrs-gc       = {{ path = "{ws}/crates/cljrs-gc" }}
cljrs-value    = {{ path = "{ws}/crates/cljrs-value" }}
cljrs-reader   = {{ path = "{ws}/crates/cljrs-reader" }}
cljrs-env      = {{ path = "{ws}/crates/cljrs-env" }}
cljrs-eval     = {{ path = "{ws}/crates/cljrs-eval" }}
cljrs-stdlib   = {{ path = "{ws}/crates/cljrs-stdlib" }}
cljrs-compiler = {{ path = "{ws}/crates/cljrs-compiler" }}

[build-dependencies]
cc = "1"
"#,
        ws = workspace_root.display()
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

    let main_rs = format!(
        r#"//! Auto-generated AOT harness for clojurust.
//!
//! Initializes the runtime, then calls the compiled `__cljrs_main`.

#![allow(improper_ctypes)]

use cljrs_value::Value;

unsafe extern "C" {{
    fn __cljrs_main() -> *const Value;
}}

fn main() {{
    // Ensure all rt_* symbols are linked into the binary.
    cljrs_compiler::rt_abi::anchor_rt_symbols();

    // Initialize the standard environment so that rt_call and other
    // runtime bridge functions can look up builtins.
    let globals = cljrs_stdlib::standard_env();

    // Register bundled dependency sources so require can find them
    // without needing source files on disk.
{bundled}
    let mut env = cljrs_eval::Env::new(globals, "user");

    // Push an eval context so rt_call can dispatch through the interpreter.
    cljrs_env::callback::push_eval_context(&env);
{preamble}
    // Call the compiled code.
    let _result = unsafe {{ __cljrs_main() }};

    // Pop the eval context.
    cljrs_env::callback::pop_eval_context();
}}
"#,
        preamble = preamble_code,
        bundled = bundled_registration
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

    // Generate the namespace strings array inline
    let ns_strings: Vec<String> = namespaces
        .iter()
        .map(|s| format!("\"{}\".to_string()", s))
        .collect();

    code.push_str(
        r#"//! Auto-generated AOT test harness for clojurust.
//!
//! Discovers and runs all clojure.test tests in the bundled namespaces.

use cljrs_value::Value;

fn main() {
    // Initialize the standard environment.
    let globals = cljrs_stdlib::standard_env();

    // Register bundled dependency sources so require can find them
    // without needing source files on disk.
"#,
    );

    code.push_str(bundled_registration);
    code.push_str(
        r#"    let mut env = cljrs_eval::Env::new(globals, "user");

    // Push an eval context so rt_call can dispatch through the interpreter.
    cljrs_env::callback::push_eval_context(&env);

    // Load clojure.test if not already loaded
    let _ = cljrs_eval::eval(
        &cljrs_reader::Parser::new(
            "(require 'clojure.test)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0],
        &mut env
    );

    // Load all test namespaces
    (|| {
"#,
    );

    for ns in namespaces.iter() {
        code.push_str(&format!(
            "        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(\n            \"(require '{})\".to_string(),\n            \"<test-harness>\".to_string()\n        ).parse_all().unwrap()[0], &mut env);\n",
            ns
        ));
    }

    code.push_str(
        r#"    })();

    // Run tests for each namespace separately
    let mut total_pass = 0i64;
    let mut total_fail = 0i64;
    let mut total_error = 0i64;
    let mut total_test_count = 0i64;

    for ns_str in vec![
"#,
    );

    for ns_str in ns_strings.iter() {
        code.push_str(&format!("        {},\n", ns_str));
    }

    code.push_str(r#"    ].iter() {
        let run_result = cljrs_eval::eval(
            &cljrs_reader::Parser::new(
                format!("(clojure.test/run-tests '{})", ns_str)
                    .to_string(),
                "<run-tests>".to_string()
            ).parse_all().unwrap()[0],
            &mut env
        );
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
    }

    // Flush output before exiting
    std::io::Write::flush(&mut std::io::stdout()).unwrap();
    println!("Ran {} tests containing {} assertions.", total_test_count, total_pass + total_fail + total_error);
    std::io::Write::flush(&mut std::io::stdout()).unwrap();
    println!("{} passed, {} failed, {} errors.", total_pass, total_fail, total_error);
    std::io::Write::flush(&mut std::io::stdout()).unwrap();
    if total_fail > 0 || total_error > 0 {
        std::process::exit(1);
    }

    // Pop the eval context.
    cljrs_env::callback::pop_eval_context();
}"#);

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
