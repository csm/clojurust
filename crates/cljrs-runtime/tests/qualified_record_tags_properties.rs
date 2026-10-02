//! Property tests for qualified record tags: a record named without its
//! namespace reaches its qualified tag through `(:import ...)`, whatever the
//! namespace, the type name and the spelling of the import.
//!
//! Every case runs over one shared runtime (see `common`), so each takes
//! namespaces of its own: a namespace name means the same thing to every case
//! on the thread.

use std::cell::Cell;

use proptest::prelude::*;

mod common;
use common::{eval_in, fresh_env};

thread_local! {
    static NEXT_CASE: Cell<u64> = const { Cell::new(0) };
}

/// A namespace prefix no other case on this thread has used.
fn case_prefix() -> String {
    NEXT_CASE.with(|c| {
        let n = c.get();
        c.set(n + 1);
        format!("qrt{n}")
    })
}

/// The tail of a namespace name: dotted segments, some hyphenated.
fn ns_tail() -> impl Strategy<Value = String> {
    "[a-z]{1,4}(-[a-z]{1,3})?(\\.[a-z]{1,4}(-[a-z]{1,3})?){0,2}"
}

/// A type name that cannot collide with a `clojure.core` var.
fn type_name() -> impl Strategy<Value = String> {
    "[A-Z][a-z]{1,5}".prop_map(|s| format!("T{s}"))
}

/// The ways an `ns` form can import `name` from `ns`.
#[derive(Clone, Copy, Debug)]
enum Import {
    Vector,
    List,
    Dotted,
    /// The JVM's package spelling: `-` written as `_`.
    MungedVector,
    MungedDotted,
}

impl Import {
    fn clause(self, ns: &str, name: &str) -> String {
        let munged = ns.replace('-', "_");
        match self {
            Import::Vector => format!("(:import [{ns} {name}])"),
            Import::List => format!("(:import ({ns} {name}))"),
            Import::Dotted => format!("(:import {ns}.{name})"),
            Import::MungedVector => format!("(:import [{munged} {name}])"),
            Import::MungedDotted => format!("(:import {munged}.{name})"),
        }
    }
}

fn import() -> impl Strategy<Value = Import> {
    prop_oneof![
        Just(Import::Vector),
        Just(Import::List),
        Just(Import::Dotted),
        Just(Import::MungedVector),
        Just(Import::MungedDotted),
    ]
}

