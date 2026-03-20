//! Persistent structure virtualization for the interpreter.
//!
//! Detects patterns in `let` bindings where a sequence of `assoc` or `conj`
//! calls builds a collection through a chain of intermediate values that are
//! never used elsewhere. When detected, the chain is evaluated using transient
//! (mutable) operations internally, avoiding N intermediate persistent
//! collection allocations.
//!
//! Example pattern:
//! ```clojure
//! (let [a (assoc m :x 1)
//!       b (assoc a :y 2)
//!       c (assoc b :z 3)]
//!   c)
//! ```
//! Optimized to internally: `transient(m) → assoc! :x 1 → assoc! :y 2 → assoc! :z 3 → persistent!`

use cljrs_reader::Form;
use cljrs_reader::form::FormKind;

/// An assoc/conj chain detected in let bindings.
#[derive(Debug)]
pub struct LetChain {
    /// Index of the first binding in the chain (0-based, in pairs).
    pub start: usize,
    /// Number of bindings in the chain.
    pub len: usize,
    /// The operation kind for each step.
    pub ops: Vec<ChainOpKind>,
}

/// The kind of collection-building operation in a chain step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainOpKind {
    /// `(assoc coll key val)` or `(assoc coll k1 v1 k2 v2 ...)`
    Assoc,
    /// `(conj coll val)` or `(conj coll v1 v2 ...)`
    Conj,
}

/// Detect assoc/conj chains in a let binding vector.
///
/// `bindings` is the flat `[name expr name expr ...]` vector from the let form.
/// Returns chains of length >= 2 that can be optimized.
pub fn detect_let_chains(bindings: &[Form]) -> Vec<LetChain> {
    if bindings.len() < 4 {
        // Need at least 2 binding pairs for a chain.
        return vec![];
    }

    let mut chains = Vec::new();
    let pairs: Vec<(&Form, &Form)> = bindings.chunks(2).map(|c| (&c[0], &c[1])).collect();

    let mut i = 0;
    while i < pairs.len() {
        // Try to start a chain at pair `i`.
        if let Some(chain) = try_start_chain(&pairs, i)
            && chain.len >= 2
        {
            let end = chain.start + chain.len;
            chains.push(chain);
            i = end; // Skip past the chain.
            continue;
        }
        i += 1;
    }

    chains
}

/// Try to build a chain starting at pair index `start`.
fn try_start_chain(pairs: &[(&Form, &Form)], start: usize) -> Option<LetChain> {
    let (first_name, first_expr) = pairs[start];

    // The first binding must be an assoc/conj call.
    let first_op = classify_call(first_expr)?;

    // Get the binding name as a string for matching.
    let first_name_str = symbol_name(first_name)?;

    let mut ops = vec![first_op];
    let mut prev_name = first_name_str;

    // Extend the chain.
    for &(name, expr) in &pairs[(start + 1)..] {
        let op = match classify_call(expr) {
            Some(op) => op,
            None => break,
        };

        // The collection argument (first arg of assoc/conj) must be the
        // previous binding's name.
        if !call_collection_arg_is(expr, prev_name) {
            break;
        }

        prev_name = match symbol_name(name) {
            Some(s) => s,
            None => break,
        };
        ops.push(op);
    }

    if ops.len() >= 2 {
        Some(LetChain {
            start,
            len: ops.len(),
            ops,
        })
    } else {
        None
    }
}

/// Classify a call form as Assoc or Conj (if it is one).
fn classify_call(expr: &Form) -> Option<ChainOpKind> {
    let forms = match &expr.kind {
        FormKind::List(forms) if !forms.is_empty() => forms,
        _ => return None,
    };

    let head = match &forms[0].kind {
        FormKind::Symbol(s) => s.as_str(),
        _ => return None,
    };

    match head {
        "assoc" | "clojure.core/assoc" if forms.len() >= 4 => Some(ChainOpKind::Assoc),
        "conj" | "clojure.core/conj" if forms.len() >= 3 => Some(ChainOpKind::Conj),
        _ => None,
    }
}

/// Check if the first argument of a call form (the collection) is a symbol
/// matching `name`.
fn call_collection_arg_is(expr: &Form, name: &str) -> bool {
    let forms = match &expr.kind {
        FormKind::List(forms) if forms.len() >= 2 => forms,
        _ => return false,
    };
    matches!(&forms[1].kind, FormKind::Symbol(s) if s == name)
}

/// Extract a simple symbol name from a form.
fn symbol_name(form: &Form) -> Option<&str> {
    match &form.kind {
        FormKind::Symbol(s) => Some(s.as_str()),
        _ => None,
    }
}

/// Check if a binding name is used anywhere in the body forms (outside the chain).
///
/// If a chain intermediate is used in the body, we can't virtualize it because
/// the user needs the persistent version.
pub fn binding_used_in_body(name: &str, body: &[Form]) -> bool {
    body.iter().any(|f| form_references_symbol(f, name))
}

