//! Regression tests pinning PR #354: a protocol named in an **impl position**
//! may be qualified, and must resolve through its own namespace.
//!
//! `defrecord`/`deftype`/`reify`/`extend-type`/`extend-protocol` used to look
//! the protocol up with `lookup_in_ns(current_ns, "mp/IThing")` — passing the
//! whole symbol string. A qualified name is neither interned nor referred under
//! that string in the current ns, so a cross-namespace impl failed with
//! "mp/IThing is not a protocol" even though the protocol was loaded and
//! `(resolve 'mini.proto/IThing)` was truthy. That sinks every design where the
//! protocol and its implementations live in different namespaces — which is
//! most of them.
//!
//! `resolve_protocol_sym` fixed it, but its three call sites were lost across a
//! `main` merge, leaving the helper dead and the bug back. Nothing caught that,
//! because nothing tested it. These tests are that test.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

/// Evaluate `src` and return the last value rendered with `pr-str`.
fn eval_pr(src: &str) -> String {
    let (_globals, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env).expect("eval error");
    }
    match result {
        Value::Str(s) => s.get().as_str().to_string(),
        // The type name, not `{:?}`: a `Value` may be a `Uuid`, and CodeQL
        // reads Debug-formatting one into a panic as logging it in cleartext.
        // Which type came back instead of a string is what this assertion is
        // actually about.
        other => panic!("expected a string from pr-str, got a {}", other.type_name()),
    }
}

/// A protocol defined in `mini.proto`, with the caller back in `user` and
/// `mp` aliased to it — the shape every port/adapter design takes.
const PRELUDE: &str = "(ns mini.proto)
     (defprotocol IThing (-describe [this]))
     (ns user)
     (alias 'mp 'mini.proto)
     ";

// ── The three impl sites ─────────────────────────────────────────────────────

#[test]
fn defrecord_implements_a_protocol_from_another_namespace() {
    assert_eq!(
        eval_pr(&format!(
            "{PRELUDE}
             (defrecord R [n] mp/IThing (-describe [_] [:record n]))
             (pr-str (mp/-describe (->R 1)))"
        )),
        "[:record 1]"
    );
}

#[test]
fn deftype_implements_a_protocol_from_another_namespace() {
    assert_eq!(
        eval_pr(&format!(
            "{PRELUDE}
             (deftype T [n] mp/IThing (-describe [_] [:type n]))
             (pr-str (mp/-describe (->T 2)))"
        )),
        "[:type 2]"
    );
}

#[test]
fn reify_implements_a_protocol_from_another_namespace() {
    assert_eq!(
        eval_pr(&format!(
            "{PRELUDE}
             (pr-str (mp/-describe (reify mp/IThing (-describe [_] :reified))))"
        )),
        ":reified"
    );
}

#[test]
fn extend_type_names_a_protocol_from_another_namespace() {
    assert_eq!(
        eval_pr(&format!(
            "{PRELUDE}
             (extend-type String mp/IThing (-describe [s] [:string s]))
             (pr-str (mp/-describe \"hi\"))"
        )),
        "[:string \"hi\"]"
    );
}

#[test]
fn extend_protocol_names_a_protocol_from_another_namespace() {
    assert_eq!(
        eval_pr(&format!(
            "{PRELUDE}
             (extend-protocol mp/IThing Long (-describe [n] [:long n]))
             (pr-str (mp/-describe 7))"
        )),
        "[:long 7]"
    );
}

// ── Fully qualified, no alias ────────────────────────────────────────────────

#[test]
fn a_fully_qualified_protocol_name_resolves_without_an_alias() {
    assert_eq!(
        eval_pr(
            "(ns mini.proto)
             (defprotocol IThing (-describe [this]))
             (ns user)
             (defrecord R [] mini.proto/IThing (-describe [_] :qualified))
             (pr-str (mini.proto/-describe (->R)))"
        ),
        ":qualified"
    );
}

// ── The unqualified case still resolves in the current ns ────────────────────

#[test]
fn an_unqualified_protocol_name_still_resolves_in_the_current_ns() {
    assert_eq!(
        eval_pr(
            "(defprotocol P (-describe [this]))
             (defrecord R [] P (-describe [_] :same-ns))
             (pr-str (-describe (->R)))"
        ),
        ":same-ns"
    );
}
