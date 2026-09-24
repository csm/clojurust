//! Property oracle for reader metadata.
//!
//! The example-based tests pin the cases that were reported. These pin the
//! *laws*, over generated annotations and generated forms, so a case nobody
//! thought to report is covered too:
//!
//! 1. **Attachment** — `(meta ^A F)` is `A` when `F` constructs an `IObj`, and
//!    `nil` otherwise. Nothing in between, for any `A` and any `F`.
//! 2. **Preservation** — an annotation never changes the value: `=`, `type`,
//!    `count` and `hash` all answer as they do without it.
//! 3. **Stacking** — `^A ^B F` carries `(merge B A)`: both survive, outer wins.
//! 4. **Duality** — a shorthand expands identically quoted and evaluated; the
//!    two positions differ only in whether the general case is evaluated.
//! 5. **Transparency** — no predicate or accessor can tell an annotated value
//!    from a bare one.
//!
//! Assertions are phrased *in Clojure* (`(= … …)` → `"true"`) rather than by
//! comparing printed forms, so map iteration order cannot make a law flaky.

use cljrs_runtime::env::env::Env;
use proptest::prelude::*;

mod common;
use common::fresh_env as make_env;

/// Evaluate in a caller-supplied environment, rendered as the text a reader
/// would see. Building a `Runtime` per case would dominate the run time of a
/// property test, so `make_env` hands out a namespace over a shared one.
fn eval_in(env: &mut Env, src: &str) -> Result<String, String> {
    common::eval_in(env, src).map(|v| format!("{v}"))
}

fn is_true(env: &mut Env, src: &str) -> bool {
    match eval_in(env, src) {
        Ok(s) => s == "true",
        Err(e) => panic!("{src}\n{e}"),
    }
}

/// An observer applied to a value: its answer, or the error it threw.
///
/// Not every observer is total — `(empty? :kw)` throws — and transparency has
/// to hold for the *failing* cases too: an annotation must not turn a throw
/// into an answer, or one error into another. Comparing outcomes rather than
/// values is what makes a partial observer testable at all.
fn outcome(env: &mut Env, observer: &str, value: &str) -> String {
    match eval_in(env, &format!("({observer} {value})")) {
        Ok(v) => format!("ok:{v}"),
        // The message, not the whole error: two throws from the same cause
        // carry different spans.
        Err(e) => {
            let e = e.replace('\n', " ");
            format!("err:{}", e.split("message:").last().unwrap_or(&e).trim())
        }
    }
}

// ── Generators ───────────────────────────────────────────────────────────────

/// A `^meta` annotation together with the map it denotes, as source.
#[derive(Clone, Debug)]
struct Annotation {
    /// The annotation as written, without the leading `^`.
    src: String,
    /// A Clojure expression for the map it must expand to.
    expected: String,
}

/// Names that mean something to another part of the reader or evaluator.
const RESERVED: &[&str] = &["async", "tag", "private", "dynamic", "macro", "const"];

fn name_strategy() -> impl Strategy<Value = String> {
    "[a-z]{1,5}".prop_filter("reserved annotation key", |s: &String| {
        !RESERVED.contains(&s.as_str())
    })
}

fn scalar_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        (-1000i64..1000).prop_map(|n| n.to_string()),
        name_strategy().prop_map(|s| format!(":{s}")),
        name_strategy().prop_map(|s| format!("\"{s}\"")),
        Just("true".to_string()),
        Just("nil".to_string()),
    ]
}

fn annotation_strategy() -> impl Strategy<Value = Annotation> {
    prop_oneof![
        // ^:kw → {:kw true}
        name_strategy().prop_map(|k| Annotation {
            src: format!(":{k}"),
            expected: format!("{{:{k} true}}"),
        }),
        // ^Sym → {:tag Sym}
        "[A-Z][a-z]{0,4}".prop_map(|t| Annotation {
            src: t.clone(),
            expected: format!("{{:tag '{t}}}"),
        }),
        // ^"Str" → {:tag "Str"}
        name_strategy().prop_map(|t| Annotation {
            src: format!("\"{t}\""),
            expected: format!("{{:tag \"{t}\"}}"),
        }),
        // ^{:k v} → {:k v}
        (name_strategy(), scalar_strategy()).prop_map(|(k, v)| Annotation {
            src: format!("{{:{k} {v}}}"),
            expected: format!("{{:{k} {v}}}"),
        }),
    ]
}

/// Forms that construct an `IObj`, so an evaluated-position annotation on one
/// becomes runtime metadata.
fn constructing_form_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("[]".to_string()),
        Just("()".to_string()),
        Just("[1 2 3]".to_string()),
        Just("{:k 1}".to_string()),
        Just("#{1 2}".to_string()),
        Just("(fn [] 1)".to_string()),
        Just("(fn [x] x)".to_string()),
        Just("#(inc %)".to_string()),
        Just("[[1] {:k 1}]".to_string()),
    ]
}

/// Forms that take the annotation as a compile-time hint and drop it.
fn hint_form_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("(list 1)".to_string()),
        Just("(vector 1)".to_string()),
        Just("(if true [1] [2])".to_string()),
        Just("(do [1])".to_string()),
        Just("(let [q [1]] q)".to_string()),
        Just("'(1 2)".to_string()),
        Just("'sym".to_string()),
        Just("42".to_string()),
        Just("\"s\"".to_string()),
        Just(":kw".to_string()),
    ]
}

