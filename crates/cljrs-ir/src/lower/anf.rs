//! ANF lowering: `Form` AST → `IrFunction`.
//!
//! Mirrors `cljrs.compiler.anf`. Receives fully macro-expanded `Form` nodes
//! and produces a well-formed SSA IR in A-normal form.

use std::sync::Arc;

use cljrs_reader::form::{Form, FormKind};

use crate::{BlockId, ClosureTemplate, Const, Inst, IrFunction, KnownFn, Terminator, VarId};

use super::context::{LowerCtx, fresh_global_name_id};
use super::known::{resolve_known_fn, strip_ns_prefix};

// ── Error type ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum LowerError {
    UnsupportedForm(String),
    MalformedSpecialForm(String),
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LowerError::UnsupportedForm(s) => write!(f, "unsupported form in ANF lowering: {s}"),
            LowerError::MalformedSpecialForm(s) => {
                write!(f, "malformed special form in ANF lowering: {s}")
            }
        }
    }
}

impl std::error::Error for LowerError {}

type R = Result<VarId, LowerError>;

// ── Public entry point ────────────────────────────────────────────────────────

/// Lower a function arity's body to an `IrFunction`.
///
/// `name`   — function name (for diagnostics), or `None` for anonymous.
/// `ns`     — current Clojure namespace name.
/// `params` — flat parameter name list, including the rest param if variadic
///            (the rest param is the last element).
/// `body`   — sequence of already macro-expanded body forms (implicit `do`).
pub fn lower_fn_body(
    name: Option<&str>,
    ns: &str,
    params: &[Arc<str>],
    body: &[Form],
) -> Result<IrFunction, LowerError> {
    let mut ctx = LowerCtx::new(name.map(Arc::from), Arc::from(ns));

    // Bind each param to a fresh VarId.
    let mut bound_params: Vec<(Arc<str>, VarId)> = Vec::with_capacity(params.len());
    for pname in params {
        let id = ctx.fresh_var();
        ctx.bind_local(pname.clone(), id);
        bound_params.push((pname.clone(), id));
    }

    // Lower the body; emit a Return terminator.
    let result = lower_body(&mut ctx, body)?;
    ctx.finish_block(Terminator::Return(result));

    Ok(ctx.build(bound_params))
}

// ── Body lowering ─────────────────────────────────────────────────────────────

fn lower_body(ctx: &mut LowerCtx, forms: &[Form]) -> R {
    if forms.is_empty() {
        return Ok(ctx.emit_const(Const::Nil));
    }
    let mut last = VarId(0);
    for form in forms {
        last = lower_form(ctx, form)?;
    }
    Ok(last)
}

// ── Form dispatch ─────────────────────────────────────────────────────────────

fn lower_form(ctx: &mut LowerCtx, form: &Form) -> R {
    match &form.kind {
        FormKind::Nil => Ok(ctx.emit_const(Const::Nil)),
        FormKind::Bool(b) => Ok(ctx.emit_const(Const::Bool(*b))),
        FormKind::Int(n) => Ok(ctx.emit_const(Const::Long(*n))),
        FormKind::BigInt(s) => {
            // Parse as i64 if possible, otherwise error.
            let n: i64 = s.parse().unwrap_or(0);
            Ok(ctx.emit_const(Const::Long(n)))
        }
        FormKind::Float(f) => Ok(ctx.emit_const(Const::Double(*f))),
        FormKind::BigDecimal(s) => {
            let f: f64 = s.parse().unwrap_or(0.0);
            Ok(ctx.emit_const(Const::Double(f)))
        }
        FormKind::Ratio(s) => {
            // Evaluate a/b ratio as f64.
            if let Some(pos) = s.find('/') {
                let num: f64 = s[..pos].parse().unwrap_or(0.0);
                let den: f64 = s[pos + 1..].parse().unwrap_or(1.0);
                Ok(ctx.emit_const(Const::Double(if den != 0.0 { num / den } else { 0.0 })))
            } else {
                let f: f64 = s.parse().unwrap_or(0.0);
                Ok(ctx.emit_const(Const::Double(f)))
            }
        }
        FormKind::Char(c) => Ok(ctx.emit_const(Const::Char(*c))),
        FormKind::Str(s) => Ok(ctx.emit_const(Const::Str(Arc::from(s.as_str())))),
        FormKind::Regex(s) => Ok(ctx.emit_const(Const::Str(Arc::from(s.as_str())))),
        FormKind::Symbolic(f) => Ok(ctx.emit_const(Const::Double(*f))),
        FormKind::Keyword(s) => {
            // Strip namespace prefix for keyword constants (mirrors `(name kw)`).
            let local_name = kw_local_name(s);
            Ok(ctx.emit_const(Const::Keyword(Arc::from(local_name))))
        }
        FormKind::AutoKeyword(s) => Ok(ctx.emit_const(Const::Keyword(Arc::from(s.as_str())))),
        FormKind::Symbol(s) => lower_symbol(ctx, s),
        FormKind::Vector(elems) => {
            let vars: Result<Vec<VarId>, _> = elems.iter().map(|e| lower_form(ctx, e)).collect();
            let vars = vars?;
            let dst = ctx.fresh_var();
            ctx.emit(Inst::AllocVector(dst, vars));
            Ok(dst)
        }
        FormKind::Map(pairs) => {
            // Flat key/value pairs.
            let mut kv_pairs: Vec<(VarId, VarId)> = Vec::with_capacity(pairs.len() / 2);
            let mut i = 0;
            while i + 1 < pairs.len() {
                let k = lower_form(ctx, &pairs[i])?;
                let v = lower_form(ctx, &pairs[i + 1])?;
                kv_pairs.push((k, v));
                i += 2;
            }
            let dst = ctx.fresh_var();
            ctx.emit(Inst::AllocMap(dst, kv_pairs));
            Ok(dst)
        }
        FormKind::Set(elems) => {
            let vars: Result<Vec<VarId>, _> = elems.iter().map(|e| lower_form(ctx, e)).collect();
            let vars = vars?;
            let dst = ctx.fresh_var();
            ctx.emit(Inst::AllocSet(dst, vars));
            Ok(dst)
        }
        FormKind::List(parts) => {
            if parts.is_empty() {
                let dst = ctx.fresh_var();
                ctx.emit(Inst::AllocList(dst, vec![]));
                return Ok(dst);
            }
            lower_list(ctx, parts)
        }
        FormKind::Quote(inner) => lower_quote(ctx, inner),
        FormKind::Deref(inner) => {
            let src = lower_form(ctx, inner)?;
            let dst = ctx.fresh_var();
            ctx.emit(Inst::Deref(dst, src));
            Ok(dst)
        }
        FormKind::Var(inner) => {
            // `#'symbol` — load the Var object.
            if let FormKind::Symbol(s) = &inner.kind {
                let (var_ns, var_name) = split_sym(s, ctx.ns());
                let dst = ctx.fresh_var();
                ctx.emit(Inst::LoadVar(dst, var_ns, var_name));
                Ok(dst)
            } else {
                Err(LowerError::UnsupportedForm(format!(
                    "#' expects symbol, got {:?}",
                    inner.kind
                )))
            }
        }
        FormKind::Meta(_, inner) => lower_form(ctx, inner),
        // These should have been expanded before reaching the lowering pass.
        FormKind::SyntaxQuote(_)
        | FormKind::Unquote(_)
        | FormKind::UnquoteSplice(_)
        | FormKind::AnonFn(_) => Err(LowerError::UnsupportedForm(format!(
            "un-expanded reader macro: {:?}",
            form.kind
        ))),
        FormKind::ReaderCond { .. } => Err(LowerError::UnsupportedForm(
            "un-expanded reader conditional".to_string(),
        )),
        FormKind::TaggedLiteral(tag, _) => Err(LowerError::UnsupportedForm(format!(
            "tagged literal #{tag} not supported in IR lowering"
        ))),
    }
}

// ── List dispatch ─────────────────────────────────────────────────────────────

fn lower_list(ctx: &mut LowerCtx, parts: &[Form]) -> R {
    let head = &parts[0];
    let args = &parts[1..];

    // Keyword-as-function: (:key m) → (get m :key)
    if let FormKind::Keyword(s) = &head.kind {
        return match args.len() {
            1 => {
                let m = lower_form(ctx, &args[0])?;
                let local = kw_local_name(s);
                let k = ctx.emit_const(Const::Keyword(Arc::from(local)));
                let dst = ctx.fresh_var();
                ctx.emit(Inst::CallKnown(dst, KnownFn::Get, vec![m, k]));
                Ok(dst)
            }
            2 => {
                // (:key m default) — fall through to dynamic call
                let callee = lower_form(ctx, head)?;
                let arg_vars: Result<Vec<VarId>, _> =
                    args.iter().map(|a| lower_form(ctx, a)).collect();
                let arg_vars = arg_vars?;
                let dst = ctx.fresh_var();
                ctx.emit(Inst::Call(dst, callee, arg_vars));
                Ok(dst)
            }
            n => Err(LowerError::MalformedSpecialForm(format!(
                "keyword lookup expects 1 or 2 args, got {n}"
            ))),
        };
    }

    // Non-symbol head (lambda, etc.) — generic call.
    let FormKind::Symbol(sym) = &head.kind else {
        return lower_call(ctx, head, args);
    };

    match sym.as_str() {
        "if" => lower_if(ctx, args),
        "do" => lower_body(ctx, args),
        "let" | "let*" => lower_let(ctx, args),
        "loop" | "loop*" => lower_loop(ctx, args),
        "recur" => lower_recur(ctx, args),
        "def" => lower_def(ctx, args),
        "fn" | "fn*" => lower_fn(ctx, args),
        "defn" => lower_defn(ctx, args),
        "quote" => {
            if args.len() != 1 {
                return Err(LowerError::MalformedSpecialForm(
                    "quote expects 1 argument".into(),
                ));
            }
            lower_quote(ctx, &args[0])
        }
        "throw" => lower_throw(ctx, args),
        "set!" => lower_set_bang(ctx, args),
        "and" => lower_and(ctx, args),
        "or" => lower_or(ctx, args),
        "try" => lower_try(ctx, args),
        "binding" => lower_binding(ctx, args),
        "letfn" => lower_letfn(ctx, args),
        "with-out-str" => lower_with_out_str(ctx, args),
        // Module-level forms should never reach here.
        "ns" | "require" | "in-ns" | "alias" | "load-file" => Err(LowerError::UnsupportedForm(
            format!("{} is a module-level form and cannot be compiled", sym),
        )),
        // These should be expanded before lowering.
        "defmacro" | "defonce" => Err(LowerError::UnsupportedForm(format!(
            "{sym} should be expanded before ANF lowering"
        ))),
        // Protocol/record forms and other constructs fall through to a
        // generic function call.
        _ => lower_call(ctx, head, args),
    }
}

