//! ANF (A-Normal Form) lowering: convert `Form` AST to IR instructions.
//!
//! The lowering pass walks the AST and emits IR instructions where every
//! sub-expression is bound to a temporary variable. This makes data flow
//! explicit and enables SSA-based analysis.
//!
//! Control flow (if, loop/recur, try) produces multiple basic blocks connected
//! by terminators.

use std::collections::HashMap;
use std::sync::Arc;

use cljrs_reader::Form;
use cljrs_reader::form::FormKind;

use crate::ir::*;

/// Errors that can occur during ANF lowering.
#[derive(Debug)]
pub enum LowerError {
    /// A form that cannot (yet) be lowered to IR.
    Unsupported(String),
    /// Structural error in the input AST.
    Malformed(String),
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LowerError::Unsupported(msg) => write!(f, "unsupported form: {msg}"),
            LowerError::Malformed(msg) => write!(f, "malformed form: {msg}"),
        }
    }
}

impl std::error::Error for LowerError {}

type LowerResult<T> = Result<T, LowerError>;

// ── Known function resolution ────────────────────────────────────────────────

/// Map a symbol name (potentially namespace-qualified) to a `KnownFn`.
fn resolve_known_fn(name: &str) -> Option<KnownFn> {
    // Strip namespace prefix for core functions.
    let base = name
        .rsplit_once('/')
        .map(|(_, name)| name)
        .unwrap_or(name);

    match base {
        "vector" => Some(KnownFn::Vector),
        "hash-map" => Some(KnownFn::HashMap),
        "hash-set" => Some(KnownFn::HashSet),
        "list" => Some(KnownFn::List),
        "assoc" => Some(KnownFn::Assoc),
        "dissoc" => Some(KnownFn::Dissoc),
        "conj" => Some(KnownFn::Conj),
        "disj" => Some(KnownFn::Disj),
        "get" => Some(KnownFn::Get),
        "nth" => Some(KnownFn::Nth),
        "count" => Some(KnownFn::Count),
        "contains?" => Some(KnownFn::Contains),
        "transient" => Some(KnownFn::Transient),
        "assoc!" => Some(KnownFn::AssocBang),
        "conj!" => Some(KnownFn::ConjBang),
        "persistent!" => Some(KnownFn::PersistentBang),
        "first" => Some(KnownFn::First),
        "rest" => Some(KnownFn::Rest),
        "next" => Some(KnownFn::Next),
        "cons" => Some(KnownFn::Cons),
        "seq" => Some(KnownFn::Seq),
        "lazy-seq" => Some(KnownFn::LazySeq),
        "+" => Some(KnownFn::Add),
        "-" => Some(KnownFn::Sub),
        "*" => Some(KnownFn::Mul),
        "/" => Some(KnownFn::Div),
        "rem" | "mod" => Some(KnownFn::Rem),
        "=" => Some(KnownFn::Eq),
        "<" => Some(KnownFn::Lt),
        ">" => Some(KnownFn::Gt),
        "<=" => Some(KnownFn::Lte),
        ">=" => Some(KnownFn::Gte),
        "nil?" => Some(KnownFn::IsNil),
        "seq?" => Some(KnownFn::IsSeq),
        "vector?" => Some(KnownFn::IsVector),
        "map?" => Some(KnownFn::IsMap),
        "identical?" => Some(KnownFn::Identical),
        "str" => Some(KnownFn::Str),
        "deref" => Some(KnownFn::Deref),
        "println" => Some(KnownFn::Println),
        "pr" => Some(KnownFn::Pr),
        "apply" => Some(KnownFn::Apply),
        _ => None,
    }
}

// ── Lowering context ─────────────────────────────────────────────────────────

/// Builder for constructing an `IrFunction` from a Form AST.
pub struct AstLowering {
    func: IrFunction,
    /// Current block being built.
    current_block: BlockId,
    /// Instructions accumulated for the current block.
    current_insts: Vec<Inst>,
    /// Map from local variable names to their current VarId (for SSA).
    locals: Vec<HashMap<Arc<str>, VarId>>,
    /// The current namespace (for resolving globals).
    current_ns: Arc<str>,
    /// Stack of loop headers (block ID + phi VarIds) for recur resolution.
    loop_headers: Vec<(BlockId, Vec<VarId>)>,
}

impl AstLowering {
    /// Create a new lowering context.
    pub fn new(name: Option<Arc<str>>, ns: &str) -> Self {
        let mut func = IrFunction::new(name, None);
        let entry = func.fresh_block();
        Self {
            func,
            current_block: entry,
            current_insts: Vec::new(),
            locals: vec![HashMap::new()],
            current_ns: Arc::from(ns),
            loop_headers: Vec::new(),
        }
    }

