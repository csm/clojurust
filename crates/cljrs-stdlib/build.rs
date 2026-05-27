//! Build script: pre-lower clojure.core and the cljrs.compiler.* namespaces
//! to IR and write the bundles to OUT_DIR so they can be embedded via
//! `include_bytes!`.
//!
//! Only runs when the `prebuild-ir` feature is enabled.

#[cfg(feature = "prebuild-ir")]
fn main() {
    use std::path::PathBuf;

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    // Re-run if bootstrap sources change.
    println!("cargo::rerun-if-changed=../cljrs-builtins/src/bootstrap.cljrs");
    println!("cargo::rerun-if-changed=../cljrs-ir/src/cljrs/compiler/anf.cljrs");
    println!("cargo::rerun-if-changed=../cljrs-ir/src/cljrs/compiler/ir.cljrs");
    println!("cargo::rerun-if-changed=../cljrs-ir/src/cljrs/compiler/known.cljrs");
    println!("cargo::rerun-if-changed=../cljrs-ir/src/cljrs/compiler/escape.cljrs");
    println!("cargo::rerun-if-changed=../cljrs-ir/src/cljrs/compiler/optimize.cljrs");

    // The Clojure compiler uses deep recursion; run on a large-stack thread.
    let result = std::thread::Builder::new()
        .name("ir-prebuild".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || prebuild_all(&out_dir))
        .expect("failed to spawn prebuild thread")
        .join()
        .expect("prebuild thread panicked");

    match result {
        Ok((core_count, compiler_count)) => {
            eprintln!(
                "cljrs-stdlib build.rs: pre-lowered {core_count} clojure.core arities \
                 and {compiler_count} compiler arities to IR"
            );
        }
        Err(e) => {
            // Don't fail the build — just warn. The runtime will fall back to
            // eager lowering or tree-walking if no prebuilt IR is available.
            eprintln!("cljrs-stdlib build.rs: IR pre-lowering failed: {e}");
            eprintln!(
                "cljrs-stdlib build.rs: writing empty bundles (runtime will use tree-walking)"
            );
            let empty_bytes = cljrs_ir::serialize_bundle(&cljrs_ir::IrBundle::new()).unwrap();
            let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
            std::fs::write(out_dir.join("core_ir.bin"), &empty_bytes).unwrap();
            std::fs::write(out_dir.join("compiler_ir.bin"), &empty_bytes).unwrap();
        }
    }
}

#[cfg(feature = "prebuild-ir")]
fn prebuild_all(out_dir: &std::path::Path) -> Result<(usize, usize), String> {
    // Boot a minimal env with the tree-walking interpreter only (no IR dispatch).
    // We use cljrs_interp directly to avoid the IR hooks — we're *producing* the
    // IR, not consuming it.
    let globals = cljrs_interp::standard_env_minimal(None, None, None);

    // Register compiler sources so they can be required.
    cljrs_eval::register_compiler_sources(&globals);

    let mut env = cljrs_eval::Env::new(globals.clone(), "user");

    // Load the compiler namespaces (ir, known, anf, escape, optimize).
    if !cljrs_eval::ensure_compiler_loaded(&globals, &mut env) {
        return Err("failed to load compiler namespaces".to_string());
    }

    // ensure_compiler_loaded loads all 5 namespaces including optimize, so
    // escape analysis is available for the optimize pass.

    // ── clojure.core ──────────────────────────────────────────────────────────

    let (core_count, core_fails, core_bundle) =
        lower_ns_to_bundle("clojure.core", &globals, &mut env);

    if core_fails > 0 {
        eprintln!(
            "cljrs-stdlib build.rs: {core_fails} clojure.core arities failed to lower+optimize"
        );
    }

    // Diagnostic: count region instructions so changes to the optimize pass
    // have a visible effect at build time.
    let (region_insts, fns_with_regions) = count_region_insts(&core_bundle);
    eprintln!(
        "cljrs-stdlib build.rs: core bundle contains {region_insts} region instructions \
         across {fns_with_regions} functions"
    );

    let core_bytes = cljrs_ir::serialize_bundle(&core_bundle)
        .map_err(|e| format!("core bundle serialization failed: {e}"))?;
    std::fs::write(out_dir.join("core_ir.bin"), &core_bytes)
        .map_err(|e| format!("failed to write core_ir.bin: {e}"))?;

    // ── cljrs.compiler.* ─────────────────────────────────────────────────────

    const COMPILER_NS: &[&str] = &[
        "cljrs.compiler.ir",
        "cljrs.compiler.known",
        "cljrs.compiler.anf",
        "cljrs.compiler.escape",
        "cljrs.compiler.optimize",
    ];

    let mut compiler_bundle = cljrs_ir::IrBundle::new();
    let mut compiler_count = 0usize;
    let mut compiler_fails = 0usize;

    for ns_name in COMPILER_NS {
        let (count, fails, bundle) = lower_ns_to_bundle(ns_name, &globals, &mut env);
        compiler_count += count;
        compiler_fails += fails;
        for (key, ir_func) in bundle.functions {
            compiler_bundle.insert(key, ir_func);
        }
    }

    if compiler_fails > 0 {
        eprintln!(
            "cljrs-stdlib build.rs: {compiler_fails} compiler arities skipped \
             (destructured params or unsupported forms)"
        );
    }

    let (compiler_region_insts, compiler_fns_with_regions) = count_region_insts(&compiler_bundle);
    eprintln!(
        "cljrs-stdlib build.rs: compiler bundle contains {compiler_region_insts} region \
         instructions across {compiler_fns_with_regions} functions"
    );

    let compiler_bytes = cljrs_ir::serialize_bundle(&compiler_bundle)
        .map_err(|e| format!("compiler bundle serialization failed: {e}"))?;
    std::fs::write(out_dir.join("compiler_ir.bin"), &compiler_bytes)
        .map_err(|e| format!("failed to write compiler_ir.bin: {e}"))?;

    Ok((core_count, compiler_count))
}