// ── Special forms ─────────────────────────────────────────────────────────────

fn lower_if(ctx: &mut LowerCtx, args: &[Form]) -> R {
    if args.is_empty() || args.len() > 3 {
        return Err(LowerError::MalformedSpecialForm(
            "if expects 1-3 arguments".into(),
        ));
    }
    let test = lower_form(ctx, &args[0])?;
    let then_block = ctx.fresh_block();
    let else_block = ctx.fresh_block();
    let join_block = ctx.fresh_block();

    ctx.finish_block(Terminator::Branch {
        cond: test,
        then_block,
        else_block,
    });

    // Then branch.
    ctx.start_block(then_block);
    let then_val = if args.len() >= 2 {
        lower_form(ctx, &args[1])?
    } else {
        ctx.emit_const(Const::Nil)
    };
    let then_exit = ctx.current_block_id();
    ctx.finish_block(Terminator::Jump(join_block));

    // Else branch.
    ctx.start_block(else_block);
    let else_val = if args.len() >= 3 {
        lower_form(ctx, &args[2])?
    } else {
        ctx.emit_const(Const::Nil)
    };
    let else_exit = ctx.current_block_id();
    ctx.finish_block(Terminator::Jump(join_block));

    // Join with phi.
    ctx.start_block(join_block);
    let result = ctx.fresh_var();
    ctx.emit_phi(result, vec![(then_exit, then_val), (else_exit, else_val)]);
    Ok(result)
}

// ── Destructuring ─────────────────────────────────────────────────────────────

fn lower_destructure_binding(
    ctx: &mut LowerCtx,
    pattern: &Form,
    val: VarId,
) -> Result<(), LowerError> {
    match &pattern.kind {
        FormKind::Symbol(s) => {
            ctx.bind_local(Arc::from(s.as_str()), val);
            Ok(())
        }
        FormKind::Vector(pats) => lower_destructure_sequential(ctx, pats, val),
        FormKind::Map(pairs) => lower_destructure_associative(ctx, pairs, val),
        _ => Err(LowerError::UnsupportedForm(format!(
            "unsupported binding pattern: {:?}",
            pattern.kind
        ))),
    }
}

fn lower_destructure_sequential(
    ctx: &mut LowerCtx,
    pattern: &[Form],
    val: VarId,
) -> Result<(), LowerError> {
    let n = pattern.len();
    let mut i = 0;
    let mut pos_idx: usize = 0;

    while i < n {
        let p = &pattern[i];
        match &p.kind {
            FormKind::Symbol(s) if s == "&" => {
                // Rest pattern
                i += 1;
                if i >= n {
                    return Err(LowerError::MalformedSpecialForm(
                        "& must be followed by a pattern".into(),
                    ));
                }
                let rest_var = lower_emit_rest_from(ctx, val, pos_idx);
                lower_destructure_binding(ctx, &pattern[i], rest_var)?;
                i += 1;
                // Optional `:as alias` after rest
                if i + 1 < n
                    && let FormKind::Keyword(k) = &pattern[i].kind
                    && k == "as"
                {
                    lower_destructure_binding(ctx, &pattern[i + 1], val)?;
                    // don't advance i — the outer loop will stop
                }
            }
            FormKind::Keyword(k) if k == "as" => {
                i += 1;
                if i < n {
                    lower_destructure_binding(ctx, &pattern[i], val)?;
                }
                i += 1;
            }
            _ => {
                let item = lower_emit_nth(ctx, val, pos_idx as i64);
                lower_destructure_binding(ctx, p, item)?;
                pos_idx += 1;
                i += 1;
            }
        }
    }
    Ok(())
}

fn lower_emit_nth(ctx: &mut LowerCtx, val: VarId, idx: i64) -> VarId {
    let idx_var = ctx.emit_const(Const::Long(idx));
    let dst = ctx.fresh_var();
    ctx.emit(Inst::CallKnown(dst, KnownFn::Nth, vec![val, idx_var]));
    dst
}

fn lower_emit_rest_from(ctx: &mut LowerCtx, val: VarId, idx: usize) -> VarId {
    let mut current = val;
    for _ in 0..idx {
        let dst = ctx.fresh_var();
        ctx.emit(Inst::CallKnown(dst, KnownFn::Rest, vec![current]));
        current = dst;
    }
    // Wrap in `seq` to normalize nil for empty rest.
    let dst = ctx.fresh_var();
    ctx.emit(Inst::CallKnown(dst, KnownFn::Seq, vec![current]));
    dst
}

fn lower_destructure_associative(
    ctx: &mut LowerCtx,
    pairs: &[Form],
    val: VarId,
) -> Result<(), LowerError> {
    // Collect `:or` defaults first (flat map pairs).
    let mut defaults: Vec<(String, Form)> = Vec::new();
    let mut i = 0;
    while i + 1 < pairs.len() {
        if let FormKind::Keyword(k) = &pairs[i].kind
            && k == "or"
            && let FormKind::Map(or_pairs) = &pairs[i + 1].kind
        {
            let mut j = 0;
            while j + 1 < or_pairs.len() {
                if let FormKind::Symbol(sym) = &or_pairs[j].kind {
                    defaults.push((sym.clone(), or_pairs[j + 1].clone()));
                }
                j += 2;
            }
        }
        i += 2;
    }

    // Second pass: process bindings.
    i = 0;
    while i + 1 < pairs.len() {
        let k = &pairs[i];
        let v = &pairs[i + 1];

        match &k.kind {
            FormKind::Keyword(kname) if kname == "keys" => {
                if let FormKind::Vector(syms) = &v.kind {
                    for sym_form in syms {
                        if let FormKind::Symbol(sym) = &sym_form.kind {
                            let kw_var = ctx.emit_const(Const::Keyword(Arc::from(sym.as_str())));
                            let got = lower_emit_get(ctx, val, kw_var);
                            let final_var = apply_default_if_nil(ctx, got, sym, &defaults)?;
                            ctx.bind_local(Arc::from(sym.as_str()), final_var);
                        }
                    }
                }
            }
            FormKind::Keyword(kname) if kname == "strs" => {
                if let FormKind::Vector(syms) = &v.kind {
                    for sym_form in syms {
                        if let FormKind::Symbol(sym) = &sym_form.kind {
                            let str_var = ctx.emit_const(Const::Str(Arc::from(sym.as_str())));
                            let got = lower_emit_get(ctx, val, str_var);
                            let final_var = apply_default_if_nil(ctx, got, sym, &defaults)?;
                            ctx.bind_local(Arc::from(sym.as_str()), final_var);
                        }
                    }
                }
            }
            FormKind::Keyword(kname) if kname == "syms" => {
                if let FormKind::Vector(syms) = &v.kind {
                    for sym_form in syms {
                        if let FormKind::Symbol(sym) = &sym_form.kind {
                            let sym_var = ctx.emit_const(Const::Symbol(Arc::from(sym.as_str())));
                            let got = lower_emit_get(ctx, val, sym_var);
                            let final_var = apply_default_if_nil(ctx, got, sym, &defaults)?;
                            ctx.bind_local(Arc::from(sym.as_str()), final_var);
                        }
                    }
                }
            }
            FormKind::Keyword(kname) if kname == "as" => {
                if let FormKind::Symbol(sym) = &v.kind {
                    ctx.bind_local(Arc::from(sym.as_str()), val);
                }
            }
            FormKind::Keyword(kname) if kname == "or" => {
                // Already collected above; skip.
            }
            _ => {
                // Regular {binding-form lookup-key}
                let key_var = lower_form(ctx, v)?;
                let got = lower_emit_get(ctx, val, key_var);
                // If binding target is a symbol, try to apply default.
                let final_var = if let FormKind::Symbol(sym) = &k.kind {
                    apply_default_if_nil(ctx, got, sym, &defaults)?
                } else {
                    got
                };
                lower_destructure_binding(ctx, k, final_var)?;
            }
        }
        i += 2;
    }
    Ok(())
}

fn lower_emit_get(ctx: &mut LowerCtx, map: VarId, key: VarId) -> VarId {
    let dst = ctx.fresh_var();
    ctx.emit(Inst::CallKnown(dst, KnownFn::Get, vec![map, key]));
    dst
}

fn apply_default_if_nil(
    ctx: &mut LowerCtx,
    got: VarId,
    sym: &str,
    defaults: &[(String, Form)],
) -> R {
    let default_form = defaults.iter().find(|(s, _)| s == sym).map(|(_, f)| f);
    match default_form {
        Some(def) => lower_with_default(ctx, got, def),
        None => Ok(got),
    }
}

fn lower_with_default(ctx: &mut LowerCtx, got: VarId, default_form: &Form) -> R {
    let nil_check = ctx.fresh_var();
    ctx.emit(Inst::CallKnown(nil_check, KnownFn::IsNil, vec![got]));

    let then_block = ctx.fresh_block();
    let else_block = ctx.fresh_block();
    let merge_block = ctx.fresh_block();

    ctx.finish_block(Terminator::Branch {
        cond: nil_check,
        then_block,
        else_block,
    });

    ctx.start_block(then_block);
    let default_var = lower_form(ctx, default_form)?;
    let then_exit = ctx.current_block_id();
    ctx.finish_block(Terminator::Jump(merge_block));

    ctx.start_block(else_block);
    let else_exit = ctx.current_block_id();
    ctx.finish_block(Terminator::Jump(merge_block));

    ctx.start_block(merge_block);
    let result = ctx.fresh_var();
    ctx.emit_phi(result, vec![(then_exit, default_var), (else_exit, got)]);
    Ok(result)
}

