//! Property tests for the namespaced-map PREFIX, at the lexer boundary.
//!
//! The sibling file states laws over `qualify_keys`; these state laws over what
//! the lexer accepts as a prefix, which is the half `MapNs::parse` guards.

use cljrs_reader::MapNs;
use cljrs_reader::lexer::Lexer;
use cljrs_reader::token::Token;
use proptest::prelude::*;

fn lex_first(src: &str) -> Result<Token, String> {
    let mut lexer = Lexer::new(src.to_string(), "<proptest>".to_string());
    lexer
        .next_token()
        .map(|(t, _)| t)
        .map_err(|e| format!("{e:?}"))
}

/// A bare identifier: the shape a prefix is allowed to have.
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

/// Any run of the characters the JVM reader skips between the prefix and `{`.
fn separator() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(vec![" ", "\t", "\n", "\r", ","]), 0..6)
        .prop_map(|parts| parts.concat())
}

proptest! {
    /// Whitespace and commas between the prefix and `{` are not part of the
    /// literal: they cannot change which prefix was read.
    #[test]
    fn separators_before_the_brace_never_change_the_prefix(ns in ns_name(), sep in separator()) {
        let tight = lex_first(&format!("#:{ns}{{:a 1}}"));
        let spaced = lex_first(&format!("#:{ns}{sep}{{:a 1}}"));
        prop_assert_eq!(tight, spaced);
    }

    /// The same, for both auto-resolved spellings.
    #[test]
    fn separators_before_the_brace_never_change_an_auto_prefix(
        ns in prop::option::of(ns_name()),
        sep in separator(),
    ) {
        let alias = ns.unwrap_or_default();
        let tight = lex_first(&format!("#::{alias}{{:a 1}}"));
        let spaced = lex_first(&format!("#::{alias}{sep}{{:a 1}}"));
        prop_assert_eq!(tight, spaced);
    }

    /// A comment is not whitespace: it ends the literal, whatever follows.
    #[test]
    fn a_comment_after_the_prefix_is_never_a_map(ns in ns_name()) {
        let err = lex_first(&format!("#:{ns} ;; a comment\n{{:a 1}}")).unwrap_err();
        prop_assert!(err.contains("must be followed by a map"), "{}", err);
    }

    /// Every accepted prefix carries the namespace it was written with, and the
    /// `#::` spellings are exactly the ones that defer resolution.
    #[test]
    fn an_accepted_prefix_round_trips_its_namespace(ns in ns_name()) {
        prop_assert_eq!(
            lex_first(&format!("#:{ns}{{:a 1}}")).unwrap(),
            Token::NamespacedMap(MapNs::Literal(ns.clone()))
        );
        prop_assert_eq!(
            lex_first(&format!("#::{ns}{{:a 1}}")).unwrap(),
            Token::NamespacedMap(MapNs::Alias(ns))
        );
    }

    /// A prefix carrying its own namespace is a read error, never a key that
    /// silently gains two namespaces.
    #[test]
    fn a_qualified_prefix_is_always_refused(outer in ns_name(), inner in bare_name()) {
        for src in [
            format!("#:{outer}/{inner}{{:a 1}}"),
            format!("#::{outer}/{inner}{{:a 1}}"),
        ] {
            let err = lex_first(&src).unwrap_err();
            prop_assert!(err.contains("unqualified symbol"), "{}: {}", src, err);
        }
    }

    /// A prefix that starts with a digit cannot be a symbol, so it cannot be a
    /// prefix either.
    #[test]
    fn a_numeric_prefix_is_always_refused(n in 0u32..100_000, rest in "[a-z]{0,4}") {
        let err = lex_first(&format!("#:{n}{rest}{{:a 1}}")).unwrap_err();
        prop_assert!(err.contains("unqualified symbol"), "{}", err);
    }
}
