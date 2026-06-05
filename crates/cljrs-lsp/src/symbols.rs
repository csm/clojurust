//! Extract [`DocumentSymbol`]s (an outline of definitions) from parsed forms.
//!
//! Forms come from a single recovered chunk, so their spans are relative to the
//! chunk substring; callers pass the chunk's byte `delta` to shift spans back
//! to document coordinates.

use cljrs_reader::{Form, FormKind};
use tower_lsp::lsp_types::{DocumentSymbol, SymbolKind};

use crate::line_index::{LineIndex, OffsetEncoding};

/// Collect definition symbols from a chunk's forms into `out`.
pub fn collect(
    forms: &[Form],
    delta: usize,
    li: &LineIndex,
    enc: OffsetEncoding,
    out: &mut Vec<DocumentSymbol>,
) {
    for form in forms {
        collect_form(form, delta, li, enc, None, out);
    }
}

fn collect_form(
    form: &Form,
    delta: usize,
    li: &LineIndex,
    enc: OffsetEncoding,
    platform: Option<&str>,
    out: &mut Vec<DocumentSymbol>,
) {
    match &form.kind {
        // `^meta (def ...)` — descend into the annotated target.
        FormKind::Meta(_, target) => collect_form(target, delta, li, enc, platform, out),
        // Surface definitions from every reader-conditional branch, tagging the
        // platform in `detail`. Clauses are flat `[kw, form, kw, form, ...]`.
        FormKind::ReaderCond { clauses, .. } => {
            for pair in clauses.chunks(2) {
                if let [key, body] = pair {
                    let label = match &key.kind {
                        FormKind::Keyword(k) => Some(k.as_str()),
                        _ => None,
                    };
                    collect_form(body, delta, li, enc, label, out);
                }
            }
        }
        FormKind::List(items) => {
            if let Some(sym) = def_symbol(form, items, delta, li, enc, platform) {
                out.push(sym);
            }
        }
        _ => {}
    }
}

/// Map a `(head name ...)` list to a [`DocumentSymbol`], or `None` if `head`
/// is not a recognized defining form or `name` is missing/not a symbol.
fn def_symbol(
    form: &Form,
    items: &[Form],
    delta: usize,
    li: &LineIndex,
    enc: OffsetEncoding,
    platform: Option<&str>,
) -> Option<DocumentSymbol> {
    let head = match items.first().map(|f| &f.kind) {
        Some(FormKind::Symbol(s)) => s.as_str(),
        _ => return None,
    };

    let kind = match head {
        "ns" => SymbolKind::NAMESPACE,
        "def" | "defonce" => SymbolKind::VARIABLE,
        "defn" | "defn-" | "defmacro" | "defmulti" | "deftest" | "definline" => {
            SymbolKind::FUNCTION
        }
        "defprotocol" => SymbolKind::INTERFACE,
        "defrecord" | "deftype" | "defstruct" => SymbolKind::STRUCT,
        _ => return None,
    };

    let name_form = items.get(1)?;
    let name = match &name_form.kind {
        FormKind::Symbol(s) => s.clone(),
        _ => return None,
    };

    let detail = match platform {
        Some(p) => format!("{head} (:{p})"),
        None => head.to_string(),
    };

    let range = li.range(form.span.start + delta, form.span.end + delta, enc);
    let selection_range = li.range(
        name_form.span.start + delta,
        name_form.span.end + delta,
        enc,
    );

    #[allow(deprecated)] // `deprecated` field is required by the struct literal.
    Some(DocumentSymbol {
        name,
        detail: Some(detail),
        kind,
        tags: None,
        deprecated: None,
        range,
        selection_range,
        children: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cljrs_reader::Parser;

    fn symbols(src: &str) -> Vec<DocumentSymbol> {
        let li = LineIndex::new(src);
        let forms = Parser::new(src.to_string(), "<test>".to_string())
            .parse_all()
            .expect("parse");
        let mut out = Vec::new();
        collect(&forms, 0, &li, OffsetEncoding::Utf8, &mut out);
        out
    }

    #[test]
    fn extracts_common_defs() {
        let src = "(ns my.app)\n(def x 1)\n(defn f [a] a)\n(defn- g [] 0)\n\
                   (defmacro m [] nil)\n(defonce o 1)\n(defprotocol P)\n\
                   (defrecord R [a])\n(defmulti d :k)\n(deftest t)";
        let syms = symbols(src);
        let names: Vec<_> = syms.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["my.app", "x", "f", "g", "m", "o", "P", "R", "d", "t"]
        );
        assert_eq!(syms[0].kind, SymbolKind::NAMESPACE);
        assert_eq!(syms[1].kind, SymbolKind::VARIABLE);
        assert_eq!(syms[2].kind, SymbolKind::FUNCTION);
        assert_eq!(syms[6].kind, SymbolKind::INTERFACE);
        assert_eq!(syms[7].kind, SymbolKind::STRUCT);
    }

    #[test]
    fn selection_within_range() {
        let syms = symbols("(defn foo [x] x)");
        let s = &syms[0];
        assert!(s.selection_range.start >= s.range.start);
        assert!(s.selection_range.end <= s.range.end);
        assert_eq!(s.name, "foo");
    }

    #[test]
    fn malformed_def_emits_nothing() {
        assert!(symbols("(def)").is_empty());
        assert!(symbols("(defn 42 [])").is_empty());
    }

    #[test]
    fn reader_conditional_defs() {
        let syms = symbols("#?(:rust (defn r [] 1) :clj (defn c [] 2))");
        let names: Vec<_> = syms.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["r", "c"]);
        assert_eq!(syms[0].detail.as_deref(), Some("defn (:rust)"));
    }

    #[test]
    fn metadata_wrapped_def() {
        let syms = symbols("^:private (def secret 1)");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "secret");
    }

    #[test]
    fn ignores_non_definitions() {
        assert!(symbols("(println \"hi\")").is_empty());
    }
}