// ── let ───────────────────────────────────────────────────────────────────────

fn lower_let(ctx: &mut LowerCtx, args: &[Form]) -> R {
    if args.is_empty() {
        return Err(LowerError::MalformedSpecialForm(
            "let requires a binding vector".into(),
        ));
    }
    let FormKind::Vector(bindings) = &args[0].kind else {
        return Err(LowerError::MalformedSpecialForm(
            "let bindings must be a vector".into(),
        ));
    };
    if bindings.len() % 2 != 0 {
        return Err(LowerError::MalformedSpecialForm(
            "let requires even number of binding forms".into(),
        ));
    }

    ctx.push_scope();
    let mut i = 0;
    while i + 1 < bindings.len() {
        let pattern = &bindings[i];
        let val = lower_form(ctx, &bindings[i + 1])?;
        lower_destructure_binding(ctx, pattern, val)?;
        i += 2;
    }
    let result = lower_body(ctx, &args[1..])?;
    ctx.pop_scope();
    Ok(result)
}

// ── loop ──────────────────────────────────────────────────────────────────────

struct BindingInfo {
    pattern: Form,
    gensym_name: Arc<str>,
    init_val: VarId,
}

fn lower_loop(ctx: &mut LowerCtx, args: &[Form]) -> R {
    if args.is_empty() {
        return Err(LowerError::MalformedSpecialForm(
            "loop requires a binding vector".into(),
        ));
    }
    let FormKind::Vector(bindings) = &args[0].kind else {
        return Err(LowerError::MalformedSpecialForm(
            "loop bindings must be a vector".into(),
        ));
    };
    if bindings.len() % 2 != 0 {
        return Err(LowerError::MalformedSpecialForm(
            "loop requires even number of binding forms".into(),
        ));
    }

    // Evaluate initial values; generate gensym names for phi nodes.
    let mut binding_info: Vec<BindingInfo> = Vec::new();
    let mut i = 0;
    while i + 1 < bindings.len() {
        let pattern = bindings[i].clone();
        let init_val = lower_form(ctx, &bindings[i + 1])?;
        let gensym_name: Arc<str> = Arc::from(format!("__loop_{}", ctx.fresh_var().0).as_str());
        binding_info.push(BindingInfo {
            pattern,
            gensym_name,
            init_val,
        });
        i += 2;
    }

    let header = ctx.fresh_block();
    let init_block = ctx.current_block_id();
    ctx.finish_block(Terminator::Jump(header));

    // Header block: phi nodes for each binding.
    ctx.start_block(header);
    ctx.push_scope();

    let phi_vars: Vec<VarId> = binding_info
        .iter()
        .map(|b| {
            let phi_var = ctx.fresh_var();
            ctx.emit_phi(phi_var, vec![(init_block, b.init_val)]);
            ctx.bind_local(b.gensym_name.clone(), phi_var);
            phi_var
        })
        .collect();

    // Apply destructuring from phi vars to the actual patterns.
    for (info, &phi_var) in binding_info.iter().zip(phi_vars.iter()) {
        if let FormKind::Symbol(s) = &info.pattern.kind {
            ctx.bind_local(Arc::from(s.as_str()), phi_var);
        } else {
            lower_destructure_binding(ctx, &info.pattern, phi_var)?;
        }
    }

    ctx.push_loop_header(header, phi_vars);

    let body_result = lower_body(ctx, &args[1..])?;
    let body_exit = ctx.current_block_id();

    ctx.pop_loop_header();

    let exit_block = ctx.fresh_block();
    ctx.finish_block(Terminator::Jump(exit_block));
    ctx.pop_scope();

    ctx.start_block(exit_block);
    let result = ctx.fresh_var();
    ctx.emit_phi(result, vec![(body_exit, body_result)]);
    Ok(result)
}

// ── recur ─────────────────────────────────────────────────────────────────────

fn lower_recur(ctx: &mut LowerCtx, args: &[Form]) -> R {
    let arg_vars: Result<Vec<VarId>, _> = args.iter().map(|a| lower_form(ctx, a)).collect();
    let arg_vars = arg_vars?;

    let (header, phi_vars) = ctx
        .current_loop_header()
        .ok_or_else(|| LowerError::MalformedSpecialForm("recur outside of loop".into()))?;

    let recur_block = ctx.current_block_id();

    // Patch each phi node in the header with our new predecessor.
    for (i, &arg_var) in arg_vars.iter().enumerate() {
        ctx.update_phi_in_header(header, i, recur_block, arg_var);
    }

    ctx.finish_block(Terminator::RecurJump {
        target: header,
        args: arg_vars,
    });
    let _ = phi_vars; // used only for arity check if needed

    // Dead block after recur.
    let new_block = ctx.fresh_block();
    ctx.start_block(new_block);
    Ok(ctx.emit_const(Const::Nil))
}

// ── def ───────────────────────────────────────────────────────────────────────

fn lower_def(ctx: &mut LowerCtx, args: &[Form]) -> R {
    if args.is_empty() {
        return Err(LowerError::MalformedSpecialForm(
            "def requires a name".into(),
        ));
    }
    let FormKind::Symbol(name_sym) = &args[0].kind else {
        return Err(LowerError::MalformedSpecialForm(
            "def name must be a symbol".into(),
        ));
    };
    let name_str: Arc<str> = Arc::from(name_sym.as_str());
    let ns = ctx.ns().clone();

    let val = if args.len() >= 2 {
        lower_form(ctx, &args[1])?
    } else {
        ctx.emit_const(Const::Nil)
    };

    let dst = ctx.fresh_var();
    ctx.emit(Inst::DefVar(dst, ns, name_str, val));
    Ok(dst)
}

// ── defn ──────────────────────────────────────────────────────────────────────

fn lower_defn(ctx: &mut LowerCtx, args: &[Form]) -> R {
    if args.is_empty() {
        return Err(LowerError::MalformedSpecialForm(
            "defn requires a name".into(),
        ));
    }
    let FormKind::Symbol(name_sym) = &args[0].kind else {
        return Err(LowerError::MalformedSpecialForm(
            "defn name must be a symbol".into(),
        ));
    };
    let name_str = name_sym.clone();

    // Skip optional docstring.
    let rest_start = if args.len() > 2 {
        if let FormKind::Str(_) = &args[1].kind {
            2
        } else {
            1
        }
    } else {
        1
    };

    // Build (fn name params body...) args.
    let fn_args: Vec<Form> = std::iter::once(args[0].clone())
        .chain(args[rest_start..].iter().cloned())
        .collect();

    let fn_val = lower_fn(ctx, &fn_args)?;
    let ns = ctx.ns().clone();
    let dst = ctx.fresh_var();
    ctx.emit(Inst::DefVar(dst, ns, Arc::from(name_str.as_str()), fn_val));
    Ok(dst)
}

// ── fn / fn* ──────────────────────────────────────────────────────────────────

fn parse_params(params_vec: &[Form]) -> Result<(Vec<Form>, Option<Form>), LowerError> {
    let mut fixed = Vec::new();
    let mut rest: Option<Form> = None;
    let mut i = 0;
    while i < params_vec.len() {
        if let FormKind::Symbol(s) = &params_vec[i].kind
            && s == "&"
        {
            i += 1;
            if i >= params_vec.len() {
                return Err(LowerError::MalformedSpecialForm(
                    "& must be followed by a parameter name".into(),
                ));
            }
            rest = Some(params_vec[i].clone());
            break;
        }
        fixed.push(params_vec[i].clone());
        i += 1;
    }
    Ok((fixed, rest))
}

struct AritySpec {
    fixed: Vec<Form>,
    rest: Option<Form>,
    body: Vec<Form>,
}