    /// Lower a function body (list of forms) into an `IrFunction`.
    ///
    /// `params` are the function parameter names, mapped to VarIds.
    pub fn lower_function(
        mut self,
        params: &[Arc<str>],
        body: &[Form],
    ) -> LowerResult<IrFunction> {
        // Assign VarIds to parameters.
        for name in params {
            let id = self.func.fresh_var();
            self.func.params.push((Arc::clone(name), id));
            self.bind_local(name, id);
        }

        // Lower the body as a `do` block.
        let result = self.lower_body(body)?;

        // Terminate with return.
        self.finish_block(Terminator::Return(result));

        Ok(self.func)
    }

    /// Lower a sequence of forms (implicit `do`), returning the VarId of the
    /// last expression's result.
    fn lower_body(&mut self, forms: &[Form]) -> LowerResult<VarId> {
        if forms.is_empty() {
            return self.emit_const(Const::Nil);
        }
        let mut result = None;
        for form in forms {
            result = Some(self.lower_form(form)?);
        }
        Ok(result.unwrap())
    }

    /// Lower a single `Form` into IR instructions, returning the VarId
    /// holding the result.
    fn lower_form(&mut self, form: &Form) -> LowerResult<VarId> {
        match &form.kind {
            // ── Atoms ────────────────────────────────────────────────────
            FormKind::Nil => self.emit_const(Const::Nil),
            FormKind::Bool(b) => self.emit_const(Const::Bool(*b)),
            FormKind::Int(n) => self.emit_const(Const::Long(*n)),
            FormKind::Float(f) | FormKind::Symbolic(f) => self.emit_const(Const::Double(*f)),
            FormKind::Str(s) => self.emit_const(Const::Str(Arc::from(s.as_str()))),
            FormKind::Char(c) => self.emit_const(Const::Char(*c)),
            FormKind::Keyword(s) => self.emit_const(Const::Keyword(Arc::from(s.as_str()))),

            // ── Symbol lookup ────────────────────────────────────────────
            FormKind::Symbol(s) => self.lower_symbol(s),

            // ── Collections ──────────────────────────────────────────────
            FormKind::Vector(elems) => {
                let vars = self.lower_forms(elems)?;
                let dst = self.func.fresh_var();
                self.emit(Inst::AllocVector(dst, vars));
                Ok(dst)
            }
            FormKind::Map(pairs) => {
                if pairs.len() % 2 != 0 {
                    return Err(LowerError::Malformed(
                        "map literal requires even number of forms".into(),
                    ));
                }
                let mut kv_vars = Vec::with_capacity(pairs.len() / 2);
                for chunk in pairs.chunks(2) {
                    let k = self.lower_form(&chunk[0])?;
                    let v = self.lower_form(&chunk[1])?;
                    kv_vars.push((k, v));
                }
                let dst = self.func.fresh_var();
                self.emit(Inst::AllocMap(dst, kv_vars));
                Ok(dst)
            }
            FormKind::Set(elems) => {
                let vars = self.lower_forms(elems)?;
                let dst = self.func.fresh_var();
                self.emit(Inst::AllocSet(dst, vars));
                Ok(dst)
            }

            // ── List (call or special form) ──────────────────────────────
            FormKind::List(forms) => {
                if forms.is_empty() {
                    // Empty list literal
                    let dst = self.func.fresh_var();
                    self.emit(Inst::AllocList(dst, vec![]));
                    return Ok(dst);
                }
                self.lower_list(forms)
            }

            // ── Reader macros ────────────────────────────────────────────
            FormKind::Quote(inner) => self.lower_quote(inner),
            FormKind::Deref(inner) => {
                let val = self.lower_form(inner)?;
                let dst = self.func.fresh_var();
                self.emit(Inst::Deref(dst, val));
                Ok(dst)
            }

            // ── Unsupported (for now) ────────────────────────────────────
            FormKind::BigInt(_) => Err(LowerError::Unsupported("BigInt literal".into())),
            FormKind::BigDecimal(_) => Err(LowerError::Unsupported("BigDecimal literal".into())),
            FormKind::Ratio(_) => Err(LowerError::Unsupported("Ratio literal".into())),
            FormKind::Regex(_) => Err(LowerError::Unsupported("Regex literal".into())),
            FormKind::AutoKeyword(_) => Err(LowerError::Unsupported("auto-keyword".into())),
            FormKind::SyntaxQuote(_) => Err(LowerError::Unsupported("syntax-quote in IR".into())),
            FormKind::Unquote(_) => Err(LowerError::Unsupported("unquote outside syntax-quote".into())),
            FormKind::UnquoteSplice(_) => {
                Err(LowerError::Unsupported("unquote-splice outside syntax-quote".into()))
            }
            FormKind::Var(_) => Err(LowerError::Unsupported("#'var in IR".into())),
            FormKind::Meta(_, inner) => {
                // Ignore metadata, lower the annotated form.
                self.lower_form(inner)
            }
            FormKind::AnonFn(_) => Err(LowerError::Unsupported("#() should be expanded before IR lowering".into())),
            FormKind::TaggedLiteral(_, _) => Err(LowerError::Unsupported("tagged literal".into())),
            FormKind::ReaderCond { .. } => {
                Err(LowerError::Unsupported("reader conditional should be resolved before IR lowering".into()))
            }
        }
    }

