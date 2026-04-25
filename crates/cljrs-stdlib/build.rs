//! Build script: pre-lower clojure.core functions to IR and write the bundle
//! to OUT_DIR so it can be embedded via `include_bytes!`.
//!
//! Only runs when the `prebuild-ir` feature is enabled.

#[cfg(feature = "prebuild-ir")]
fn main() {
    use std::path::PathBuf;

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let output = out_dir.join("core_ir.bin");

    // Re-run if bootstrap sources change.
    println!("cargo::rerun-if-changed=../cljrs-builtins/src/bootstrap.cljrs");
    println!("cargo::rerun-if-changed=../cljrs-ir/src/clojure/compiler/anf.cljrs");
    println!("cargo::rerun-if-changed=../cljrs-ir/src/clojure/compiler/ir.cljrs");
    println!("cargo::rerun-if-changed=../cljrs-ir/src/clojure/compiler/known.cljrs");
    println!("cargo::rerun-if-changed=../cljrs-ir/src/clojure/compiler/escape.cljrs");
    println!("cargo::rerun-if-changed=../cljrs-ir/src/clojure/compiler/optimize.cljrs");

    // The Clojure compiler uses deep recursion; run on a large-stack thread.
    let result = std::thread::Builder::new()
        .name("ir-prebuild".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || prebuild_core(&output))
        .expect("failed to spawn prebuild thread")
        .join()
        .expect("prebuild thread panicked");

    match result {
        Ok(count) => {
            eprintln!("cljrs-stdlib build.rs: pre-lowered {count} clojure.core arities to IR");
        }
        Err(e) => {
            // Don't fail the build — just warn. The runtime will fall back to
            // eager lowering or tree-walking if no prebuilt IR is available.
            eprintln!("cljrs-stdlib build.rs: IR pre-lowering failed: {e}");
            eprintln!(
                "cljrs-stdlib build.rs: writing empty bundle (runtime will use tree-walking)"
            );
            let empty = cljrs_ir::IrBundle::new();
            let bytes = cljrs_ir::serialize_bundle(&empty).unwrap();
            std::fs::write(out_dir.join("core_ir.bin"), &bytes).unwrap();
        }
    }
}

#[cfg(feature = "prebuild-ir")]
fn prebuild_core(output: &std::path::Path) -> Result<usize, String> {
    use std::sync::Arc;

    // Boot a minimal env with the tree-walking interpreter only (no IR dispatch).
    // We use cljrs_interp directly to avoid the IR hooks — we're *producing* the
    // IR, not consuming it.
    let globals = cljrs_interp::standard_env_minimal(None, None, None);

    // Register compiler sources so they can be required.
    cljrs_eval::register_compiler_sources(&globals);

    let mut env = cljrs_eval::Env::new(globals.clone(), "user");

    // Load the compiler namespaces (anf, ir, known).
    if !cljrs_eval::ensure_compiler_loaded(&globals, &mut env) {
        return Err("failed to load compiler namespaces".to_string());
    }

    // Also load the optimize namespace (which transitively requires escape) so
    // we can run escape analysis on the lowered IR — that's what inserts the
    // RegionStart/RegionAlloc instructions consumed by the IR interpreter.
    {
        let span = cljrs_types::span::Span::new(Arc::new("<prebuild>".to_string()), 0, 0, 1, 1);
        let require_form = cljrs_reader::Form::new(
            cljrs_reader::form::FormKind::List(vec![
                cljrs_reader::Form::new(
                    cljrs_reader::form::FormKind::Symbol("require".into()),
                    span.clone(),
                ),
                cljrs_reader::Form::new(
                    cljrs_reader::form::FormKind::Quote(Box::new(cljrs_reader::Form::new(
                        cljrs_reader::form::FormKind::Symbol("cljrs.compiler.optimize".into()),
                        span.clone(),
                    ))),
                    span,
                ),
            ]),
            cljrs_types::span::Span::new(Arc::new("<prebuild>".to_string()), 0, 0, 1, 1),
        );
        cljrs_eval::eval(&require_form, &mut env)
            .map_err(|e| format!("failed to require cljrs.compiler.optimize: {e:?}"))?;
    }

    // Walk clojure.core and lower every function arity.
    let mut bundle = cljrs_ir::IrBundle::new();
    let mut count = 0usize;
    let mut fail_count = 0usize;
    let mut first_failure: Option<String> = None;

    let var_entries: Vec<(Arc<str>, cljrs_value::Value)> = {
        let ns_map = globals.namespaces.read().unwrap();
        let ns = ns_map
            .get("clojure.core")
            .ok_or("clojure.core namespace not found")?;
        let interns = ns.get().interns.lock().unwrap();
        interns
            .iter()
            .map(|(name, var)| {
                let val = var.get().deref().unwrap_or(cljrs_value::Value::Nil);
                (name.clone(), val)
            })
            .collect()
    };

    for (var_name, val) in &var_entries {
        let f = match val {
            cljrs_value::Value::Fn(gc_fn) => gc_fn.get().clone(),
            _ => continue,
        };
        if f.is_macro {
            continue;
        }

        let ns_arc: Arc<str> = Arc::from("clojure.core");
        for arity in &f.arities {
            let key = if arity.rest_param.is_some() {
                format!("clojure.core/{var_name}:{}+", arity.params.len())
            } else {
                format!("clojure.core/{var_name}:{}", arity.params.len())
            };

            match cljrs_eval::lower::lower_and_optimize_arity(
                f.name.as_deref(),
                &arity.params,
                arity.rest_param.as_ref(),
                &arity.body,
                &ns_arc,
                &mut env,
            ) {
                Ok(ir_func) => {
                    bundle.insert(key, ir_func);
                    count += 1;
                }
                Err(e) => {
                    fail_count += 1;
                    if first_failure.is_none() {
                        first_failure = Some(format!("{key}: {e}"));
                    }
                }
            }
        }
    }

    let bytes =
        cljrs_ir::serialize_bundle(&bundle).map_err(|e| format!("serialization failed: {e}"))?;
    std::fs::write(output, &bytes)
        .map_err(|e| format!("failed to write {}: {e}", output.display()))?;

    if fail_count > 0 {
        eprintln!(
            "cljrs-stdlib build.rs: {fail_count} arities failed to lower+optimize (first: {})",
            first_failure.as_deref().unwrap_or("?"),
        );
    }

    Ok(count)
}

#[cfg(not(feature = "prebuild-ir"))]
fn main() {
    // No-op when prebuild-ir feature is disabled.
}