fn lower_fn(ctx: &mut LowerCtx, args: &[Form]) -> R {
    // Detect optional name.
    let (fn_name, body_start) = if let Some(FormKind::Symbol(s)) = args.first().map(|f| &f.kind) {
        (Some(s.clone()), 1)
    } else {
        (None, 0)
    };

    let ns = ctx.ns().clone();

    // If the function is named and not already in scope as a local, create
    // a mutable self-reference var so recursive calls work.
    let self_var_reg: Option<VarId> = if let Some(ref fname) = fn_name {
        if ctx.lookup_local(fname).is_none() {
            let nil_val = ctx.emit_const(Const::Nil);
            let def_dst = ctx.fresh_var();
            ctx.emit(Inst::DefVar(
                def_dst,
                ns.clone(),
                Arc::from(fname.as_str()),
                nil_val,
            ));
            ctx.push_scope();
            ctx.bind_local(Arc::from(fname.as_str()), def_dst);
            Some(def_dst)
        } else {
            None
        }
    } else {
        None
    };

    // Capture all locals (after self-ref scope is pushed).
    let all_locals = ctx.get_all_locals();
    let capture_names: Vec<Arc<str>> = all_locals.iter().map(|(n, _)| n.clone()).collect();
    let capture_vars: Vec<VarId> = all_locals.iter().map(|(_, v)| *v).collect();

    // Pop self-ref scope now that captures are computed.
    if self_var_reg.is_some() {
        ctx.pop_scope();
    }

    let rest_args = &args[body_start..];

    // Parse arities: single [params] body... or multi ([params] body...) ...
    let raw_arities: Vec<AritySpec> = if !rest_args.is_empty() {
        if let FormKind::Vector(_) = &rest_args[0].kind {
            // Single arity
            let (fixed, rest) = parse_params(match &rest_args[0].kind {
                FormKind::Vector(v) => v,
                _ => unreachable!(),
            })?;
            vec![AritySpec {
                fixed,
                rest,
                body: rest_args[1..].to_vec(),
            }]
        } else {
            // Multi arity: each element is a list (params body...)
            rest_args
                .iter()
                .map(|arity_form| {
                    let FormKind::List(arity_parts) = &arity_form.kind else {
                        return Err(LowerError::MalformedSpecialForm(
                            "fn* multi-arity: expected list".into(),
                        ));
                    };
                    let FormKind::Vector(params_vec) = &arity_parts[0].kind else {
                        return Err(LowerError::MalformedSpecialForm(
                            "fn* arity: first element must be param vector".into(),
                        ));
                    };
                    let (fixed, rest) = parse_params(params_vec)?;
                    Ok(AritySpec {
                        fixed,
                        rest,
                        body: arity_parts[1..].to_vec(),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
        }
    } else {
        vec![AritySpec {
            fixed: vec![],
            rest: None,
            body: vec![],
        }]
    };

    let base_name = fn_name
        .clone()
        .unwrap_or_else(|| format!("__cljrs_anon_{}", fresh_global_name_id()));
    let fn_uid = fresh_global_name_id();

    let mut arity_fn_names: Vec<Arc<str>> = Vec::new();
    let mut param_counts: Vec<usize> = Vec::new();
    let mut is_variadic_flags: Vec<bool> = Vec::new();

    for (i, arity) in raw_arities.iter().enumerate() {
        let va_suffix = if arity.rest.is_some() { "_va" } else { "" };
        let arity_name: Arc<str> = Arc::from(format!(
            "__cljrs_fn_{ns}_{base_name}_{fn_uid}_arity{}{}",
            arity.fixed.len(),
            va_suffix
        ));
        arity_fn_names.push(arity_name.clone());
        param_counts.push(arity.fixed.len());
        is_variadic_flags.push(arity.rest.is_some());

        // Lower each arity as a subfunction.
        let subfn = lower_fn_arity(
            ctx,
            Some(arity_name),
            ns.clone(),
            &capture_names,
            &arity.fixed,
            arity.rest.as_ref(),
            &arity.body,
        )?;
        ctx.add_subfunction(subfn);

        let _ = i;
    }

    let tmpl = ClosureTemplate {
        name: fn_name.as_deref().map(Arc::from),
        arity_fn_names,
        param_counts,
        is_variadic: is_variadic_flags,
        capture_names: capture_names.clone(),
    };

    let dst = ctx.fresh_var();
    ctx.emit(Inst::AllocClosure(dst, tmpl, capture_vars));

    // If we created a self-ref var, point it at the closure.
    if let Some(self_reg) = self_var_reg {
        ctx.emit(Inst::SetBang(self_reg, dst));
    }

    Ok(dst)
}

fn lower_fn_arity(
    _parent_ctx: &mut LowerCtx,
    arity_name: Option<Arc<str>>,
    ns: Arc<str>,
    capture_names: &[Arc<str>],
    fixed_params: &[Form],
    rest_param: Option<&Form>,
    body_forms: &[Form],
) -> Result<IrFunction, LowerError> {
    // Compute raw parameter info (gensym for destructuring patterns).
    struct ParamInfo {
        name: Arc<str>,
        pattern: Option<Form>,
    }

    let param_infos: Vec<ParamInfo> = fixed_params
        .iter()
        .map(|p| {
            if let FormKind::Symbol(s) = &p.kind {
                ParamInfo {
                    name: Arc::from(s.as_str()),
                    pattern: None,
                }
            } else {
                ParamInfo {
                    name: Arc::from(format!("__destructure_{}", fresh_global_name_id()).as_str()),
                    pattern: Some(p.clone()),
                }
            }
        })
        .collect();

    let rest_info: Option<ParamInfo> = rest_param.map(|p| {
        if let FormKind::Symbol(s) = &p.kind {
            ParamInfo {
                name: Arc::from(s.as_str()),
                pattern: None,
            }
        } else {
            ParamInfo {
                name: Arc::from(format!("__destructure_rest_{}", fresh_global_name_id()).as_str()),
                pattern: Some(p.clone()),
            }
        }
    });

    // Build full param list: captures first, then fixed, then rest.
    let mut all_param_names: Vec<Arc<str>> = capture_names.to_vec();
    for info in &param_infos {
        all_param_names.push(info.name.clone());
    }
    if let Some(ref ri) = rest_info {
        all_param_names.push(ri.name.clone());
    }

    let mut sub = LowerCtx::new(arity_name, ns);

    // Bind all params.
    let mut bound_params: Vec<(Arc<str>, VarId)> = Vec::with_capacity(all_param_names.len());
    for pname in &all_param_names {
        let id = sub.fresh_var();
        sub.bind_local(pname.clone(), id);
        bound_params.push((pname.clone(), id));
    }

    // User params (excluding captures) are the recur targets.
    let user_param_names: Vec<Arc<str>> = {
        let mut v: Vec<Arc<str>> = param_infos.iter().map(|pi| pi.name.clone()).collect();
        if let Some(ref ri) = rest_info {
            v.push(ri.name.clone());
        }
        v
    };

    // Set up implicit loop for recur support:
    // entry block → jump to header → phi nodes → body
    let init_block = sub.current_block_id();
    let init_vals: Vec<VarId> = user_param_names
        .iter()
        .map(|n| sub.lookup_local(n).unwrap())
        .collect();

    let header = sub.fresh_block();
    sub.finish_block(Terminator::Jump(header));
    sub.start_block(header);
    sub.push_scope();

    // Phi nodes for each user param.
    let phi_vars: Vec<VarId> = user_param_names
        .iter()
        .zip(init_vals.iter())
        .map(|(pname, &init_val)| {
            let phi_var = sub.fresh_var();
            sub.emit_phi(phi_var, vec![(init_block, init_val)]);
            sub.bind_local(pname.clone(), phi_var);
            phi_var
        })
        .collect();

    // Emit destructuring for pattern params.
    for info in &param_infos {
        if let Some(ref pat) = info.pattern {
            let gensym_var = sub.lookup_local(&info.name).unwrap();
            lower_destructure_binding(&mut sub, pat, gensym_var)?;
        }
    }
    if let Some(ref ri) = rest_info
        && let Some(ref pat) = ri.pattern
    {
        let gensym_var = sub.lookup_local(&ri.name).unwrap();
        lower_destructure_binding(&mut sub, pat, gensym_var)?;
    }

    // Push loop header for recur.
    sub.push_loop_header(header, phi_vars);

    let body_result = lower_body(&mut sub, body_forms)?;
    let body_exit = sub.current_block_id();

    sub.pop_loop_header();

    let exit_block = sub.fresh_block();
    sub.finish_block(Terminator::Jump(exit_block));
    sub.pop_scope();
    sub.start_block(exit_block);

    let exit_result = sub.fresh_var();
    sub.emit_phi(exit_result, vec![(body_exit, body_result)]);
    sub.finish_block(Terminator::Return(exit_result));

    // Propagate any subfunctions from the nested context to the parent.
    // (fn* inside fn* produces sub-subfunctions that belong to the tree.)
    let subfunctions = std::mem::take(&mut sub.subfunctions);
    let mut ir = sub.build(bound_params);
    ir.subfunctions = subfunctions;
    Ok(ir)
}

// ── throw ─────────────────────────────────────────────────────────────────────

fn lower_throw(ctx: &mut LowerCtx, args: &[Form]) -> R {
    if args.len() != 1 {
        return Err(LowerError::MalformedSpecialForm(
            "throw expects 1 argument".into(),
        ));
    }
    let val = lower_form(ctx, &args[0])?;
    ctx.emit(Inst::Throw(val));
    ctx.finish_block(Terminator::Unreachable);

    let new_block = ctx.fresh_block();
    ctx.start_block(new_block);
    Ok(ctx.emit_const(Const::Nil))
}

// ── try ───────────────────────────────────────────────────────────────────────

fn lower_try(ctx: &mut LowerCtx, args: &[Form]) -> R {
    // Split into body forms, catch clause, finally clause.
    let body_forms: Vec<Form> = args
        .iter()
        .take_while(|f| !is_catch_or_finally(f))
        .cloned()
        .collect();

    let catch_form = args.iter().find(|f| {
        if let FormKind::List(p) = &f.kind {
            matches!(p.first().map(|h| &h.kind), Some(FormKind::Symbol(s)) if s == "catch")
        } else {
            false
        }
    });

    let finally_form = args.iter().find(|f| {
        if let FormKind::List(p) = &f.kind {
            matches!(p.first().map(|h| &h.kind), Some(FormKind::Symbol(s)) if s == "finally")
        } else {
            false
        }
    });

    let ns = ctx.ns().clone();
    let all_locals = ctx.get_all_locals();
    let capture_names: Vec<Arc<str>> = all_locals.iter().map(|(n, _)| n.clone()).collect();
    let capture_vars: Vec<VarId> = all_locals.iter().map(|(_, v)| *v).collect();
    let ncaptures = capture_names.len();

    // Body closure.
    let body_name: Arc<str> =
        Arc::from(format!("__cljrs_try_body_{}", fresh_global_name_id()).as_str());
    let body_fn_ir = lower_fn_arity(
        ctx,
        Some(body_name.clone()),
        ns.clone(),
        &capture_names,
        &[],
        None,
        &body_forms,
    )?;
    ctx.add_subfunction(body_fn_ir);
    let body_closure = ctx.fresh_var();
    ctx.emit(Inst::AllocClosure(
        body_closure,
        ClosureTemplate {
            name: None,
            arity_fn_names: vec![body_name],
            param_counts: vec![ncaptures],
            is_variadic: vec![false],
            capture_names: capture_names.clone(),
        },
        capture_vars.clone(),
    ));

    // Catch closure.
    let catch_closure = if let Some(cf) = catch_form {
        if let FormKind::List(cp) = &cf.kind {
            let catch_sym = if cp.len() > 2 {
                if let FormKind::Symbol(s) = &cp[2].kind {
                    s.clone()
                } else {
                    "e".to_string()
                }
            } else {
                "e".to_string()
            };
            let catch_body = if cp.len() > 3 {
                cp[3..].to_vec()
            } else {
                vec![]
            };
            let catch_name: Arc<str> =
                Arc::from(format!("__cljrs_try_catch_{}", fresh_global_name_id()).as_str());
            let catch_params = vec![Form::new(
                FormKind::Symbol(catch_sym),
                cljrs_types::span::Span::new(Arc::new("<try>".to_string()), 0, 0, 1, 1),
            )];
            let catch_fn_ir = lower_fn_arity(
                ctx,
                Some(catch_name.clone()),
                ns.clone(),
                &capture_names,
                &catch_params,
                None,
                &catch_body,
            )?;
            ctx.add_subfunction(catch_fn_ir);
            let dst = ctx.fresh_var();
            ctx.emit(Inst::AllocClosure(
                dst,
                ClosureTemplate {
                    name: None,
                    arity_fn_names: vec![catch_name],
                    param_counts: vec![ncaptures + 1],
                    is_variadic: vec![false],
                    capture_names: capture_names.clone(),
                },
                capture_vars.clone(),
            ));
            dst
        } else {
            ctx.emit_const(Const::Nil)
        }
    } else {
        ctx.emit_const(Const::Nil)
    };

    // Finally closure.
    let finally_closure = if let Some(ff) = finally_form {
        if let FormKind::List(fp) = &ff.kind {
            let fin_body = fp[1..].to_vec();
            let fin_name: Arc<str> =
                Arc::from(format!("__cljrs_try_finally_{}", fresh_global_name_id()).as_str());
            let fin_fn_ir = lower_fn_arity(
                ctx,
                Some(fin_name.clone()),
                ns.clone(),
                &capture_names,
                &[],
                None,
                &fin_body,
            )?;
            ctx.add_subfunction(fin_fn_ir);
            let dst = ctx.fresh_var();
            ctx.emit(Inst::AllocClosure(
                dst,
                ClosureTemplate {
                    name: None,
                    arity_fn_names: vec![fin_name],
                    param_counts: vec![ncaptures],
                    is_variadic: vec![false],
                    capture_names: capture_names.clone(),
                },
                capture_vars.clone(),
            ));
            dst
        } else {
            ctx.emit_const(Const::Nil)
        }
    } else {
        ctx.emit_const(Const::Nil)
    };

    let result = ctx.fresh_var();
    ctx.emit(Inst::CallKnown(
        result,
        KnownFn::TryCatchFinally,
        vec![body_closure, catch_closure, finally_closure],
    ));
    Ok(result)
}

fn is_catch_or_finally(form: &Form) -> bool {
    if let FormKind::List(p) = &form.kind
        && let Some(FormKind::Symbol(s)) = p.first().map(|f| &f.kind)
    {
        return s == "catch" || s == "finally";
    }
    false
}

// ── binding ───────────────────────────────────────────────────────────────────

fn lower_binding(ctx: &mut LowerCtx, args: &[Form]) -> R {
    if args.is_empty() {
        return Err(LowerError::MalformedSpecialForm(
            "binding requires a binding vector".into(),
        ));
    }
    let FormKind::Vector(bindings) = &args[0].kind else {
        return Err(LowerError::MalformedSpecialForm(
            "binding bindings must be a vector".into(),
        ));
    };
    if bindings.len() % 2 != 0 {
        return Err(LowerError::MalformedSpecialForm(
            "binding requires even number of forms".into(),
        ));
    }

    // Evaluate binding pairs: Var object + value.
    let mut flat_bindings: Vec<VarId> = Vec::new();
    let mut i = 0;
    while i + 1 < bindings.len() {
        let var_sym = &bindings[i];
        let FormKind::Symbol(sym_str) = &var_sym.kind else {
            return Err(LowerError::MalformedSpecialForm(
                "binding var must be a symbol".into(),
            ));
        };
        let (var_ns, var_name) = split_sym(sym_str, ctx.ns());
        let var_dst = ctx.fresh_var();
        ctx.emit(Inst::LoadVar(var_dst, var_ns, var_name));

        let val_var = lower_form(ctx, &bindings[i + 1])?;
        flat_bindings.push(var_dst);
        flat_bindings.push(val_var);
        i += 2;
    }

    // Build the body as a closure.
    let ns = ctx.ns().clone();
    let all_locals = ctx.get_all_locals();
    let capture_names: Vec<Arc<str>> = all_locals.iter().map(|(n, _)| n.clone()).collect();
    let capture_vars: Vec<VarId> = all_locals.iter().map(|(_, v)| *v).collect();
    let ncaptures = capture_names.len();

    let body_name: Arc<str> =
        Arc::from(format!("__cljrs_binding_body_{}", fresh_global_name_id()).as_str());
    let body_fn_ir = lower_fn_arity(
        ctx,
        Some(body_name.clone()),
        ns,
        &capture_names,
        &[],
        None,
        &args[1..],
    )?;
    ctx.add_subfunction(body_fn_ir);
    let body_closure = ctx.fresh_var();
    ctx.emit(Inst::AllocClosure(
        body_closure,
        ClosureTemplate {
            name: None,
            arity_fn_names: vec![body_name],
            param_counts: vec![ncaptures],
            is_variadic: vec![false],
            capture_names,
        },
        capture_vars,
    ));

    flat_bindings.push(body_closure);
    let result = ctx.fresh_var();
    ctx.emit(Inst::CallKnown(
        result,
        KnownFn::WithBindings,
        flat_bindings,
    ));
    Ok(result)
}

// ── letfn ─────────────────────────────────────────────────────────────────────

fn lower_letfn(ctx: &mut LowerCtx, args: &[Form]) -> R {
    if args.len() < 2 {
        return Err(LowerError::MalformedSpecialForm(
            "letfn requires bindings and body".into(),
        ));
    }
    let FormKind::Vector(bindings) = &args[0].kind else {
        return Err(LowerError::MalformedSpecialForm(
            "letfn bindings must be a vector".into(),
        ));
    };

    let ns = ctx.ns().clone();

    struct LetfnBinding {
        name: Arc<str>,
        params: Form,
        body: Vec<Form>,
    }

    let parsed: Vec<LetfnBinding> = bindings
        .iter()
        .map(|b| {
            let FormKind::List(parts) = &b.kind else {
                return Err(LowerError::MalformedSpecialForm(
                    "letfn binding must be a list".into(),
                ));
            };
            let FormKind::Symbol(sym) = &parts[0].kind else {
                return Err(LowerError::MalformedSpecialForm(
                    "letfn binding name must be a symbol".into(),
                ));
            };
            if parts.len() < 2 {
                return Err(LowerError::MalformedSpecialForm(
                    "letfn binding needs params".into(),
                ));
            }
            Ok(LetfnBinding {
                name: Arc::from(sym.as_str()),
                params: parts[1].clone(),
                body: parts[2..].to_vec(),
            })
        })
        .collect::<Result<_, _>>()?;

    // 1. Create a DefVar cell for each binding, initialised to nil.
    let var_regs: Vec<VarId> = parsed
        .iter()
        .map(|p| {
            let nil_val = ctx.emit_const(Const::Nil);
            let dst = ctx.fresh_var();
            ctx.emit(Inst::DefVar(dst, ns.clone(), p.name.clone(), nil_val));
            dst
        })
        .collect();

    // 2. Bind each name → its Var object so closures capture the cell.
    ctx.push_scope();
    for (p, &var_reg) in parsed.iter().zip(var_regs.iter()) {
        ctx.bind_local(p.name.clone(), var_reg);
    }

    // 3. Compile each closure with the Var-refs in scope.
    let closures: Vec<VarId> = parsed
        .iter()
        .map(|p| {
            // Build (fn name params body...) args.
            let fn_args_forms: Vec<Form> = {
                let name_form = Form::new(
                    FormKind::Symbol(p.name.to_string()),
                    cljrs_types::span::Span::new(Arc::new("<letfn>".to_string()), 0, 0, 1, 1),
                );
                let mut v = vec![name_form, p.params.clone()];
                v.extend(p.body.clone());
                v
            };
            lower_fn(ctx, &fn_args_forms)
        })
        .collect::<Result<_, _>>()?;

    // 4. Pop the Var-ref scope — closures have already captured them.
    ctx.pop_scope();

    // 5. Fill each Var with its closure.
    for (&var_reg, &closure) in var_regs.iter().zip(closures.iter()) {
        ctx.emit(Inst::SetBang(var_reg, closure));
    }

    // 6. Bind names to resolved function values for the body.
    ctx.push_scope();
    for (p, &var_reg) in parsed.iter().zip(var_regs.iter()) {
        let fn_val = ctx.fresh_var();
        ctx.emit(Inst::Deref(fn_val, var_reg));
        ctx.bind_local(p.name.clone(), fn_val);
    }

    let result = lower_body(ctx, &args[1..])?;
    ctx.pop_scope();
    Ok(result)
}

// ── with-out-str ──────────────────────────────────────────────────────────────

fn lower_with_out_str(ctx: &mut LowerCtx, body_forms: &[Form]) -> R {
    let ns = ctx.ns().clone();
    let all_locals = ctx.get_all_locals();
    let capture_names: Vec<Arc<str>> = all_locals.iter().map(|(n, _)| n.clone()).collect();
    let capture_vars: Vec<VarId> = all_locals.iter().map(|(_, v)| *v).collect();
    let ncaptures = capture_names.len();

    let body_name: Arc<str> =
        Arc::from(format!("__cljrs_with_out_str_{}", fresh_global_name_id()).as_str());
    let body_fn_ir = lower_fn_arity(
        ctx,
        Some(body_name.clone()),
        ns,
        &capture_names,
        &[],
        None,
        body_forms,
    )?;
    ctx.add_subfunction(body_fn_ir);

    let body_closure = ctx.fresh_var();
    ctx.emit(Inst::AllocClosure(
        body_closure,
        ClosureTemplate {
            name: None,
            arity_fn_names: vec![body_name],
            param_counts: vec![ncaptures],
            is_variadic: vec![false],
            capture_names,
        },
        capture_vars,
    ));

    let result = ctx.fresh_var();
    ctx.emit(Inst::CallKnown(
        result,
        KnownFn::WithOutStr,
        vec![body_closure],
    ));
    Ok(result)
}

// ── set! ──────────────────────────────────────────────────────────────────────

fn lower_set_bang(ctx: &mut LowerCtx, args: &[Form]) -> R {
    if args.len() != 2 {
        return Err(LowerError::MalformedSpecialForm(
            "set! expects 2 arguments".into(),
        ));
    }
    let FormKind::Symbol(sym_str) = &args[0].kind else {
        return Err(LowerError::MalformedSpecialForm(
            "set! target must be a symbol".into(),
        ));
    };
    let (var_ns, var_name) = split_sym(sym_str, ctx.ns());
    let var_dst = ctx.fresh_var();
    ctx.emit(Inst::LoadVar(var_dst, var_ns, var_name));

    let val = lower_form(ctx, &args[1])?;
    ctx.emit(Inst::SetBang(var_dst, val));
    Ok(val)
}

// ── and / or ──────────────────────────────────────────────────────────────────

fn lower_and(ctx: &mut LowerCtx, args: &[Form]) -> R {
    match args.len() {
        0 => Ok(ctx.emit_const(Const::Bool(true))),
        1 => lower_form(ctx, &args[0]),
        _ => {
            let first_val = lower_form(ctx, &args[0])?;
            let rest_block = ctx.fresh_block();
            let join_block = ctx.fresh_block();
            let first_exit = ctx.current_block_id();

            ctx.finish_block(Terminator::Branch {
                cond: first_val,
                then_block: rest_block,
                else_block: join_block,
            });
            ctx.start_block(rest_block);

            let rest_val = lower_and(ctx, &args[1..])?;
            let rest_exit = ctx.current_block_id();
            ctx.finish_block(Terminator::Jump(join_block));

            ctx.start_block(join_block);
            let result = ctx.fresh_var();
            ctx.emit_phi(result, vec![(first_exit, first_val), (rest_exit, rest_val)]);
            Ok(result)
        }
    }
}

fn lower_or(ctx: &mut LowerCtx, args: &[Form]) -> R {
    match args.len() {
        0 => Ok(ctx.emit_const(Const::Nil)),
        1 => lower_form(ctx, &args[0]),
        _ => {
            let first_val = lower_form(ctx, &args[0])?;
            let rest_block = ctx.fresh_block();
            let join_block = ctx.fresh_block();
            let first_exit = ctx.current_block_id();

            // or: if truthy, short-circuit; else try rest.
            ctx.finish_block(Terminator::Branch {
                cond: first_val,
                then_block: join_block,
                else_block: rest_block,
            });
            ctx.start_block(rest_block);

            let rest_val = lower_or(ctx, &args[1..])?;
            let rest_exit = ctx.current_block_id();
            ctx.finish_block(Terminator::Jump(join_block));

            ctx.start_block(join_block);
            let result = ctx.fresh_var();
            ctx.emit_phi(result, vec![(first_exit, first_val), (rest_exit, rest_val)]);
            Ok(result)
        }
    }
}

// ── Call lowering ─────────────────────────────────────────────────────────────

fn lower_call(ctx: &mut LowerCtx, callee_form: &Form, arg_forms: &[Form]) -> R {
    let sym_name = if let FormKind::Symbol(s) = &callee_form.kind {
        Some(s.as_str())
    } else {
        None
    };

    // Try inline expansions first.
    if let Some(name) = sym_name
        && let Some(result) = try_inline_expansion(ctx, name, arg_forms)?
    {
        return Ok(result);
    }

    // Known function?
    if let Some(name) = sym_name
        && let Some(known) = resolve_known_fn(name)
    {
        return lower_known_call(ctx, known, name, arg_forms);
    }

    // Generic dynamic call.
    let callee = lower_form(ctx, callee_form)?;
    let arg_vars: Result<Vec<VarId>, _> = arg_forms.iter().map(|a| lower_form(ctx, a)).collect();
    let arg_vars = arg_vars?;
    let dst = ctx.fresh_var();
    ctx.emit(Inst::Call(dst, callee, arg_vars));
    Ok(dst)
}

fn lower_known_call(ctx: &mut LowerCtx, known: KnownFn, name: &str, arg_forms: &[Form]) -> R {
    // Special dispatch for variadic known functions.
    match &known {
        KnownFn::Apply => return lower_apply_call(ctx, arg_forms),

        KnownFn::Reduce2 => {
            // reduce dispatches by arity: 2→Reduce2, 3→Reduce3.
            let arg_vars: Result<Vec<VarId>, _> =
                arg_forms.iter().map(|a| lower_form(ctx, a)).collect();
            let arg_vars = arg_vars?;
            let kf = match arg_vars.len() {
                2 => KnownFn::Reduce2,
                3 => KnownFn::Reduce3,
                _ => {
                    let callee = lower_symbol(ctx, name)?;
                    let dst = ctx.fresh_var();
                    ctx.emit(Inst::Call(dst, callee, arg_vars));
                    return Ok(dst);
                }
            };
            let dst = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(dst, kf, arg_vars));
            return Ok(dst);
        }

        KnownFn::Into => {
            let arg_vars: Result<Vec<VarId>, _> =
                arg_forms.iter().map(|a| lower_form(ctx, a)).collect();
            let arg_vars = arg_vars?;
            let kf = match arg_vars.len() {
                2 => KnownFn::Into,
                3 => KnownFn::Into3,
                _ => {
                    let callee = lower_symbol(ctx, name)?;
                    let dst = ctx.fresh_var();
                    ctx.emit(Inst::Call(dst, callee, arg_vars));
                    return Ok(dst);
                }
            };
            let dst = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(dst, kf, arg_vars));
            return Ok(dst);
        }

        KnownFn::Range1 => {
            let arg_vars: Result<Vec<VarId>, _> =
                arg_forms.iter().map(|a| lower_form(ctx, a)).collect();
            let arg_vars = arg_vars?;
            let kf = match arg_vars.len() {
                1 => KnownFn::Range1,
                2 => KnownFn::Range2,
                3 => KnownFn::Range3,
                _ => {
                    let callee = lower_symbol(ctx, name)?;
                    let dst = ctx.fresh_var();
                    ctx.emit(Inst::Call(dst, callee, arg_vars));
                    return Ok(dst);
                }
            };
            let dst = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(dst, kf, arg_vars));
            return Ok(dst);
        }

        KnownFn::Partition2 => {
            let arg_vars: Result<Vec<VarId>, _> =
                arg_forms.iter().map(|a| lower_form(ctx, a)).collect();
            let arg_vars = arg_vars?;
            let kf = match arg_vars.len() {
                2 => KnownFn::Partition2,
                3 => KnownFn::Partition3,
                4 => KnownFn::Partition4,
                _ => {
                    let callee = lower_symbol(ctx, name)?;
                    let dst = ctx.fresh_var();
                    ctx.emit(Inst::Call(dst, callee, arg_vars));
                    return Ok(dst);
                }
            };
            let dst = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(dst, kf, arg_vars));
            return Ok(dst);
        }

        // Variadic: emit as-is, codegen uses stack-spill.
        KnownFn::Concat | KnownFn::Merge | KnownFn::Juxt | KnownFn::Comp | KnownFn::Partial => {
            let arg_vars: Result<Vec<VarId>, _> =
                arg_forms.iter().map(|a| lower_form(ctx, a)).collect();
            let arg_vars = arg_vars?;
            let dst = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(dst, known, arg_vars));
            return Ok(dst);
        }

        _ => {}
    }

    // Binary-foldable arithmetic/comparison?
    if is_binary_foldable(&known) {
        let arg_vars: Result<Vec<VarId>, _> =
            arg_forms.iter().map(|a| lower_form(ctx, a)).collect();
        let arg_vars = arg_vars?;
        return emit_binary_fold(ctx, known, arg_vars);
    }

    // Fixed-arity known call.
    let arg_vars: Result<Vec<VarId>, _> = arg_forms.iter().map(|a| lower_form(ctx, a)).collect();
    let arg_vars = arg_vars?;

    // Arity check for `pr` with 0 args.
    if known == KnownFn::Pr && arg_vars.is_empty() {
        let empty = ctx.emit_const(Const::Str(Arc::from("")));
        let dst = ctx.fresh_var();
        ctx.emit(Inst::CallKnown(dst, KnownFn::Pr, vec![empty]));
        return Ok(dst);
    }

    let expected = known_fn_arity(&known);
    if let Some(exp) = expected
        && arg_vars.len() != exp
    {
        // Arity mismatch — fall through to dynamic call.
        let callee = lower_symbol(ctx, name)?;
        let dst = ctx.fresh_var();
        ctx.emit(Inst::Call(dst, callee, arg_vars));
        return Ok(dst);
    }

    let dst = ctx.fresh_var();
    ctx.emit(Inst::CallKnown(dst, known, arg_vars));
    Ok(dst)
}

