//! `clojure.pprint`: the data pretty-printer.
//!
//! The namespace did not exist, so `(:require [clojure.pprint ...])` failed
//! outright and any library that pretty-prints anything broke at load rather
//! than degrading.
//!
//! What is pinned here is the property that makes `pprint` worth calling at
//! all: a value that FITS inside `*print-right-margin*` stays on one line, and
//! one that does not breaks with its children aligned under the opening
//! delimiter. A pretty-printer that always breaks, or never breaks, passes a
//! "does it run" test and is useless.
//!
//! One runtime is shared by every assertion: building one re-evaluates
//! `bootstrap.cljrs`, and doing that per assertion is what made other suites
//! expensive (CLJRS-TEST-RUNTIME-REUSE).

use cljrs_runtime::{ExecutionMode, Runtime};

/// A runtime with the stdlib installed and `clojure.pprint` required.
fn env_with_pprint() -> cljrs_runtime::tiered::Env {
    let runtime = Runtime::builder()
        .execution_mode(ExecutionMode::TreeWalk)
        .build()
        .expect("bootstrap clojure.core");
    cljrs_stdlib::install(&runtime);
    let mut env = runtime.env("user");
    eval(&mut env, "(require '[clojure.pprint :as pp])");
    env
}

/// Evaluate `src` and return the result as text.
///
/// A string result is unwrapped to its raw contents rather than rendered:
/// `Value::to_string` produces the *readable* form, so a pretty-printed string
/// would come back quoted with its newlines escaped, and every assertion here
/// is about the layout those newlines make.
fn eval(env: &mut cljrs_runtime::tiered::Env, src: &str) -> String {
    let mut parser = cljrs_reader::Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser
        .parse_all()
        .unwrap_or_else(|e| panic!("parse {src}: {e:?}"));
    let mut out = cljrs_value::Value::Nil;
    for form in forms {
        out = cljrs_runtime::interp::eval::eval(&form, env)
            .unwrap_or_else(|e| panic!("eval {src}: {e:?}"));
    }
    match out {
        cljrs_value::Value::Str(s) => s.get().to_string(),
        other => other.to_string(),
    }
}

/// `(pp/pprint-str x)` — the text `pprint` would print, quoted out of the way.
fn pretty(env: &mut cljrs_runtime::tiered::Env, src: &str) -> String {
    eval(env, &format!("(pp/pprint-str {src})"))
}

#[test]
fn a_value_that_fits_stays_on_one_line() {
    let mut env = env_with_pprint();
    assert_eq!(pretty(&mut env, "{:a 1 :b 2}"), "{:a 1, :b 2}");
    assert_eq!(pretty(&mut env, "[1 2 3]"), "[1 2 3]");

    // Scalars and empty collections are returned untouched rather than
    // wrapped or broken.
    assert_eq!(pretty(&mut env, "nil"), "nil");
    assert_eq!(pretty(&mut env, "42"), "42");
    assert_eq!(pretty(&mut env, "[]"), "[]");
    assert_eq!(pretty(&mut env, "{}"), "{}");
}

#[test]
fn a_value_wider_than_the_margin_breaks_aligned() {
    let mut env = env_with_pprint();
    let out = pretty(
        &mut env,
        r#"{:name "widget" :parts [{:id 1 :label "aaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
                                   {:id 2 :label "bbbbbbbbbbbbbbbbbbbbbbbbbbbb"}]}"#,
    );
    assert_eq!(
        out,
        "{:name \"widget\"\n \
         :parts [{:id 1, :label \"aaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}\n         \
         {:id 2, :label \"bbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}]}",
        "children must align under the opening delimiter:\n{out}"
    );
}

/// The margin is what decides, so moving it must move the output. Without
/// this, a printer that breaks on some fixed nesting depth would pass.
#[test]
fn the_right_margin_decides_where_breaks_happen() {
    let mut env = env_with_pprint();
    let src = "{:a [1 2 3] :b [4 5 6]}";

    // Wide enough: one line.
    assert_eq!(
        eval(
            &mut env,
            &format!("(binding [pp/*print-right-margin* 200] (pp/pprint-str {src}))")
        ),
        "{:a [1 2 3], :b [4 5 6]}"
    );

    // Narrow: the same value breaks.
    assert_eq!(
        eval(
            &mut env,
            &format!("(binding [pp/*print-right-margin* 20] (pp/pprint-str {src}))")
        ),
        "{:a [1 2 3]\n :b [4 5 6]}"
    );
}

/// A record must break like a map while keeping its printed tag, so the
/// output still reads back the way `pr-str` wrote it.
///
/// This is the case that caught a real runtime divergence: `(coll? a-record)`
/// is false here and true on the JVM, so the layout guard treated records as
/// scalars and a 118-column record printed on one line under a 72-column
/// margin. See CLJRS-RECORD-NOT-COLL.
#[test]
fn a_record_breaks_and_keeps_its_tag() {
    let mut env = env_with_pprint();
    eval(&mut env, "(defrecord Person [name email address])");
    let out = pretty(
        &mut env,
        r#"(->Person "Ada Lovelace" "ada@example.org"
                     {:street "aaaaaaaaaaaaaaaaaaaaa" :city "bbbbbbbbbbbb"})"#,
    );
    assert!(
        out.starts_with("#Person{:name \"Ada Lovelace\"\n"),
        "record must break under its own tag:\n{out}"
    );
    assert!(
        out.contains("\n        :email "),
        "fields must align past the tag:\n{out}"
    );
}

#[test]
fn print_table_lays_out_columns() {
    let mut env = env_with_pprint();
    let out = eval(
        &mut env,
        r#"(with-out-str (pp/print-table [{:a 1 :b "hello"} {:a 22 :b "hi"}]))"#,
    );
    assert!(out.contains("| :a |    :b |"), "header:\n{out}");
    assert!(out.contains("|----+-------|"), "separator:\n{out}");
    assert!(out.contains("|  1 | hello |"), "row 1:\n{out}");
    assert!(out.contains("| 22 |    hi |"), "row 2:\n{out}");
}

/// An empty table prints nothing rather than throwing on `(first nil)`.
#[test]
fn print_table_tolerates_no_rows() {
    let mut env = env_with_pprint();
    assert_eq!(eval(&mut env, "(with-out-str (pp/print-table []))"), "");
}
