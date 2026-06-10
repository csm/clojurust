//! Rust-native IR lowering orchestration.
//!
//! Calls the Rust `cljrs_ir::lower` pipeline directly (no Clojure interpreter
//! round-trip).  Macro expansion still runs through the interpreter since
//! macros are user-defined Clojure functions.

use std::sync::Arc;

use cljrs_ir::IrFunction;
use cljrs_reader::Form;

use cljrs_env::env::Env;

// ── Error type ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum LowerError {
    /// The Rust lowering function failed.
    LowerFailed(String),
    /// IR conversion failed (kept for compatibility; no longer used internally).
    ConvertFailed(String),
    /// The compiler namespaces are not loaded yet (no longer used with Rust lowering,
    /// kept for callers that may still pattern-match on this variant).
    NotReady,
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LowerError::NotReady => write!(f, "compiler not ready"),
            LowerError::LowerFailed(msg) => write!(f, "lowering failed: {msg}"),
            LowerError::ConvertFailed(msg) => write!(f, "IR conversion failed: {msg}"),
        }
    }
}

// ── Public entry point ──────────────────────────────────────────────────────

/// Lower a function arity's body to IR using the native Rust compiler pipeline.
///
/// `destructure_params` carries the original destructuring patterns for any
/// parameters the interpreter replaced with gensym placeholders (each paired
/// with its index into `params`); `destructure_rest` is the rest parameter's
/// pattern, if it is itself destructured.  Both are expanded into explicit
/// bindings in the IR prologue.
#[allow(clippy::too_many_arguments)]
pub fn lower_arity(
    name: Option<&str>,
    params: &[Arc<str>],
    rest_param: Option<&Arc<str>>,
    destructure_params: &[(usize, Form)],
    destructure_rest: Option<&Form>,
    body: &[Form],
    ns: &Arc<str>,
    env: &mut Env,
    is_async: bool,
) -> Result<IrFunction, LowerError> {
    lower_arity_inner(
        name,
        params,
        rest_param,
        destructure_params,
        destructure_rest,
        body,
        ns,
        env,
        false,
        is_async,
    )
    .map(|(ir, _)| ir)
}

/// Like [`lower_arity`], but also runs the region-optimization pass.
#[allow(clippy::too_many_arguments)]
pub fn lower_and_optimize_arity(
    name: Option<&str>,
    params: &[Arc<str>],
    rest_param: Option<&Arc<str>>,
    destructure_params: &[(usize, Form)],
    destructure_rest: Option<&Form>,
    body: &[Form],
    ns: &Arc<str>,
    env: &mut Env,
    is_async: bool,
) -> Result<IrFunction, LowerError> {
    lower_and_optimize_arity_tracked(
        name,
        params,
        rest_param,
        destructure_params,
        destructure_rest,
        body,
        ns,
        env,
        is_async,
    )
    .map(|(ir, _)| ir)
}

/// Like [`lower_and_optimize_arity`], but also returns the `(ns, name)` set
/// of cross-defn externals (see [`crate::defn_registry`]) the optimizer
/// consulted — the caller must register those as invalidation dependencies.
#[allow(clippy::too_many_arguments)]
pub fn lower_and_optimize_arity_tracked(
    name: Option<&str>,
    params: &[Arc<str>],
    rest_param: Option<&Arc<str>>,
    destructure_params: &[(usize, Form)],
    destructure_rest: Option<&Form>,
    body: &[Form],
    ns: &Arc<str>,
    env: &mut Env,
    is_async: bool,
) -> Result<(IrFunction, Vec<(Arc<str>, Arc<str>)>), LowerError> {
    lower_arity_inner(
        name,
        params,
        rest_param,
        destructure_params,
        destructure_rest,
        body,
        ns,
        env,
        true,
        is_async,
    )
}

#[allow(clippy::too_many_arguments)]
fn lower_arity_inner(
    name: Option<&str>,
    params: &[Arc<str>],
    rest_param: Option<&Arc<str>>,
    destructure_params: &[(usize, Form)],
    destructure_rest: Option<&Form>,
    body: &[Form],
    ns: &Arc<str>,
    env: &mut Env,
    do_optimize: bool,
    is_async: bool,
) -> Result<(IrFunction, Vec<(Arc<str>, Arc<str>)>), LowerError> {
    cljrs_logging::feat_debug!(
        "lower",
        "lowering {:?}/{:?} optimize? {}",
        ns,
        name,
        do_optimize
    );

    // Macro expansion still requires the interpreter.
    // Guard against re-entrant lowering during macro expansion.
    use crate::apply::IR_LOWERING_ACTIVE;
    let was_active = IR_LOWERING_ACTIVE.get();
    IR_LOWERING_ACTIVE.set(true);

    let expanded_body: Vec<Form> = body
        .iter()
        .map(|f| cljrs_interp::macros::macroexpand_all(f, env).unwrap_or_else(|_| f.clone()))
        .collect();

    IR_LOWERING_ACTIVE.with(|c| c.set(was_active));

    // Build the flat params list (includes rest param as last element if present).
    let mut all_params: Vec<Arc<str>> = params.to_vec();
    if let Some(rest) = rest_param {
        all_params.push(rest.clone());
    }

    // Build the combined destructuring list, indexed into `all_params`.  Fixed
    // params keep their recorded index; the rest param, if destructured, sits at
    // the final position (`params.len()`).
    let mut destructures: Vec<(usize, Form)> = destructure_params.to_vec();
    if let Some(rest_pat) = destructure_rest {
        destructures.push((params.len(), rest_pat.clone()));
    }

    let ir = cljrs_ir::lower::lower_fn_body_destructured(
        name,
        ns,
        &all_params,
        &destructures,
        &expanded_body,
        is_async,
    )
    .map_err(|e| LowerError::LowerFailed(format!("{e:?}")))?;

    if !do_optimize {
        return Ok((ir, Vec::new()));
    }

    // Make previously-lowered defns this function references visible to
    // escape analysis and stage-4 cross-function region promotion (the
    // script/REPL counterpart of AOT's whole-program lowering).
    let referenced = referenced_globals(&ir);
    let externals = crate::defn_registry::externals_for(globals_id(env), &referenced);
    let (ir, used) = cljrs_ir::lower::optimize_with_externals(ir, &externals);
    Ok((ir, used.into_iter().collect()))
}

/// Identity of the `GlobalEnv` behind `env`, used to scope the cross-defn
/// registry per isolate.
pub(crate) fn globals_id(env: &Env) -> usize {
    Arc::as_ptr(&env.globals) as usize
}

/// Collect every `(ns, name)` pair the IR tree loads as a global — the
/// candidate set of cross-defn externals.
fn referenced_globals(ir: &IrFunction) -> std::collections::HashSet<(Arc<str>, Arc<str>)> {
    use cljrs_ir::Inst;
    let mut out = std::collections::HashSet::new();
    fn walk(f: &IrFunction, out: &mut std::collections::HashSet<(Arc<str>, Arc<str>)>) {
        for block in &f.blocks {
            for inst in block.phis.iter().chain(block.insts.iter()) {
                if let Inst::LoadGlobal(_, ns, name) | Inst::LoadVar(_, ns, name) = inst {
                    out.insert((ns.clone(), name.clone()));
                }
            }
        }
        for sub in &f.subfunctions {
            walk(sub, out);
        }
    }
    walk(ir, &mut out);
    out
}
