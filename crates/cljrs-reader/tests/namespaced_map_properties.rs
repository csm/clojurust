//! Property tests for namespaced-map key qualification.
//!
//! `qualify_keys` is a pure source-level rewrite, so the interesting statements
//! are laws over arbitrary key/value vectors rather than individual spellings.
//! The load-bearing one is idempotence: qualification must be a projection, or
//! a map that survives two reader passes changes meaning between them.

use cljrs_reader::form::{Form, FormKind};
use cljrs_reader::namespaced_map::{MapNs, qualify_keys};
use cljrs_types::span::Span;
use proptest::prelude::*;
use std::sync::Arc;

fn span() -> Span {
    Span::new(Arc::new("<proptest>".to_string()), 0, 0, 1, 1)
}

fn form(kind: FormKind) -> Form {
    Form::new(kind, span())
}

/// A bare identifier: no `/`, so always a qualification candidate.
fn bare_name() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9*+!?-]{0,7}"
}

/// A namespace: bare identifiers joined by dots, as `my.app.core`.
/// `nil`, `true` and `false` read as literals, not symbols, so they are never
/// a valid prefix; the generator must not produce them.
fn ns_name() -> impl Strategy<Value = String> {
    prop::collection::vec(bare_name(), 1..3)
        .prop_map(|parts| parts.join("."))
        .prop_filter("reserved literal, not a symbol", |ns| {
            !matches!(ns.as_str(), "nil" | "true" | "false")
        })
}

/// Any key form the reader can hand to qualification: the two qualifiable
/// kinds, the `/` and `_/x` special cases, and a non-identifier.
fn key_form() -> impl Strategy<Value = Form> {
    prop_oneof![
        bare_name().prop_map(|n| form(FormKind::Keyword(n))),
        bare_name().prop_map(|n| form(FormKind::Symbol(n))),
        (ns_name(), bare_name()).prop_map(|(ns, n)| form(FormKind::Keyword(format!("{ns}/{n}")))),
        bare_name().prop_map(|n| form(FormKind::Keyword(format!("_/{n}")))),
        Just(form(FormKind::Symbol("/".to_string()))),
        any::<i64>().prop_map(|i| form(FormKind::Int(i))),
    ]
}

/// A flat key/value body of even length, as the parser produces.
fn body() -> impl Strategy<Value = Vec<Form>> {
    prop::collection::vec((key_form(), any::<i64>()), 0..8).prop_map(|pairs| {
        pairs
            .into_iter()
            .flat_map(|(k, v)| [k, form(FormKind::Int(v))])
            .collect()
    })
}

/// A body whose keys are keywords only - the case that can never error, so
/// properties about the Ok value can be stated unconditionally.
///
/// The `:_/k` opt-out is excluded: it is a one-shot escape, so it is the one
/// key kind qualification is deliberately not idempotent over (see
/// `underscore_optout_is_one_shot_by_design`).
fn keyword_body() -> impl Strategy<Value = Vec<Form>> {
    prop::collection::vec((key_form(), any::<i64>()), 0..8).prop_map(|pairs| {
        pairs
            .into_iter()
            .filter(|(k, _)| match &k.kind {
                FormKind::Symbol(_) => false,
                FormKind::Keyword(n) => !n.starts_with("_/"),
                _ => true,
            })
            .flat_map(|(k, v)| [k, form(FormKind::Int(v))])
            .collect()
    })
}

fn kinds(forms: Vec<Form>) -> Vec<FormKind> {
    forms.into_iter().map(|f| f.kind).collect()
}