/// Check if a binding name is used in any let bindings outside the chain.
pub fn binding_used_in_other_bindings(
    name: &str,
    bindings: &[Form],
    chain_start: usize,
    chain_len: usize,
) -> bool {
    let chain_end_pair = chain_start + chain_len;
    for (i, chunk) in bindings.chunks(2).enumerate() {
        if i >= chain_start && i < chain_end_pair {
            // Skip the chain itself — references within the chain are expected.
            // But check non-collection-arg positions.
            if i > chain_start {
                // For chain members after the first, only the collection arg (index 1)
                // should reference the previous name. If the name appears elsewhere
                // in the expression, it's an additional use.
                if let FormKind::List(forms) = &chunk[1].kind {
                    // Check args after the collection arg.
                    for arg_form in forms.iter().skip(2) {
                        if form_references_symbol(arg_form, name) {
                            return true;
                        }
                    }
                }
            }
            continue;
        }
        // Check the value expression of this binding.
        if chunk.len() >= 2 && form_references_symbol(&chunk[1], name) {
            return true;
        }
    }
    false
}

/// Recursively check if a form references a symbol by name.
fn form_references_symbol(form: &Form, name: &str) -> bool {
    match &form.kind {
        FormKind::Symbol(s) => s == name,
        FormKind::List(forms)
        | FormKind::Vector(forms)
        | FormKind::Set(forms)
        | FormKind::Map(forms) => forms.iter().any(|f| form_references_symbol(f, name)),
        FormKind::Quote(_) => false, // Quoted forms don't reference live bindings.
        FormKind::SyntaxQuote(inner)
        | FormKind::Unquote(inner)
        | FormKind::UnquoteSplice(inner)
        | FormKind::Deref(inner)
        | FormKind::Var(inner) => form_references_symbol(inner, name),
        FormKind::Meta(m, inner) => {
            form_references_symbol(m, name) || form_references_symbol(inner, name)
        }
        FormKind::AnonFn(forms) => forms.iter().any(|f| form_references_symbol(f, name)),
        FormKind::TaggedLiteral(_, inner) => form_references_symbol(inner, name),
        FormKind::ReaderCond { clauses, .. } => {
            clauses.iter().any(|f| form_references_symbol(f, name))
        }
        _ => false, // Atoms, keywords, etc.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cljrs_reader::Parser;

    fn parse_bindings(src: &str) -> Vec<Form> {
        // Parse a let form and extract the binding vector.
        let mut parser = Parser::new(src.to_string(), "<test>".to_string());
        let form = parser.parse_one().unwrap().unwrap();
        match form.kind {
            FormKind::List(forms) => match &forms[1].kind {
                FormKind::Vector(v) => v.clone(),
                _ => panic!("expected vector"),
            },
            _ => panic!("expected list"),
        }
    }

    #[test]
    fn test_detect_assoc_chain() {
        let bindings =
            parse_bindings("(let [a (assoc m :x 1) b (assoc a :y 2) c (assoc b :z 3)] c)");
        let chains = detect_let_chains(&bindings);
        assert_eq!(chains.len(), 1);
        assert_eq!(chains[0].start, 0);
        assert_eq!(chains[0].len, 3);
        assert!(chains[0].ops.iter().all(|op| *op == ChainOpKind::Assoc));
    }

    #[test]
    fn test_detect_conj_chain() {
        let bindings = parse_bindings("(let [a (conj v 1) b (conj a 2) c (conj b 3)] c)");
        let chains = detect_let_chains(&bindings);
        assert_eq!(chains.len(), 1);
        assert_eq!(chains[0].len, 3);
        assert!(chains[0].ops.iter().all(|op| *op == ChainOpKind::Conj));
    }

    #[test]
    fn test_no_chain_too_short() {
        let bindings = parse_bindings("(let [a (assoc m :x 1)] a)");
        let chains = detect_let_chains(&bindings);
        assert!(chains.is_empty());
    }

    #[test]
    fn test_no_chain_different_names() {
        // b doesn't reference a — not a chain.
        let bindings = parse_bindings("(let [a (assoc m :x 1) b (assoc m :y 2)] b)");
        let chains = detect_let_chains(&bindings);
        assert!(chains.is_empty());
    }

    #[test]
    fn test_mixed_chain() {
        let bindings = parse_bindings("(let [a (assoc m :x 1) b (conj a 2) c (assoc b :z 3)] c)");
        let chains = detect_let_chains(&bindings);
        assert_eq!(chains.len(), 1);
        assert_eq!(chains[0].len, 3);
    }

    #[test]
    fn test_body_reference_detected() {
        let mut parser = Parser::new("(println a)".to_string(), "<test>".to_string());
        let form = parser.parse_one().unwrap().unwrap();
        assert!(binding_used_in_body("a", &[form]));

        let mut parser = Parser::new("(println b)".to_string(), "<test>".to_string());
        let form = parser.parse_one().unwrap().unwrap();
        assert!(!binding_used_in_body("a", &[form]));
    }
}