fn lower_apply_call(ctx: &mut LowerCtx, arg_forms: &[Form]) -> R {
    if arg_forms.len() < 2 {
        return Err(LowerError::MalformedSpecialForm(
            "apply requires at least 2 arguments".into(),
        ));
    }
    let arg_vars: Result<Vec<VarId>, _> = arg_forms.iter().map(|a| lower_form(ctx, a)).collect();
    let arg_vars = arg_vars?;

    if arg_vars.len() == 2 {
        let dst = ctx.fresh_var();
        ctx.emit(Inst::CallKnown(dst, KnownFn::Apply, arg_vars));
        return Ok(dst);
    }

    // Multi-arg: (apply f a b c arglist) → prepend a,b,c to arglist via cons.
    let f_var = arg_vars[0];
    let fixed_args = &arg_vars[1..arg_vars.len() - 1];
    let arglist_var = *arg_vars.last().unwrap();

    let combined = fixed_args.iter().rev().fold(arglist_var, |tail, &fixed| {
        let dst = ctx.fresh_var();
        ctx.emit(Inst::CallKnown(dst, KnownFn::Cons, vec![fixed, tail]));
        dst
    });

    let dst = ctx.fresh_var();
    ctx.emit(Inst::CallKnown(dst, KnownFn::Apply, vec![f_var, combined]));
    Ok(dst)
}

