//! Conservative capture analysis for tree-walking functions.
//!
//! Unlike IR bodies, these forms have not been macro-expanded. Expansion is
//! deliberately left until execution: it may have side effects and may inspect
//! `&env`. Keep the complete environment when a call could expand a macro.

use std::collections::HashSet;
use std::sync::Arc;

use cljrs_reader::{Form, form::FormKind};
use cljrs_value::{CljxFnArity, Symbol, Value};

use crate::env::env::Env;

pub(super) fn capture_locals(
    arities: &[CljxFnArity],
    name: Option<&str>,
    env: &Env,
) -> (Vec<Arc<str>>, Vec<Value>) {
    let (names, vals) = env.all_local_bindings();
    if names.is_empty() {
        return (names, vals);
    }
    let mut scan = CaptureScan {
        env,
        bound: Vec::new(),
        referenced: HashSet::new(),
    };
    for arity in arities {
        // Defaults in destructuring patterns run before the body, and can
        // themselves expand macros. Preserve their environment conservatively.
        if !arity.destructure_params.is_empty() || arity.destructure_rest.is_some() {
            return (names, vals);
        }
        scan.bound.clear();
        scan.bound.extend(name.map(str::to_owned));
        scan.bound
            .extend(arity.params.iter().map(|p| p.to_string()));
        scan.bound
            .extend(arity.rest_param.iter().map(|p| p.to_string()));
        if !scan.forms(&arity.body) {
            return (names, vals);
        }
    }
    names
        .into_iter()
        .zip(vals)
        // Only the innermost value of a referenced name is visible.
        .rev()
        .filter(|(n, _)| scan.referenced.remove(n.as_ref()))
        .unzip()
}

struct CaptureScan<'a> {
    env: &'a Env,
    bound: Vec<String>,
    referenced: HashSet<String>,
}

impl CaptureScan<'_> {
    fn is_bound(&self, name: &str) -> bool {
        self.bound.iter().any(|n| n == name)
    }

    fn forms(&mut self, forms: &[Form]) -> bool {
        forms.iter().all(|f| self.form(f))
    }

    /// `false` means the form may refer to locals not explicit in its syntax.
    fn form(&mut self, form: &Form) -> bool {
        match &form.kind {
            FormKind::Symbol(s) => {
                if !self.is_bound(s) {
                    self.referenced.insert(s.clone());
                }
                true
            }
            FormKind::List(parts) => self.list(parts),
            FormKind::Vector(parts) | FormKind::Map(parts) | FormKind::Set(parts) => {
                self.forms(parts)
            }
            FormKind::Deref(inner) => self.form(inner),
            FormKind::Quote(_) | FormKind::Var(_) => true,
            FormKind::Meta(meta, inner) => self.form(meta) && self.form(inner),
            FormKind::ReaderCond { clauses, .. } => self.forms(clauses),
            FormKind::AnonFn(body) => self.form(&crate::builtins::form::expand_anon_fn(
                body,
                form.span.clone(),
            )),
            // Syntax quotes, tagged readers, and unquotes are intentionally
            // conservative; their evaluation is not ordinary symbol lookup.
            FormKind::SyntaxQuote(_)
            | FormKind::Unquote(_)
            | FormKind::UnquoteSplice(_)
            | FormKind::TaggedLiteral(_, _) => false,
            _ => true,
        }
    }

    fn list(&mut self, parts: &[Form]) -> bool {
        let Some(head) = parts.first() else {
            return true;
        };
        let args = &parts[1..];
        match head.as_symbol() {
            Some("quote" | "var") => true,
            Some("fn" | "fn*") => self.function(args),
            Some("let" | "let*" | "loop" | "loop*") => {
                let Some(bindings) = args.first().and_then(Form::as_vector) else {
                    return false;
                };
                let depth = self.bound.len();
                let (pairs, remainder) = bindings.as_chunks::<2>();
                let mut safe = remainder.is_empty();
                for [pattern, initializer] in pairs {
                    // Initializers see earlier bindings, including an outer
                    // binding of their own name until this pair is installed.
                    safe = safe && self.form(initializer) && self.bind(pattern);
                }
                safe = safe && self.forms(&args[1..]);
                self.bound.truncate(depth);
                safe
            }
            Some("if" | "do" | "recur" | "throw" | "and" | "or" | "def" | "defonce") => {
                self.forms(args)
            }
            // Other special forms can introduce bindings or implicit local
            // accesses (e.g. set! uses the hidden mutable-field receiver).
            Some(s) if super::eval::is_special_form(s) => false,
            Some(s) if !self.is_bound(s) => {
                let sym = Symbol::parse(s);
                let callee = self.env.lookup(s).or_else(|| {
                    let ns = self.env.resolve_ns_part(sym.namespace.as_deref()?);
                    self.env.globals.lookup_in_ns(&ns, &sym.name)
                });
                // A macro can manufacture a reference absent from the source
                // or inspect any surrounding local through &env. Unknown and
                // computed callees can also turn out to be macros at runtime.
                matches!(
                    callee.as_ref().map(Value::unwrap_meta),
                    Some(Value::Fn(_) | Value::NativeFunction(_) | Value::Keyword(_))
                ) && self.forms(parts)
            }
            _ => false,
        }
    }

    fn bind(&mut self, pattern: &Form) -> bool {
        if let Some(s) = pattern.as_symbol() {
            self.bound.push(s.to_owned());
            true
        } else {
            false
        }
    }

    fn function(&mut self, args: &[Form]) -> bool {
        let depth = self.bound.len();
        let args = if let Some(name) = args.first().and_then(Form::as_symbol) {
            self.bound.push(name.to_owned());
            &args[1..]
        } else {
            args
        };
        let safe = if args.first().and_then(Form::as_vector).is_some() {
            self.arity(args)
        } else {
            args.iter()
                .all(|a| a.as_list().is_some_and(|a| self.arity(a)))
        };
        self.bound.truncate(depth);
        safe
    }

    fn arity(&mut self, forms: &[Form]) -> bool {
        let Some(params) = forms.first().and_then(Form::as_vector) else {
            return false;
        };
        let depth = self.bound.len();
        let safe = params.iter().all(|p| self.bind(p)) && self.forms(&forms[1..]);
        self.bound.truncate(depth);
        safe
    }
}