    // ── List dispatch ────────────────────────────────────────────────────────

    /// Lower a non-empty list form (call or special form).
    fn lower_list(&mut self, forms: &[Form]) -> LowerResult<VarId> {
        let head = &forms[0];
        let args = &forms[1..];

        // Check for special forms.
        if let FormKind::Symbol(s) = &head.kind {
            match s.as_str() {
                "if" => return self.lower_if(args),
                "do" => return self.lower_body(args),
                "let" | "let*" => return self.lower_let(args),
                "loop" | "loop*" => return self.lower_loop(args),
                "recur" => return self.lower_recur(args),
                "def" => return self.lower_def(args),
                "fn" | "fn*" => return self.lower_fn(args),
                "quote" => {
                    if args.len() != 1 {
                        return Err(LowerError::Malformed("quote expects 1 argument".into()));
                    }
                    return self.lower_quote(&args[0]);
                }
                "throw" => return self.lower_throw(args),
                "set!" => return self.lower_set_bang(args),
                // These special forms are not lowered — they're namespace/module-level
                // operations that only execute at load time.
                "ns" | "require" | "in-ns" | "alias" | "load-file" => {
                    return Err(LowerError::Unsupported(format!("{s} (module-level only)")));
                }
                // Protocol/multimethod definitions are also module-level.
                "defprotocol" | "extend-type" | "extend-protocol" | "defmulti" | "defmethod"
                | "defrecord" | "reify" => {
                    return Err(LowerError::Unsupported(format!("{s} (not yet in IR)")));
                }
                "binding" | "with-out-str" | "try" | "letfn" => {
                    return Err(LowerError::Unsupported(format!("{s} (not yet in IR)")));
                }
                // defn desugars to (def name (fn* name ...))
                "defn" => return self.lower_defn(args),
                // defmacro, defonce not supported in AOT.
                "defmacro" | "defonce" => {
                    return Err(LowerError::Unsupported(format!(
                        "{s} (should be expanded before IR)"
                    )));
                }
                "and" => return self.lower_and(args),
                "or" => return self.lower_or(args),
                _ => {} // Fall through to function call.
            }
        }

        // Regular function call.
        self.lower_call(head, args)
    }

    // ── Special form lowering ────────────────────────────────────────────────

    /// Lower `(if test then else?)`.
    fn lower_if(&mut self, args: &[Form]) -> LowerResult<VarId> {
        if args.is_empty() || args.len() > 3 {
            return Err(LowerError::Malformed(
                "if expects 1-3 arguments".into(),
            ));
        }

        let test = self.lower_form(&args[0])?;

        let then_block = self.func.fresh_block();
        let else_block = self.func.fresh_block();
        let join_block = self.func.fresh_block();

        self.finish_block(Terminator::Branch {
            cond: test,
            then_block,
            else_block,
        });

        // Then branch.
        self.start_block(then_block);
        let then_val = if args.len() >= 2 {
            self.lower_form(&args[1])?
        } else {
            self.emit_const(Const::Nil)?
        };
        // Capture which block we're actually in (lowering may have created more blocks).
        let then_exit = self.current_block;
        self.finish_block(Terminator::Jump(join_block));

        // Else branch.
        self.start_block(else_block);
        let else_val = if args.len() >= 3 {
            self.lower_form(&args[2])?
        } else {
            self.emit_const(Const::Nil)?
        };
        let else_exit = self.current_block;
        self.finish_block(Terminator::Jump(join_block));

        // Join with phi.
        self.start_block(join_block);
        let result = self.func.fresh_var();
        self.emit_phi(result, vec![(then_exit, then_val), (else_exit, else_val)]);
        Ok(result)
    }

    /// Lower `(let [bindings...] body...)`.
    fn lower_let(&mut self, args: &[Form]) -> LowerResult<VarId> {
        if args.is_empty() {
            return Err(LowerError::Malformed("let requires a binding vector".into()));
        }
        let bindings = match &args[0].kind {
            FormKind::Vector(v) => v,
            _ => return Err(LowerError::Malformed("let bindings must be a vector".into())),
        };
        if bindings.len() % 2 != 0 {
            return Err(LowerError::Malformed(
                "let requires even number of binding forms".into(),
            ));
        }

        self.push_scope();

        for chunk in bindings.chunks(2) {
            let name = match &chunk[0].kind {
                FormKind::Symbol(s) => Arc::from(s.as_str()),
                _ => {
                    return Err(LowerError::Unsupported(
                        "destructuring in let (not yet in IR)".into(),
                    ))
                }
            };
            let val = self.lower_form(&chunk[1])?;
            self.bind_local(&name, val);
        }

        let result = self.lower_body(&args[1..])?;
        self.pop_scope();
        Ok(result)
    }