// ── Binary folding ────────────────────────────────────────────────────────────

fn is_binary_foldable(kf: &KnownFn) -> bool {
    matches!(
        kf,
        KnownFn::Add
            | KnownFn::Sub
            | KnownFn::Mul
            | KnownFn::Div
            | KnownFn::Rem
            | KnownFn::Eq
            | KnownFn::Lt
            | KnownFn::Gt
            | KnownFn::Lte
            | KnownFn::Gte
    )
}

fn is_comparison(kf: &KnownFn) -> bool {
    matches!(
        kf,
        KnownFn::Eq | KnownFn::Lt | KnownFn::Gt | KnownFn::Lte | KnownFn::Gte
    )
}

fn emit_binary_fold(ctx: &mut LowerCtx, kf: KnownFn, args: Vec<VarId>) -> R {
    match args.len() {
        0 => match &kf {
            KnownFn::Add => Ok(ctx.emit_const(Const::Long(0))),
            KnownFn::Mul => Ok(ctx.emit_const(Const::Long(1))),
            _ => Err(LowerError::MalformedSpecialForm(format!(
                "wrong number of args (0) for {:?}",
                kf
            ))),
        },
        1 => match &kf {
            KnownFn::Add | KnownFn::Mul => Ok(args[0]),
            KnownFn::Sub => {
                let zero = ctx.emit_const(Const::Long(0));
                let dst = ctx.fresh_var();
                ctx.emit(Inst::CallKnown(dst, KnownFn::Sub, vec![zero, args[0]]));
                Ok(dst)
            }
            KnownFn::Div => {
                let one = ctx.emit_const(Const::Long(1));
                let dst = ctx.fresh_var();
                ctx.emit(Inst::CallKnown(dst, KnownFn::Div, vec![one, args[0]]));
                Ok(dst)
            }
            _ if is_comparison(&kf) => Ok(ctx.emit_const(Const::Bool(true))),
            _ => {
                let dst = ctx.fresh_var();
                ctx.emit(Inst::CallKnown(dst, kf, args));
                Ok(dst)
            }
        },
        2 => {
            let dst = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(dst, kf, args));
            Ok(dst)
        }
        _ => {
            if is_comparison(&kf) {
                emit_comparison_chain(ctx, kf, args)
            } else {
                // Left-fold: (+ (+ a b) c)
                let first = args[0];

                args[1..].iter().try_fold(first, |prev, &next| {
                    let dst = ctx.fresh_var();
                    ctx.emit(Inst::CallKnown(dst, kf.clone(), vec![prev, next]));
                    Ok(dst)
                })
            }
        }
    }
}