/// Define a protocol, a record `name` in `{prefix}.{tail}`, and a consumer
/// namespace whose `ns` form carries `clause` and which extends the protocol
/// to the record under its unqualified name. Evaluates to the method called
/// on an instance.
fn extend_from_consumer(prefix: &str, tail: &str, name: &str, clause: &str) -> String {
    format!(
        "(ns {prefix}.proto)
         (defprotocol Named (nm [x]))
         (ns {prefix}.{tail})
         (defrecord {name} [x])
         (ns {prefix}.consumer {clause})
         (extend-type {name} {prefix}.proto/Named (nm [_] :extended))
         ({prefix}.proto/nm ({prefix}.{tail}/->{name} 1))"
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// An imported record is extended under its qualified tag, for every
    /// spelling of the import.
    #[test]
    fn an_imported_record_dispatches(tail in ns_tail(), name in type_name(), import in import()) {
        let prefix = case_prefix();
        let clause = import.clause(&format!("{prefix}.{tail}"), &name);
        let (_g, mut env) = fresh_env();
        let result = eval_in(&mut env, &extend_from_consumer(&prefix, &tail, &name, &clause));
        prop_assert_eq!(result.map(|v| format!("{v}")), Ok(":extended".to_string()), "{}", clause);
    }

    /// Without the import the record's name does not resolve, the extension
    /// lands under the bare tag, and the failing call says so.
    #[test]
    fn an_unimported_record_fails_with_the_unqualified_tag_named(
        tail in ns_tail(),
        name in type_name(),
    ) {
        let prefix = case_prefix();
        let (_g, mut env) = fresh_env();
        let err = eval_in(&mut env, &extend_from_consumer(&prefix, &tail, &name, ""))
            .expect_err("no implementation under the qualified tag");
        prop_assert!(
            err.contains(&format!("for type {prefix}.{tail}.{name}")),
            "{}", err
        );
        prop_assert!(
            err.contains(&format!("registered under the unqualified tag {name}:")),
            "{}", err
        );
    }

    /// Two namespaces define a record of the same name; two consumers each
    /// import one and extend it. Neither extension reaches the other record.
    #[test]
    fn imports_of_same_named_records_stay_apart(
        a in ns_tail(),
        b in ns_tail(),
        name in type_name(),
        import_a in import(),
        import_b in import(),
    ) {
        let prefix = case_prefix();
        let (ns_a, ns_b) = (format!("{prefix}.a.{a}"), format!("{prefix}.b.{b}"));
        let src = format!(
            "(ns {prefix}.proto)
             (defprotocol Named (nm [x]))
             (ns {ns_a})
             (defrecord {name} [x])
             (ns {ns_b})
             (defrecord {name} [x])
             (ns {prefix}.consumer-a {clause_a})
             (extend-type {name} {prefix}.proto/Named (nm [_] :a))
             (ns {prefix}.consumer-b {clause_b})
             (extend-type {name} {prefix}.proto/Named (nm [_] :b))
             [({prefix}.proto/nm ({ns_a}/->{name} 1)) ({prefix}.proto/nm ({ns_b}/->{name} 1))]",
            clause_a = import_a.clause(&ns_a, &name),
            clause_b = import_b.clause(&ns_b, &name),
        );
        let (_g, mut env) = fresh_env();
        let result = eval_in(&mut env, &src);
        prop_assert_eq!(result.map(|v| format!("{v}")), Ok("[:a :b]".to_string()));
    }

    /// Importing a name that is not a loaded record or deftype changes
    /// nothing: the `ns` form evaluates and the name stays unbound.
    #[test]
    fn importing_an_unknown_name_binds_nothing(tail in ns_tail(), name in type_name(), import in import()) {
        let prefix = case_prefix();
        let clause = import.clause(&format!("{prefix}.{tail}"), &name);
        let (_g, mut env) = fresh_env();
        let result = eval_in(
            &mut env,
            &format!("(ns {prefix}.consumer {clause}) (nil? (resolve '{name}))"),
        );
        prop_assert_eq!(result.map(|v| format!("{v}")), Ok("true".to_string()), "{}", clause);
    }

    /// A namespace that already binds the name, by its own definition or by
    /// an earlier import, cannot import another type under it: the error
    /// names the var the name is bound to.
    #[test]
    fn importing_over_another_binding_is_an_error(
        a in ns_tail(),
        b in ns_tail(),
        name in type_name(),
        import_a in import(),
        import_b in import(),
        local in any::<bool>(),
    ) {
        let prefix = case_prefix();
        let (ns_a, ns_b) = (format!("{prefix}.a.{a}"), format!("{prefix}.b.{b}"));
        let consumer = format!("{prefix}.consumer");
        let (bind, bound_ns) = if local {
            (format!("(ns {consumer}) (defrecord {name} [x])"), consumer.clone())
        } else {
            (format!("(ns {consumer} {})", import_a.clause(&ns_a, &name)), ns_a.clone())
        };
        let src = format!(
            "(ns {ns_a})
             (defrecord {name} [x])
             (ns {ns_b})
             (defrecord {name} [x])
             {bind}
             (ns {consumer} {clause_b})",
            clause_b = import_b.clause(&ns_b, &name),
        );
        let (_g, mut env) = fresh_env();
        let err = eval_in(&mut env, &src).expect_err("the name is already bound");
        prop_assert!(
            err.contains(&format!(
                "{name} already refers to: #'{bound_ns}/{name} in namespace: {consumer}"
            )),
            "{}", err
        );
    }

    /// Importing the same type again, in any spelling, is not a conflict.
    #[test]
    fn importing_the_same_type_again_is_accepted(
        tail in ns_tail(),
        name in type_name(),
        first in import(),
        again in import(),
    ) {
        let prefix = case_prefix();
        let ns = format!("{prefix}.{tail}");
        let src = format!(
            "(ns {ns})
             (defrecord {name} [x])
             (ns {prefix}.consumer {first})
             (ns {prefix}.consumer {again})
             (ns {ns} {again})
             (some? (resolve '{name}))",
            first = first.clause(&ns, &name),
            again = again.clause(&ns, &name),
        );
        let (_g, mut env) = fresh_env();
        let result = eval_in(&mut env, &src);
        prop_assert_eq!(result.map(|v| format!("{v}")), Ok("true".to_string()));
    }
}