/// Lower all non-macro function arities in `ns_name` and return
/// `(succeeded, failed, bundle)`.  Arities with destructured parameters are
/// skipped; they would produce broken IR because `lower-fn-body` only knows
/// the gensym placeholder names, not the binding names introduced by
/// `bind_fn_params`.
#[cfg(feature = "prebuild-ir")]
fn lower_ns_to_bundle(
    ns_name: &str,
    globals: &std::sync::Arc<cljrs_eval::GlobalEnv>,
    env: &mut cljrs_eval::Env,
) -> (usize, usize, cljrs_ir::IrBundle) {
    use std::sync::Arc;

    let mut bundle = cljrs_ir::IrBundle::new();
    let mut count = 0usize;
    let mut fail_count = 0usize;

    let var_entries: Vec<(Arc<str>, cljrs_value::Value)> = {
        let ns_map = globals.namespaces.read().unwrap();
        let Some(ns) = ns_map.get(ns_name) else {
            eprintln!("cljrs-stdlib build.rs: namespace {ns_name} not found, skipping");
            return (0, 0, bundle);
        };
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

        let ns_arc: Arc<str> = Arc::from(ns_name);
        for arity in &f.arities {
            // Skip arities with destructured params: lower-fn-body only binds
            // the gensym placeholder names, so any reference to the original
            // binding names in the body would become LoadGlobal, producing
            // broken IR at runtime.
            if !arity.destructure_params.is_empty() || arity.destructure_rest.is_some() {
                fail_count += 1;
                continue;
            }

            let key = if arity.rest_param.is_some() {
                format!("{ns_name}/{var_name}:{}+", arity.params.len())
            } else {
                format!("{ns_name}/{var_name}:{}", arity.params.len())
            };

            match cljrs_eval::lower::lower_and_optimize_arity(
                f.name.as_deref(),
                &arity.params,
                arity.rest_param.as_ref(),
                &arity.body,
                &ns_arc,
                env,
                f.is_async,
            ) {
                Ok(ir_func) => {
                    bundle.insert(key, ir_func);
                    count += 1;
                }
                Err(_) => {
                    fail_count += 1;
                }
            }
        }
    }

    (count, fail_count, bundle)
}

#[cfg(feature = "prebuild-ir")]
fn count_region_insts(bundle: &cljrs_ir::IrBundle) -> (usize, usize) {
    let mut region_inst_count = 0usize;
    let mut fns_with_regions = 0usize;
    for ir_func in bundle.functions.values() {
        let mut had_region = false;
        for block in &ir_func.blocks {
            for inst in &block.insts {
                if matches!(
                    inst,
                    cljrs_ir::Inst::RegionStart(..)
                        | cljrs_ir::Inst::RegionAlloc(..)
                        | cljrs_ir::Inst::RegionEnd(..)
                ) {
                    region_inst_count += 1;
                    had_region = true;
                }
            }
        }
        if had_region {
            fns_with_regions += 1;
        }
    }
    (region_inst_count, fns_with_regions)
}

#[cfg(not(feature = "prebuild-ir"))]
fn main() {
    // Tell Cargo not to re-run this script unless build.rs itself changes.
    // Without this, Cargo re-runs any build script that emits no
    // rerun-if-changed directives on every single build.
    println!("cargo::rerun-if-changed=build.rs");
}