fn emit_comparison_chain(ctx: &mut LowerCtx, kf: KnownFn, args: Vec<VarId>) -> R {
    // (< a b c) → (and (< a b) (< b c))
    let merge_block = ctx.fresh_block();
    let mut predecessors: Vec<(BlockId, VarId)> = Vec::new();

    let pairs: Vec<(VarId, VarId)> = args.windows(2).map(|w| (w[0], w[1])).collect();
    let last_idx = pairs.len() - 1;

    for (i, (a, b)) in pairs.iter().enumerate() {
        let cmp_dst = ctx.fresh_var();
        ctx.emit(Inst::CallKnown(cmp_dst, kf.clone(), vec![*a, *b]));

        if i == last_idx {
            let last_exit = ctx.current_block_id();
            ctx.finish_block(Terminator::Jump(merge_block));
            ctx.start_block(merge_block);
            let result = ctx.fresh_var();
            predecessors.push((last_exit, cmp_dst));
            ctx.emit_phi(result, predecessors);
            return Ok(result);
        } else {
            let next_block = ctx.fresh_block();
            let false_exit = ctx.current_block_id();
            let false_val = ctx.emit_const(Const::Bool(false));
            ctx.finish_block(Terminator::Branch {
                cond: cmp_dst,
                then_block: next_block,
                else_block: merge_block,
            });
            predecessors.push((false_exit, false_val));
            ctx.start_block(next_block);
        }
    }

    // Unreachable if pairs is empty, but satisfy the compiler.
    Ok(ctx.emit_const(Const::Bool(true)))
}

// ── Inline expansions ─────────────────────────────────────────────────────────

fn try_inline_expansion(
    ctx: &mut LowerCtx,
    callee_name: &str,
    args: &[Form],
) -> Result<Option<VarId>, LowerError> {
    match strip_ns_prefix(callee_name) {
        "inc" => {
            let x = lower_form(
                ctx,
                args.first().ok_or_else(|| malformed("inc needs 1 arg"))?,
            )?;
            let one = ctx.emit_const(Const::Long(1));
            let dst = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(dst, KnownFn::Add, vec![x, one]));
            Ok(Some(dst))
        }
        "dec" => {
            let x = lower_form(
                ctx,
                args.first().ok_or_else(|| malformed("dec needs 1 arg"))?,
            )?;
            let one = ctx.emit_const(Const::Long(1));
            let dst = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(dst, KnownFn::Sub, vec![x, one]));
            Ok(Some(dst))
        }
        "not" => {
            let x = lower_form(
                ctx,
                args.first().ok_or_else(|| malformed("not needs 1 arg"))?,
            )?;
            Ok(Some(emit_not(ctx, x)))
        }
        "not=" => {
            let a = lower_form(
                ctx,
                args.first().ok_or_else(|| malformed("not= needs 2 args"))?,
            )?;
            let b = lower_form(
                ctx,
                args.get(1).ok_or_else(|| malformed("not= needs 2 args"))?,
            )?;
            let eq = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(eq, KnownFn::Eq, vec![a, b]));
            Ok(Some(emit_not(ctx, eq)))
        }
        "zero?" => {
            let x = lower_form(
                ctx,
                args.first().ok_or_else(|| malformed("zero? needs 1 arg"))?,
            )?;
            let zero = ctx.emit_const(Const::Long(0));
            let dst = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(dst, KnownFn::Eq, vec![x, zero]));
            Ok(Some(dst))
        }
        "pos?" => {
            let x = lower_form(
                ctx,
                args.first().ok_or_else(|| malformed("pos? needs 1 arg"))?,
            )?;
            let zero = ctx.emit_const(Const::Long(0));
            let dst = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(dst, KnownFn::Gt, vec![x, zero]));
            Ok(Some(dst))
        }
        "neg?" => {
            let x = lower_form(
                ctx,
                args.first().ok_or_else(|| malformed("neg? needs 1 arg"))?,
            )?;
            let zero = ctx.emit_const(Const::Long(0));
            let dst = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(dst, KnownFn::Lt, vec![x, zero]));
            Ok(Some(dst))
        }
        "even?" => {
            let x = lower_form(
                ctx,
                args.first().ok_or_else(|| malformed("even? needs 1 arg"))?,
            )?;
            let two = ctx.emit_const(Const::Long(2));
            let rem = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(rem, KnownFn::Rem, vec![x, two]));
            let zero = ctx.emit_const(Const::Long(0));
            let dst = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(dst, KnownFn::Eq, vec![rem, zero]));
            Ok(Some(dst))
        }
        "odd?" => {
            let x = lower_form(
                ctx,
                args.first().ok_or_else(|| malformed("odd? needs 1 arg"))?,
            )?;
            let two = ctx.emit_const(Const::Long(2));
            let rem = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(rem, KnownFn::Rem, vec![x, two]));
            let zero = ctx.emit_const(Const::Long(0));
            let eq = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(eq, KnownFn::Eq, vec![rem, zero]));
            Ok(Some(emit_not(ctx, eq)))
        }
        "true?" => {
            let x = lower_form(
                ctx,
                args.first().ok_or_else(|| malformed("true? needs 1 arg"))?,
            )?;
            let t = ctx.emit_const(Const::Bool(true));
            let dst = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(dst, KnownFn::Identical, vec![x, t]));
            Ok(Some(dst))
        }
        "false?" => {
            let x = lower_form(
                ctx,
                args.first()
                    .ok_or_else(|| malformed("false? needs 1 arg"))?,
            )?;
            let f = ctx.emit_const(Const::Bool(false));
            let dst = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(dst, KnownFn::Identical, vec![x, f]));
            Ok(Some(dst))
        }
        "max" => {
            let a = lower_form(
                ctx,
                args.first().ok_or_else(|| malformed("max needs 2 args"))?,
            )?;
            let b = lower_form(
                ctx,
                args.get(1).ok_or_else(|| malformed("max needs 2 args"))?,
            )?;
            Ok(Some(emit_max(ctx, a, b)))
        }
        "min" => {
            let a = lower_form(
                ctx,
                args.first().ok_or_else(|| malformed("min needs 2 args"))?,
            )?;
            let b = lower_form(
                ctx,
                args.get(1).ok_or_else(|| malformed("min needs 2 args"))?,
            )?;
            Ok(Some(emit_min(ctx, a, b)))
        }
        "empty?" => {
            let x = lower_form(
                ctx,
                args.first()
                    .ok_or_else(|| malformed("empty? needs 1 arg"))?,
            )?;
            let seq_dst = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(seq_dst, KnownFn::Seq, vec![x]));
            let dst = ctx.fresh_var();
            ctx.emit(Inst::CallKnown(dst, KnownFn::IsNil, vec![seq_dst]));
            Ok(Some(dst))
        }
        _ => Ok(None),
    }
}

