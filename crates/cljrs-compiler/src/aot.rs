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
use crate::ir_convert;

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
    cljrs_eval::callback::push_eval_context(env);

    let result = lower_via_clojure_inner(name, ns, params, compilable_forms, env);

    // Always pop eval context, regardless of success or failure.
    cljrs_eval::callback::pop_eval_context();

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

    // Ensure the ANF namespace is loaded.
    let require_form = cljrs_reader::Form::new(
        cljrs_reader::form::FormKind::List(vec![
            cljrs_reader::Form::new(
                cljrs_reader::form::FormKind::Symbol("require".into()),
                cljrs_types::span::Span::new(Arc::new("<aot>".to_string()), 0, 0, 1, 1),
            ),
            cljrs_reader::Form::new(
                cljrs_reader::form::FormKind::Quote(Box::new(cljrs_reader::Form::new(
                    cljrs_reader::form::FormKind::Symbol("cljrs.compiler.anf".into()),
                    cljrs_types::span::Span::new(Arc::new("<aot>".to_string()), 0, 0, 1, 1),
                ))),
                cljrs_types::span::Span::new(Arc::new("<aot>".to_string()), 0, 0, 1, 1),
            ),
        ]),
        cljrs_types::span::Span::new(Arc::new("<aot>".to_string()), 0, 0, 1, 1),
    );
    cljrs_eval::eval(&require_form, env).map_err(|e| AotError::Eval(format!("{e:?}")))?;

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
        compilable_forms.iter().map(cljrs_eval::eval::form_to_value),
    )));

    // Call the Clojure function.
    let result = cljrs_eval::callback::invoke(
        &lower_fn_val,
        vec![fname_val, ns_val, params_val, body_forms_val],
    )
    .map_err(|e| AotError::Eval(format!("Clojure lowering failed: {e:?}")))?;

    // Convert the result Value → IrFunction.
    ir_convert::value_to_ir_function(&result)
        .map_err(|e| AotError::Eval(format!("IR conversion failed: {e}")))
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

    let mut expanded = Vec::with_capacity(forms.len());
    for form in &forms {
        match cljrs_eval::macros::macroexpand(form, &mut env) {
            Ok(f) => expanded.push(f),
            Err(e) => return Err(AotError::Eval(format!("{e:?}"))),
        }
    }
    eprintln!("[aot] macro-expanded {} form(s)", expanded.len());

    // ── 2b. Partition: interpreted preamble vs compiled body ─────────
    // Forms that define functions (defn, defmacro) or require interpreter
    // features (closures) are evaluated at startup via the interpreter.
    // The rest is AOT-compiled.
    let mut interpreted_source = String::new();
    let mut compilable = Vec::new();
    for (i, form) in expanded.iter().enumerate() {
        if needs_interpreter(&forms[i]) {
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

    let ir_func = lower_via_clojure(
        Some("__cljrs_main"),
        "user",
        &params,
        &compilable_forms,
        &mut env,
    )?;
    eprintln!(
        "[aot] lowered to {} block(s), {} var(s)",
        ir_func.blocks.len(),
        ir_func.next_var
    );

    // ── 4. Cranelift codegen → .o ───────────────────────────────────────
    let mut compiler = Compiler::new()?;
    let func_id = compiler.declare_function("__cljrs_main", 0)?;
    compiler.compile_function(&ir_func, func_id)?;
    let obj_bytes = compiler.finish();
    eprintln!("[aot] generated {} bytes of object code", obj_bytes.len());

    // ── 5. Generate harness project & build ─────────────────────────────
    let harness_dir = build_harness(out_path, &obj_bytes, &interpreted_source)?;
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
                // defn, defmacro, defonce need the interpreter because
                // they create closures (fn* values) which codegen can't
                // emit yet.
                return matches!(
                    s.as_str(),
                    "defn" | "defmacro" | "defonce" | "ns" | "require"
                );
            }
            false
        }
        _ => false,
    }
}

// ── Harness generation ──────────────────────────────────────────────────────

/// Create a temporary Cargo project that links the compiled object code with
/// the clojurust runtime and produces a binary.
fn build_harness(
    out_path: &Path,
    obj_bytes: &[u8],
    interpreted_source: &str,
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

    // Write main.rs — calls into the compiled __cljrs_main.
    let preamble_code = if has_preamble {
        r#"
    // Evaluate interpreted preamble (defn, defmacro, etc.).
    let preamble = include_str!("preamble.cljrs");
    let mut parser = cljrs_reader::Parser::new(preamble.to_string(), "<preamble>".to_string());
    let forms = parser.parse_all().expect("preamble parse error");
    for form in &forms {
        cljrs_eval::eval(form, &mut env).expect("preamble eval error");
    }
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
    let mut env = cljrs_eval::Env::new(globals, "user");

    // Push an eval context so rt_call can dispatch through the interpreter.
    cljrs_eval::callback::push_eval_context(&env);
{preamble}
    // Call the compiled code.
    let _result = unsafe {{ __cljrs_main() }};

    // Pop the eval context.
    cljrs_eval::callback::pop_eval_context();
}}
"#,
        preamble = preamble_code
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