    /// Lower `(loop [bindings...] body...)`.
    fn lower_loop(&mut self, args: &[Form]) -> LowerResult<VarId> {
        if args.is_empty() {
            return Err(LowerError::Malformed("loop requires a binding vector".into()));
        }
        let bindings = match &args[0].kind {
            FormKind::Vector(v) => v,
            _ => return Err(LowerError::Malformed("loop bindings must be a vector".into())),
        };
        if bindings.len() % 2 != 0 {
            return Err(LowerError::Malformed(
                "loop requires even number of binding forms".into(),
            ));
        }

        // Evaluate initial values in the current scope.
        let mut names = Vec::new();
        let mut init_vals = Vec::new();
        for chunk in bindings.chunks(2) {
            let name = match &chunk[0].kind {
                FormKind::Symbol(s) => Arc::from(s.as_str()),
                _ => {
                    return Err(LowerError::Unsupported(
                        "destructuring in loop (not yet in IR)".into(),
                    ))
                }
            };
            let val = self.lower_form(&chunk[1])?;
            names.push(name);
            init_vals.push(val);
        }

        // Create the loop header block.
        let header = self.func.fresh_block();
        let init_block = self.current_block;
        self.finish_block(Terminator::Jump(header));

        // Start the header block with phi nodes for loop variables.
        self.start_block(header);
        self.push_scope();

        let mut phi_vars = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let phi_var = self.func.fresh_var();
            // Initially, the phi has one predecessor: the init block.
            self.emit_phi(phi_var, vec![(init_block, init_vals[i])]);
            self.bind_local(name, phi_var);
            phi_vars.push(phi_var);
        }

        // Push loop header so recur can find it.
        self.loop_headers.push((header, phi_vars.clone()));

        // Lower the body.
        let body_result = self.lower_body(&args[1..])?;
        let body_exit = self.current_block;

        // Pop loop header.
        self.loop_headers.pop();

        // The body block returns from the loop (non-recur exit).
        let exit_block = self.func.fresh_block();
        self.finish_block(Terminator::Jump(exit_block));

        // Update phi nodes: add the recur predecessor entries.
        // (Recur terminators jump back to `header` with new values;
        //  the phi update happens in lower_recur via RecurJump.)
        // For now, the phis only have the init predecessor. Recur adds more
        // in lower_recur. We store the header/phi mapping for recur to use.
        // Actually, we handle this differently: recur emits RecurJump which
        // the pass that builds the final IrFunction will convert to phi updates.

        self.pop_scope();
        self.start_block(exit_block);