fn emit_not(ctx: &mut LowerCtx, x: VarId) -> VarId {
    let then_block = ctx.fresh_block();
    let else_block = ctx.fresh_block();
    let join_block = ctx.fresh_block();

    ctx.finish_block(Terminator::Branch {
        cond: x,
        then_block,
        else_block,
    });

    ctx.start_block(then_block);
    let false_val = ctx.emit_const(Const::Bool(false));
    let then_exit = ctx.current_block_id();
    ctx.finish_block(Terminator::Jump(join_block));

    ctx.start_block(else_block);
    let true_val = ctx.emit_const(Const::Bool(true));
    let else_exit = ctx.current_block_id();
    ctx.finish_block(Terminator::Jump(join_block));

    ctx.start_block(join_block);
    let dst = ctx.fresh_var();
    ctx.emit_phi(dst, vec![(then_exit, false_val), (else_exit, true_val)]);
    dst
}

fn emit_max(ctx: &mut LowerCtx, a: VarId, b: VarId) -> VarId {
    let cmp = ctx.fresh_var();
    ctx.emit(Inst::CallKnown(cmp, KnownFn::Gt, vec![a, b]));
    let then_block = ctx.fresh_block();
    let else_block = ctx.fresh_block();
    let join_block = ctx.fresh_block();
    ctx.finish_block(Terminator::Branch {
        cond: cmp,
        then_block,
        else_block,
    });
    ctx.start_block(then_block);
    let then_exit = ctx.current_block_id();
    ctx.finish_block(Terminator::Jump(join_block));
    ctx.start_block(else_block);
    let else_exit = ctx.current_block_id();
    ctx.finish_block(Terminator::Jump(join_block));
    ctx.start_block(join_block);
    let dst = ctx.fresh_var();
    ctx.emit_phi(dst, vec![(then_exit, a), (else_exit, b)]);
    dst
}

fn emit_min(ctx: &mut LowerCtx, a: VarId, b: VarId) -> VarId {
    let cmp = ctx.fresh_var();
    ctx.emit(Inst::CallKnown(cmp, KnownFn::Lt, vec![a, b]));
    let then_block = ctx.fresh_block();
    let else_block = ctx.fresh_block();
    let join_block = ctx.fresh_block();
    ctx.finish_block(Terminator::Branch {
        cond: cmp,
        then_block,
        else_block,
    });
    ctx.start_block(then_block);
    let then_exit = ctx.current_block_id();
    ctx.finish_block(Terminator::Jump(join_block));
    ctx.start_block(else_block);
    let else_exit = ctx.current_block_id();
    ctx.finish_block(Terminator::Jump(join_block));
    ctx.start_block(join_block);
    let dst = ctx.fresh_var();
    ctx.emit_phi(dst, vec![(then_exit, a), (else_exit, b)]);
    dst
}

// ── Quote lowering ────────────────────────────────────────────────────────────

fn lower_quote(ctx: &mut LowerCtx, form: &Form) -> R {
    match &form.kind {
        FormKind::Nil => Ok(ctx.emit_const(Const::Nil)),
        FormKind::Bool(b) => Ok(ctx.emit_const(Const::Bool(*b))),
        FormKind::Int(n) => Ok(ctx.emit_const(Const::Long(*n))),
        FormKind::Float(f) => Ok(ctx.emit_const(Const::Double(*f))),
        FormKind::Str(s) => Ok(ctx.emit_const(Const::Str(Arc::from(s.as_str())))),
        FormKind::Char(c) => Ok(ctx.emit_const(Const::Char(*c))),
        FormKind::Keyword(s) => Ok(ctx.emit_const(Const::Keyword(Arc::from(kw_local_name(s))))),
        FormKind::Symbol(s) => Ok(ctx.emit_const(Const::Symbol(Arc::from(s.as_str())))),
        FormKind::Vector(elems) => {
            let vars: Result<Vec<VarId>, _> = elems.iter().map(|e| lower_quote(ctx, e)).collect();
            let vars = vars?;
            let dst = ctx.fresh_var();
            ctx.emit(Inst::AllocVector(dst, vars));
            Ok(dst)
        }
        FormKind::List(elems) => {
            let vars: Result<Vec<VarId>, _> = elems.iter().map(|e| lower_quote(ctx, e)).collect();
            let vars = vars?;
            let dst = ctx.fresh_var();
            ctx.emit(Inst::AllocList(dst, vars));
            Ok(dst)
        }
        FormKind::Map(pairs) => {
            let mut kv: Vec<(VarId, VarId)> = Vec::with_capacity(pairs.len() / 2);
            let mut i = 0;
            while i + 1 < pairs.len() {
                let k = lower_quote(ctx, &pairs[i])?;
                let v = lower_quote(ctx, &pairs[i + 1])?;
                kv.push((k, v));
                i += 2;
            }
            let dst = ctx.fresh_var();
            ctx.emit(Inst::AllocMap(dst, kv));
            Ok(dst)
        }
        FormKind::Set(elems) => {
            let vars: Result<Vec<VarId>, _> = elems.iter().map(|e| lower_quote(ctx, e)).collect();
            let vars = vars?;
            let dst = ctx.fresh_var();
            ctx.emit(Inst::AllocSet(dst, vars));
            Ok(dst)
        }
        _ => Err(LowerError::UnsupportedForm(format!(
            "unsupported form in quote: {:?}",
            form.kind
        ))),
    }
}

// ── Symbol resolution ─────────────────────────────────────────────────────────

fn lower_symbol(ctx: &mut LowerCtx, name: &str) -> R {
    // Check locals first.
    if let Some(local) = ctx.lookup_local(name) {
        return Ok(local);
    }
    // Global reference.
    let (ns, sym_name) = split_sym(name, ctx.ns());
    let dst = ctx.fresh_var();
    ctx.emit(Inst::LoadGlobal(dst, ns, sym_name));
    Ok(dst)
}

// ── Utilities ─────────────────────────────────────────────────────────────────

/// Split `"ns/name"` into `(Arc<str>, Arc<str>)`. If there is no `/`,
/// uses the current context namespace as the namespace part.
fn split_sym(s: &str, current_ns: &Arc<str>) -> (Arc<str>, Arc<str>) {
    // Special case: "/" is the division operator, not a namespace separator.
    if s == "/" {
        return (current_ns.clone(), Arc::from("/"));
    }
    match s.find('/') {
        Some(pos) => (Arc::from(&s[..pos]), Arc::from(&s[pos + 1..])),
        None => (current_ns.clone(), Arc::from(s)),
    }
}

/// Extract the local (non-namespace) part of a keyword string.
/// `"ns/foo"` → `"foo"`, `"foo"` → `"foo"`.
fn kw_local_name(s: &str) -> &str {
    match s.rfind('/') {
        Some(pos) => &s[pos + 1..],
        None => s,
    }
}

fn malformed(msg: &str) -> LowerError {
    LowerError::MalformedSpecialForm(msg.to_string())
}

/// Expected fixed arity for known functions, or `None` for variadic/multi-arity.
fn known_fn_arity(kf: &KnownFn) -> Option<usize> {
    match kf {
        KnownFn::Get => Some(2),
        KnownFn::Count => Some(1),
        KnownFn::First => Some(1),
        KnownFn::Rest => Some(1),
        KnownFn::Next => Some(1),
        KnownFn::Assoc => Some(3),
        KnownFn::Conj => Some(2),
        KnownFn::Dissoc => Some(2),
        KnownFn::Disj => Some(2),
        KnownFn::Nth => Some(2),
        KnownFn::Contains => Some(2),
        KnownFn::Cons => Some(2),
        KnownFn::Seq => Some(1),
        KnownFn::LazySeq => Some(1),
        KnownFn::Deref => Some(1),
        KnownFn::AtomDeref => Some(1),
        KnownFn::AtomReset => Some(2),
        KnownFn::IsNil => Some(1),
        KnownFn::IsVector => Some(1),
        KnownFn::IsMap => Some(1),
        KnownFn::IsSeq => Some(1),
        KnownFn::Identical => Some(2),
        KnownFn::Pr => Some(1),
        KnownFn::Apply => Some(2),
        KnownFn::Transient => Some(1),
        KnownFn::AssocBang => Some(3),
        KnownFn::ConjBang => Some(2),
        KnownFn::PersistentBang => Some(1),
        KnownFn::TryCatchFinally => Some(3),
        KnownFn::WithOutStr => Some(1),
        KnownFn::Reduce2 => Some(2),
        KnownFn::Reduce3 => Some(3),
        KnownFn::Map => Some(2),
        KnownFn::Filter => Some(2),
        KnownFn::Mapv => Some(2),
        KnownFn::Filterv => Some(2),
        KnownFn::Some => Some(2),
        KnownFn::Every => Some(2),
        KnownFn::Into => Some(2),
        KnownFn::Into3 => Some(3),
        KnownFn::Range1 => Some(1),
        KnownFn::Range2 => Some(2),
        KnownFn::Range3 => Some(3),
        KnownFn::Take => Some(2),
        KnownFn::Drop => Some(2),
        KnownFn::Reverse => Some(1),
        KnownFn::Sort => Some(1),
        KnownFn::SortBy => Some(2),
        KnownFn::Keys => Some(1),
        KnownFn::Vals => Some(1),
        KnownFn::Update => Some(3),
        KnownFn::GetIn => Some(2),
        KnownFn::AssocIn => Some(3),
        KnownFn::IsNumber => Some(1),
        KnownFn::IsString => Some(1),
        KnownFn::IsKeyword => Some(1),
        KnownFn::IsSymbol => Some(1),
        KnownFn::IsBool => Some(1),
        KnownFn::IsInt => Some(1),
        KnownFn::Prn => Some(1),
        KnownFn::Print => Some(1),
        KnownFn::Atom => Some(1),
        KnownFn::GroupBy => Some(2),
        KnownFn::Frequencies => Some(1),
        KnownFn::Keep => Some(2),
        KnownFn::Remove => Some(2),
        KnownFn::MapIndexed => Some(2),
        KnownFn::Zipmap => Some(2),
        KnownFn::Complement => Some(1),
        KnownFn::Partition2 => Some(2),
        KnownFn::Partition3 => Some(3),
        KnownFn::Partition4 => Some(4),
        // Variadic: no fixed arity.
        _ => None,
    }
}