proptest! {
    /// Qualification rewrites keys; it never adds, drops, or reorders entries.
    #[test]
    fn length_is_preserved(ns in ns_name(), b in body()) {
        let n = b.len();
        {
            let out = qualify_keys(&MapNs::Literal(ns), b);
            prop_assert_eq!(out.len(), n);
        }
    }

    /// Only even indices are keys. A value that looks exactly like a bare key
    /// must come out untouched, or `{:a :b}` silently becomes `{:ns/a :ns/b}`.
    #[test]
    fn odd_indices_are_never_rewritten(ns in ns_name(), b in body()) {
        let before = kinds(b.clone());
        {
            let out = qualify_keys(&MapNs::Literal(ns), b);
            let after = kinds(out);
            for i in (1..before.len()).step_by(2) {
                prop_assert_eq!(&before[i], &after[i]);
            }
        }
    }

    /// THE load-bearing law: qualification is a projection. Re-reading an
    /// already-qualified body under the same namespace is a no-op, because
    /// every key it produced now carries an explicit namespace.
    #[test]
    fn qualification_is_idempotent(ns in ns_name(), b in keyword_body()) {
        let once = qualify_keys(&MapNs::Literal(ns.clone()), b);
        let twice = qualify_keys(&MapNs::Literal(ns), once.clone());
        prop_assert_eq!(kinds(once), kinds(twice));
    }

    /// The exception to the law above, pinned so it stays a decision rather
    /// than a surprise: `:_/k` yields a bare `:k`, which a second pass would
    /// qualify. That is correct - the JVM reader qualifies a map literal once,
    /// at read time, and never re-qualifies the map it produced. The property
    /// exists so anyone who later makes qualification re-entrant sees this
    /// case fail rather than silently turning `:_/a` into `:ns/a`.
    #[test]
    fn underscore_optout_is_one_shot_by_design(ns in ns_name(), name in bare_name()) {
        let once = qualify_keys(
            &MapNs::Literal(ns.clone()),
            vec![form(FormKind::Keyword(format!("_/{name}"))), form(FormKind::Int(0))],
        );
        prop_assert_eq!(&once[0].kind, &FormKind::Keyword(name.clone()));

        let twice = qualify_keys(&MapNs::Literal(ns.clone()), once);
        prop_assert_eq!(&twice[0].kind, &FormKind::Keyword(format!("{ns}/{name}")));
    }

    /// Under a literal namespace every bare keyword key gains exactly that
    /// namespace - the feature's whole point, stated over arbitrary names.
    #[test]
    fn bare_keyword_keys_gain_the_literal_namespace(ns in ns_name(), name in bare_name()) {
        let out = qualify_keys(
            &MapNs::Literal(ns.clone()),
            vec![form(FormKind::Keyword(name.clone())), form(FormKind::Int(0))],
        );
        prop_assert_eq!(&out[0].kind, &FormKind::Keyword(format!("{ns}/{name}")));
    }

    /// `:_/k` opts out, for every namespace and every name. The result carries
    /// no namespace at all, not the map's and not a literal `_`.
    #[test]
    fn underscore_always_unqualifies(ns in ns_name(), name in bare_name()) {
        let out = qualify_keys(
            &MapNs::Literal(ns),
            vec![form(FormKind::Keyword(format!("_/{name}"))), form(FormKind::Int(0))],
        );
        prop_assert_eq!(&out[0].kind, &FormKind::Keyword(name));
    }

    /// A key that already names a namespace keeps it - the map's namespace
    /// never overrides an explicit one.
    #[test]
    fn explicit_namespace_always_wins(
        map_ns in ns_name(),
        key_ns in ns_name(),
        name in bare_name(),
    ) {
        prop_assume!(key_ns != "_");
        let spelled = format!("{key_ns}/{name}");
        let out = qualify_keys(
            &MapNs::Literal(map_ns),
            vec![form(FormKind::Keyword(spelled.clone())), form(FormKind::Int(0))],
        );
        prop_assert_eq!(&out[0].kind, &FormKind::Keyword(spelled));
    }

    /// A literal namespace resolves at read time, so it must never emit the
    /// deferred `AutoKeyword` the evaluator would resolve against `*ns*`.
    #[test]
    fn literal_namespace_never_defers_resolution(ns in ns_name(), b in body()) {
        {
            let out = qualify_keys(&MapNs::Literal(ns), b);
            for f in out {
                prop_assert!(!matches!(f.kind, FormKind::AutoKeyword(_)));
            }
        }
    }

    /// Auto-resolved spellings defer every bare keyword key to the evaluator,
    /// and only those: an already-qualified key stays a plain Keyword.
    #[test]
    fn auto_defers_exactly_the_bare_keyword_keys(alias in ns_name(), name in bare_name()) {
        let out = qualify_keys(
            &MapNs::Alias(alias.clone()),
            vec![form(FormKind::Keyword(name.clone())), form(FormKind::Int(0))],
        );
        prop_assert_eq!(&out[0].kind, &FormKind::AutoKeyword(format!("{alias}/{name}")));
    }

    /// A symbol key under an auto-resolved map defers exactly like a keyword
    /// key does: to `AutoSymbol`, carrying the alias when there is one, for the
    /// evaluator to resolve against the reading namespace.
    #[test]
    fn auto_defers_symbol_keys_to_auto_symbol(alias in ns_name(), name in bare_name()) {
        let out = qualify_keys(
            &MapNs::CurrentNs,
            vec![form(FormKind::Symbol(name.clone())), form(FormKind::Int(0))],
        );
        prop_assert_eq!(&out[0].kind, &FormKind::AutoSymbol(name.clone()));

        let out = qualify_keys(
            &MapNs::Alias(alias.clone()),
            vec![form(FormKind::Symbol(name.clone())), form(FormKind::Int(0))],
        );
        prop_assert_eq!(&out[0].kind, &FormKind::AutoSymbol(format!("{alias}/{name}")));
    }

    /// `/` is a NAME, not a namespace separator: `#:foo{/ 1}` has the key
    /// `foo//`, as the JVM reader produces. Leaving it bare would make it the
    /// one key the map's namespace silently skips.
    #[test]
    fn slash_takes_the_map_namespace(ns in ns_name(), name in bare_name()) {
        let out = qualify_keys(
            &MapNs::Literal(ns.clone()),
            vec![form(FormKind::Symbol("/".to_string())), form(FormKind::Int(0))],
        );
        prop_assert_eq!(&out[0].kind, &FormKind::Symbol(format!("{ns}//")));

        let out = qualify_keys(
            &MapNs::Literal(ns.clone()),
            vec![form(FormKind::Keyword("/".to_string())), form(FormKind::Int(0))],
        );
        prop_assert_eq!(&out[0].kind, &FormKind::Keyword(format!("{ns}//")));

        // ... and it is still just a name under an alias.
        let out = qualify_keys(
            &MapNs::Alias(name.clone()),
            vec![form(FormKind::Keyword("/".to_string())), form(FormKind::Int(0))],
        );
        prop_assert_eq!(&out[0].kind, &FormKind::AutoKeyword(format!("{name}//")));
    }

    /// Any bare identifier is a legal prefix; anything carrying a namespace of
    /// its own, or not starting like a symbol, is refused - the prefix must
    /// read as an unqualified symbol.
    #[test]
    fn prefix_parses_exactly_the_unqualified_symbols(
        ns in ns_name(),
        name in bare_name(),
        auto in any::<bool>(),
    ) {
        let qualified = format!("{ns}/{name}");
        let leading_digit = format!("1{ns}");
        prop_assert!(MapNs::parse(&ns, auto).is_ok(), "{} rejected", ns);
        prop_assert!(MapNs::parse(&qualified, auto).is_err(), "{} accepted", qualified);
        prop_assert!(MapNs::parse(&leading_digit, auto).is_err(), "{} accepted", leading_digit);
    }

}