/// Predicates and accessors that must not be able to see an annotation.
const OBSERVERS: &[&str] = &[
    "vector?",
    "map?",
    "set?",
    "coll?",
    "seq?",
    "fn?",
    "ifn?",
    "sequential?",
    "associative?",
    "counted?",
    "empty?",
    "some?",
    "nil?",
    "number?",
    "string?",
    "keyword?",
    "symbol?",
    "boolean?",
    "type",
    "count",
];

// ── 1. Attachment ────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn a_constructing_form_carries_exactly_the_annotation(
        ann in annotation_strategy(),
        form in constructing_form_strategy(),
    ) {
        let (_g, mut env) = make_env();
        let Annotation { src, expected } = ann;
        prop_assert!(
            is_true(&mut env, &format!("(= (meta ^{src} {form}) {expected})")),
            "^{src} {form} did not carry {expected}"
        );
    }

    #[test]
    fn a_hint_form_carries_nothing(
        ann in annotation_strategy(),
        form in hint_form_strategy(),
    ) {
        let (_g, mut env) = make_env();
        let src = ann.src;
        prop_assert!(
            is_true(&mut env, &format!("(nil? (meta ^{src} {form}))")),
            "^{src} {form} carried metadata; a hint position must carry none"
        );
    }
}

// ── 2. Preservation ──────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn an_annotation_never_changes_the_value(
        ann in annotation_strategy(),
        form in constructing_form_strategy(),
    ) {
        let (_g, mut env) = make_env();
        let src = ann.src;
        // `fn` values are only equal to themselves, so compare what is
        // observable for every shape: type, and equality for data.
        prop_assert!(
            is_true(&mut env, &format!("(= (type ^{src} {form}) (type {form}))")),
            "^{src} {form} changed its type"
        );
        if !form.starts_with("(fn") && !form.starts_with('#') || form.starts_with("#{") {
            prop_assert!(
                is_true(&mut env, &format!("(= ^{src} {form} {form})")),
                "^{src} {form} is no longer equal to {form}"
            );
            prop_assert!(
                is_true(&mut env, &format!("(= (hash ^{src} {form}) (hash {form}))")),
                "^{src} {form} hashes differently from {form}"
            );
        }
    }
}

// ── 3. Stacking ──────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn stacked_annotations_merge_outer_last(
        outer in annotation_strategy(),
        inner in annotation_strategy(),
        form in constructing_form_strategy(),
    ) {
        let (_g, mut env) = make_env();
        let expr = format!("(meta ^{} ^{} {form})", outer.src, inner.src);
        let law = format!("(merge {} {})", inner.expected, outer.expected);
        prop_assert!(
            is_true(&mut env, &format!("(= {expr} {law})")),
            "^{} ^{} {form} is not (merge inner outer)", outer.src, inner.src
        );
    }
}

// ── 4. Duality ───────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn a_shorthand_expands_the_same_quoted_and_evaluated(
        ann in annotation_strategy(),
    ) {
        let (_g, mut env) = make_env();
        let src = ann.src;
        // A map annotation may contain an expression, which is data quoted and
        // a value evaluated; every *shorthand* must agree in both positions.
        prop_assume!(!src.starts_with('{'));
        prop_assert!(
            is_true(&mut env, &format!("(= (meta '^{src} [1]) (meta ^{src} [1]))")),
            "^{src} expanded differently quoted vs evaluated"
        );
    }
}

// ── 5. Transparency ──────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn no_observer_can_see_an_annotation(
        ann in annotation_strategy(),
        form in constructing_form_strategy(),
        observer in proptest::sample::select(OBSERVERS),
    ) {
        let (_g, mut env) = make_env();
        let src = ann.src;
        let annotated = outcome(&mut env, observer, &format!("^{src} {form}"));
        let bare = outcome(&mut env, observer, &form);
        prop_assert_eq!(
            &annotated, &bare,
            "`{}` told ^{} {} apart from {}", observer, src, form, form
        );
    }

    /// The same, for a value carrying metadata put there by `with-meta` rather
    /// than by the reader — including the scalars and symbols the reader now
    /// refuses to annotate.
    #[test]
    fn no_observer_can_see_programmatic_metadata(
        ann in annotation_strategy(),
        observer in proptest::sample::select(OBSERVERS),
        form in prop_oneof![
            Just("[1 2]".to_string()),
            Just("{:k 1}".to_string()),
            Just("#{1}".to_string()),
            Just("'(1 2)".to_string()),
            Just("'sym".to_string()),
            Just(":kw".to_string()),
            Just("\"s\"".to_string()),
            Just("1".to_string()),
            Just("nil".to_string()),
        ],
    ) {
        let (_g, mut env) = make_env();
        let expected = ann.expected;
        let annotated = outcome(&mut env, observer, &format!("(with-meta {form} {expected})"));
        let bare = outcome(&mut env, observer, &form);
        prop_assert_eq!(
            &annotated, &bare,
            "`{}` told (with-meta {} …) apart from {}", observer, form, form
        );
    }
}