        // Result is the body's last expression.
        let result = self.func.fresh_var();
        self.emit_phi(result, vec![(body_exit, body_result)]);
        Ok(result)
    }

    /// Lower `(recur args...)`.
    fn lower_recur(&mut self, args: &[Form]) -> LowerResult<VarId> {
        let arg_vars = self.lower_forms(args)?;

        let (header, _phi_vars) = self
            .loop_headers
            .last()
            .cloned()
            .ok_or_else(|| LowerError::Malformed("recur outside of loop".into()))?;

        // Add this block as a predecessor to the header's phi nodes.
        let recur_block = self.current_block;
        for (i, arg) in arg_vars.iter().enumerate() {
            // Find the phi node in the header block and add our predecessor.
            if let Some(header_block) = self.func.blocks.iter_mut().find(|b| b.id == header) {
                if let Some(Inst::Phi(_, entries)) = header_block.phis.get_mut(i) {
                    entries.push((recur_block, *arg));
                }
            }
        }

        // Terminate with RecurJump back to the loop header.
        self.finish_block(Terminator::RecurJump {
            target: header,
            args: arg_vars,
        });

        // Recur never produces a value that's used, but we need to return
        // something to satisfy the type system. Start a dead block.
        let new_block = self.func.fresh_block();
        self.start_block(new_block);
        self.emit_const(Const::Nil)
    }

    /// Lower `(def name value?)`.
    fn lower_def(&mut self, args: &[Form]) -> LowerResult<VarId> {
        if args.is_empty() {
            return Err(LowerError::Malformed("def requires a name".into()));
        }
        let name = match &args[0].kind {
            FormKind::Symbol(s) => Arc::from(s.as_str()),
            FormKind::Meta(_, inner) => match &inner.kind {
                FormKind::Symbol(s) => Arc::from(s.as_str()),
                _ => return Err(LowerError::Malformed("def requires a symbol name".into())),
            },
            _ => return Err(LowerError::Malformed("def requires a symbol name".into())),
        };

        let val = if args.len() >= 2 {
            self.lower_form(&args[1])?
        } else {
            self.emit_const(Const::Nil)?
        };

        let dst = self.func.fresh_var();
        self.emit(Inst::DefVar(dst, Arc::clone(&self.current_ns), name, val));
        Ok(dst)
    }

    /// Lower `(defn name [params] body...)` — desugars to `(def name (fn* name [params] body...))`.
    fn lower_defn(&mut self, args: &[Form]) -> LowerResult<VarId> {
        if args.is_empty() {
            return Err(LowerError::Malformed("defn requires a name".into()));
        }
        let name = match &args[0].kind {
            FormKind::Symbol(s) => s.clone(),
            _ => return Err(LowerError::Malformed("defn requires a symbol name".into())),
        };
        // Skip optional docstring.
        let rest_start = if args.len() > 2 && matches!(args[1].kind, FormKind::Str(_)) {
            2
        } else {
            1
        };
        // Build (fn* name ...) args and lower it.
        let mut fn_args = vec![args[0].clone()]; // name
        fn_args.extend_from_slice(&args[rest_start..]);
        let fn_val = self.lower_fn(&fn_args)?;

        // (def name fn_val)
        let dst = self.func.fresh_var();
        self.emit(Inst::DefVar(
            dst,
            Arc::clone(&self.current_ns),
            Arc::from(name.as_str()),
            fn_val,
        ));
        Ok(dst)
    }

    /// Lower `(fn* name? [params] body...)` or `(fn* name? ([params] body...) ...)`.
    fn lower_fn(&mut self, args: &[Form]) -> LowerResult<VarId> {
        // Parse fn* form to extract name and body forms.
        let (name, body_forms) = parse_fn_shape(args)?;

        // Determine which locals are captured.
        let mut captures = Vec::new();
        let mut capture_vars = Vec::new();
        for scope in &self.locals {
            for (name, var_id) in scope {
                if !captures.contains(name) {
                    captures.push(Arc::clone(name));
                    capture_vars.push(*var_id);
                }
            }
        }

        let tmpl = ClosureTemplate {
            name: name.clone(),
            body_forms: body_forms.to_vec(),
        };

        let dst = self.func.fresh_var();
        self.emit(Inst::AllocClosure(dst, tmpl, capture_vars));
        Ok(dst)
    }

    /// Lower `(throw expr)`.
    fn lower_throw(&mut self, args: &[Form]) -> LowerResult<VarId> {
        if args.len() != 1 {
            return Err(LowerError::Malformed("throw expects 1 argument".into()));
        }
        let val = self.lower_form(&args[0])?;
        self.emit(Inst::Throw(val));
        self.finish_block(Terminator::Unreachable);

        // Dead code after throw — start a new unreachable block.
        let new_block = self.func.fresh_block();
        self.start_block(new_block);
        self.emit_const(Const::Nil)
    }

    /// Lower `(set! var-sym value)`.
    fn lower_set_bang(&mut self, args: &[Form]) -> LowerResult<VarId> {
        if args.len() != 2 {
            return Err(LowerError::Malformed("set! expects 2 arguments".into()));
        }
        let var = self.lower_form(&args[0])?;
        let val = self.lower_form(&args[1])?;
        self.emit(Inst::SetBang(var, val));
        Ok(val)
    }

    /// Lower `(and forms...)` — short-circuiting.
    fn lower_and(&mut self, args: &[Form]) -> LowerResult<VarId> {
        if args.is_empty() {
            return self.emit_const(Const::Bool(true));
        }
        if args.len() == 1 {
            return self.lower_form(&args[0]);
        }

        let first = self.lower_form(&args[0])?;
        let rest_block = self.func.fresh_block();
        let join_block = self.func.fresh_block();

        let first_exit = self.current_block;
        self.finish_block(Terminator::Branch {
            cond: first,
            then_block: rest_block,
            else_block: join_block,
        });

        self.start_block(rest_block);
        let rest_val = self.lower_and(&args[1..])?;
        let rest_exit = self.current_block;
        self.finish_block(Terminator::Jump(join_block));

        self.start_block(join_block);
        let result = self.func.fresh_var();
        self.emit_phi(result, vec![(first_exit, first), (rest_exit, rest_val)]);
        Ok(result)
    }

    /// Lower `(or forms...)` — short-circuiting.
    fn lower_or(&mut self, args: &[Form]) -> LowerResult<VarId> {
        if args.is_empty() {
            return self.emit_const(Const::Nil);
        }
        if args.len() == 1 {
            return self.lower_form(&args[0]);
        }

        let first = self.lower_form(&args[0])?;
        let rest_block = self.func.fresh_block();
        let join_block = self.func.fresh_block();

        let first_exit = self.current_block;
        // or: if truthy, short-circuit; otherwise try rest.
        self.finish_block(Terminator::Branch {
            cond: first,
            then_block: join_block,  // truthy → done
            else_block: rest_block,  // falsy → try rest
        });

        self.start_block(rest_block);
        let rest_val = self.lower_or(&args[1..])?;
        let rest_exit = self.current_block;
        self.finish_block(Terminator::Jump(join_block));

        self.start_block(join_block);
        let result = self.func.fresh_var();
        self.emit_phi(result, vec![(first_exit, first), (rest_exit, rest_val)]);
        Ok(result)
    }

    // ── Call lowering ────────────────────────────────────────────────────────

    /// Lower a function call.
    fn lower_call(&mut self, callee_form: &Form, arg_forms: &[Form]) -> LowerResult<VarId> {
        // Check if callee is a known function.
        if let FormKind::Symbol(s) = &callee_form.kind
            && let Some(known) = resolve_known_fn(s)
        {
            let args = self.lower_forms(arg_forms)?;
            let dst = self.func.fresh_var();
            self.emit(Inst::CallKnown(dst, known, args));
            return Ok(dst);
        }

        // Unknown call: lower callee and args.
        let callee = self.lower_form(callee_form)?;
        let args = self.lower_forms(arg_forms)?;
        let dst = self.func.fresh_var();
        self.emit(Inst::Call(dst, callee, args));
        Ok(dst)
    }

    // ── Quote lowering ───────────────────────────────────────────────────────

    /// Lower a quoted form to a constant. Only handles simple cases;
    /// complex quoted data structures would need recursive construction.
    fn lower_quote(&mut self, form: &Form) -> LowerResult<VarId> {
        match &form.kind {
            FormKind::Nil => self.emit_const(Const::Nil),
            FormKind::Bool(b) => self.emit_const(Const::Bool(*b)),
            FormKind::Int(n) => self.emit_const(Const::Long(*n)),
            FormKind::Float(f) | FormKind::Symbolic(f) => self.emit_const(Const::Double(*f)),
            FormKind::Str(s) => self.emit_const(Const::Str(Arc::from(s.as_str()))),
            FormKind::Char(c) => self.emit_const(Const::Char(*c)),
            FormKind::Keyword(s) => self.emit_const(Const::Keyword(Arc::from(s.as_str()))),
            FormKind::Symbol(s) => self.emit_const(Const::Symbol(Arc::from(s.as_str()))),
            // Quoted collections: build them at runtime.
            FormKind::Vector(elems) => {
                let vars = elems
                    .iter()
                    .map(|e| self.lower_quote(e))
                    .collect::<LowerResult<Vec<_>>>()?;
                let dst = self.func.fresh_var();
                self.emit(Inst::AllocVector(dst, vars));
                Ok(dst)
            }
            FormKind::List(elems) => {
                let vars = elems
                    .iter()
                    .map(|e| self.lower_quote(e))
                    .collect::<LowerResult<Vec<_>>>()?;
                let dst = self.func.fresh_var();
                self.emit(Inst::AllocList(dst, vars));
                Ok(dst)
            }
            FormKind::Map(pairs) => {
                let mut kv_vars = Vec::new();
                for chunk in pairs.chunks(2) {
                    let k = self.lower_quote(&chunk[0])?;
                    let v = self.lower_quote(&chunk[1])?;
                    kv_vars.push((k, v));
                }
                let dst = self.func.fresh_var();
                self.emit(Inst::AllocMap(dst, kv_vars));
                Ok(dst)
            }
            FormKind::Set(elems) => {
                let vars = elems
                    .iter()
                    .map(|e| self.lower_quote(e))
                    .collect::<LowerResult<Vec<_>>>()?;
                let dst = self.func.fresh_var();
                self.emit(Inst::AllocSet(dst, vars));
                Ok(dst)
            }
            _ => Err(LowerError::Unsupported(format!(
                "quoted form: {:?}",
                form.kind
            ))),
        }
    }

    // ── Symbol resolution ────────────────────────────────────────────────────

    /// Lower a symbol reference — look up in locals first, then globals.
    fn lower_symbol(&mut self, name: &str) -> LowerResult<VarId> {
        // Check locals (innermost scope first).
        let arc_name: Arc<str> = Arc::from(name);
        for scope in self.locals.iter().rev() {
            if let Some(&var_id) = scope.get(&arc_name) {
                return Ok(var_id);
            }
        }

        // It's a global reference.
        let (ns, sym_name) = if let Some((ns, n)) = name.rsplit_once('/') {
            (Arc::from(ns), Arc::from(n))
        } else {
            (Arc::clone(&self.current_ns), Arc::from(name))
        };

        let dst = self.func.fresh_var();
        self.emit(Inst::LoadGlobal(dst, ns, sym_name));
        Ok(dst)
    }

    // ── Helper methods ───────────────────────────────────────────────────────

    /// Lower a slice of forms, returning their VarIds.
    fn lower_forms(&mut self, forms: &[Form]) -> LowerResult<Vec<VarId>> {
        forms.iter().map(|f| self.lower_form(f)).collect()
    }

    /// Emit a constant, returning a fresh VarId.
    fn emit_const(&mut self, c: Const) -> LowerResult<VarId> {
        let dst = self.func.fresh_var();
        self.emit(Inst::Const(dst, c));
        Ok(dst)
    }

    /// Emit an instruction into the current block.
    fn emit(&mut self, inst: Inst) {
        self.current_insts.push(inst);
    }

    /// Emit a phi node (goes into the phis list of the current block).
    fn emit_phi(&mut self, dst: VarId, entries: Vec<(BlockId, VarId)>) {
        // Phis are stored separately — we emit them at block start.
        // But since we're building the block incrementally, store as a
        // regular instruction for now and separate later in finish_block.
        self.current_insts.push(Inst::Phi(dst, entries));
    }

    /// Finalize the current block with a terminator.
    fn finish_block(&mut self, terminator: Terminator) {
        let insts = std::mem::take(&mut self.current_insts);

        // Separate phis from regular instructions.
        let mut phis = Vec::new();
        let mut regular = Vec::new();
        for inst in insts {
            if matches!(inst, Inst::Phi(..)) {
                phis.push(inst);
            } else {
                regular.push(inst);
            }
        }

        self.func.blocks.push(Block {
            id: self.current_block,
            phis,
            insts: regular,
            terminator,
        });
    }

    /// Start building a new block.
    fn start_block(&mut self, id: BlockId) {
        self.current_block = id;
        debug_assert!(self.current_insts.is_empty());
    }

    /// Push a new scope for local bindings.
    fn push_scope(&mut self) {
        self.locals.push(HashMap::new());
    }

    /// Pop the innermost scope.
    fn pop_scope(&mut self) {
        self.locals.pop();
    }

    /// Bind a local variable name to a VarId in the current scope.
    fn bind_local(&mut self, name: &Arc<str>, var_id: VarId) {
        if let Some(scope) = self.locals.last_mut() {
            scope.insert(Arc::clone(name), var_id);
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Parse the shape of an `fn*` form to extract the name and body.
fn parse_fn_shape(args: &[Form]) -> LowerResult<(Option<Arc<str>>, &[Form])> {
    if args.is_empty() {
        return Err(LowerError::Malformed("fn* requires arguments".into()));
    }

    // fn* may have an optional name as the first arg.
    let (name, rest) = if let FormKind::Symbol(s) = &args[0].kind {
        (Some(Arc::from(s.as_str())), &args[1..])
    } else {
        (None, args)
    };

    Ok((name, rest))
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Lower a function body to IR.
///
/// `name` is the function name (if any), `ns` is the current namespace,
/// `params` are parameter names, and `body` is the list of body forms.
pub fn lower_fn_body(
    name: Option<&str>,
    ns: &str,
    params: &[Arc<str>],
    body: &[Form],
) -> LowerResult<IrFunction> {
    let lowering = AstLowering::new(name.map(Arc::from), ns);
    lowering.lower_function(params, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cljrs_reader::Parser;

    /// Parse a Clojure expression and return the Form.
    fn parse(src: &str) -> Form {
        let mut parser = Parser::new(src.to_string(), "<test>".to_string());
        parser.parse_one().expect("parse error").expect("empty input")
    }

    /// Parse the body of a `(fn [params] body)` and lower it.
    fn lower_test_fn(src: &str) -> IrFunction {
        let form = parse(src);
        match &form.kind {
            FormKind::List(forms) => {
                // Expect (fn [params] body...)
                assert!(forms.len() >= 3, "expected (fn [params] body...)");
                let params_form = &forms[1];
                let params: Vec<Arc<str>> = match &params_form.kind {
                    FormKind::Vector(v) => v
                        .iter()
                        .map(|f| match &f.kind {
                            FormKind::Symbol(s) => Arc::from(s.as_str()),
                            _ => panic!("expected symbol in params"),
                        })
                        .collect(),
                    _ => panic!("expected vector for params"),
                };
                let body = &forms[2..];
                lower_fn_body(None, "user", &params, body).expect("lower error")
            }
            _ => panic!("expected list form"),
        }
    }

    #[test]
    fn test_lower_const() {
        let ir = lower_test_fn("(fn [] 42)");
        assert_eq!(ir.blocks.len(), 1);
        assert_eq!(ir.params.len(), 0);
        // Should have: Const(v0, Long(42)), Return(v0)
        assert_eq!(ir.blocks[0].insts.len(), 1);
        match &ir.blocks[0].insts[0] {
            Inst::Const(_, Const::Long(42)) => {}
            other => panic!("expected Const(Long(42)), got {other:?}"),
        }
        match &ir.blocks[0].terminator {
            Terminator::Return(v) => assert_eq!(*v, VarId(0)),
            other => panic!("expected Return, got {other:?}"),
        }
    }

    #[test]
    fn test_lower_param_ref() {
        let ir = lower_test_fn("(fn [x] x)");
        assert_eq!(ir.params.len(), 1);
        assert_eq!(ir.params[0].0.as_ref(), "x");
        // Body should just return the param var.
        match &ir.blocks[0].terminator {
            Terminator::Return(v) => assert_eq!(*v, ir.params[0].1),
            other => panic!("expected Return of param, got {other:?}"),
        }
    }

    #[test]
    fn test_lower_if() {
        let ir = lower_test_fn("(fn [x] (if x 1 2))");
        // Should produce: entry(branch), then, else, join(phi + return)
        assert!(ir.blocks.len() >= 4, "expected at least 4 blocks for if");
        // Entry block should end with a Branch.
        match &ir.blocks[0].terminator {
            Terminator::Branch { .. } => {}
            other => panic!("expected Branch terminator, got {other:?}"),
        }
    }

    #[test]
    fn test_lower_let() {
        let ir = lower_test_fn("(fn [x] (let [y (+ x 1)] y))");
        // Should bind y to result of (+ x 1), then return y.
        let insts = &ir.blocks[0].insts;
        // Find the CallKnown for Add.
        let has_add = insts
            .iter()
            .any(|i| matches!(i, Inst::CallKnown(_, KnownFn::Add, _)));
        assert!(has_add, "expected CallKnown(Add) in instructions");
    }

    #[test]
    fn test_lower_vector_literal() {
        let ir = lower_test_fn("(fn [] [1 2 3])");
        let insts = &ir.blocks[0].insts;
        let has_alloc = insts.iter().any(|i| matches!(i, Inst::AllocVector(..)));
        assert!(has_alloc, "expected AllocVector in instructions");
    }

    #[test]
    fn test_lower_known_call() {
        let ir = lower_test_fn("(fn [m] (assoc m :a 1))");
        let insts = &ir.blocks[0].insts;
        let has_assoc = insts
            .iter()
            .any(|i| matches!(i, Inst::CallKnown(_, KnownFn::Assoc, _)));
        assert!(has_assoc, "expected CallKnown(Assoc)");
    }

    #[test]
    fn test_lower_unknown_call() {
        let ir = lower_test_fn("(fn [f x] (f x))");
        let insts = &ir.blocks[0].insts;
        let has_call = insts.iter().any(|i| matches!(i, Inst::Call(..)));
        assert!(has_call, "expected Call (unknown) in instructions");
    }

    #[test]
    fn test_lower_and() {
        let ir = lower_test_fn("(fn [a b] (and a b))");
        // and produces branching.
        assert!(ir.blocks.len() >= 3, "expected multiple blocks for and");
    }

    #[test]
    fn test_lower_nested_let() {
        let ir = lower_test_fn("(fn [x] (let [a 1 b 2] (+ a b)))");
        // Should produce constants and an Add call.
        let all_insts: Vec<_> = ir.blocks.iter().flat_map(|b| b.insts.iter()).collect();
        let has_add = all_insts
            .iter()
            .any(|i| matches!(i, Inst::CallKnown(_, KnownFn::Add, _)));
        assert!(has_add, "expected CallKnown(Add)");
    }

    #[test]
    fn test_display_ir() {
        let ir = lower_test_fn("(fn [x] (if x (+ x 1) 0))");
        let output = format!("{ir}");
        assert!(output.contains("branch"), "expected 'branch' in display output");
        assert!(
            output.contains("call_known"),
            "expected 'call_known' in display output"
        );
    }

    #[test]
    fn test_lower_assoc_chain() {
        // Pattern that escape analysis should optimize: intermediate maps don't escape.
        let ir = lower_test_fn(
            "(fn [m] (let [a (assoc m :x 1) b (assoc a :y 2) c (assoc b :z 3)] c))",
        );
        // Should see 3 CallKnown(Assoc) instructions.
        let all_insts: Vec<_> = ir.blocks.iter().flat_map(|b| b.insts.iter()).collect();
        let assoc_count = all_insts
            .iter()
            .filter(|i| matches!(i, Inst::CallKnown(_, KnownFn::Assoc, _)))
            .count();
        assert_eq!(assoc_count, 3, "expected 3 assoc calls in chain");
    }

    #[test]
    fn test_lower_map_literal() {
        let ir = lower_test_fn("(fn [] {:a 1 :b 2})");
        let insts = &ir.blocks[0].insts;
        let has_alloc_map = insts.iter().any(|i| matches!(i, Inst::AllocMap(..)));
        assert!(has_alloc_map, "expected AllocMap in instructions");
    }

    #[test]
    fn test_lower_loop_recur() {
        let ir = lower_test_fn("(fn [n] (loop [i 0 acc 0] (if (= i n) acc (recur (+ i 1) (+ acc i)))))");
        // Should have multiple blocks for loop header, body, branches.
        assert!(ir.blocks.len() >= 4, "expected at least 4 blocks for loop");
        // Should have phi nodes in the header block (block after entry).
        let has_phi = ir.blocks.iter().any(|b| !b.phis.is_empty());
        assert!(has_phi, "expected phi nodes in loop header");
    }
}
