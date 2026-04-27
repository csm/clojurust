//! Runs the Clojure-side `clojure.test` suites for the namespaces in
//! `cljrs.compiler.*` against the embedded compiler sources.
//!
//! Each test file at `test/cljrs/compiler/<name>_test.cljrs` is required
//! through the cljrs evaluator, then `(clojure.test/run-tests …)` is invoked
//! and its `:fail` / `:error` counters are checked.  A non-zero count fails
//! the Rust test.

use std::path::PathBuf;

use cljrs_eval::{Env, eval};
use cljrs_value::Value;

const TEST_NSES: &[&str] = &[
    "cljrs.compiler.ir-test",
    "cljrs.compiler.known-test",
    "cljrs.compiler.escape-test",
    "cljrs.compiler.optimize-test",
];

fn parse_one(src: &str) -> cljrs_reader::Form {
    let mut parser = cljrs_reader::Parser::new(src.to_string(), "<test-driver>".to_string());
    let forms = parser.parse_all().expect("test driver: parse failed");
    forms
        .into_iter()
        .next()
        .expect("test driver: expected at least one form")
}

fn eval_str(env: &mut Env, src: &str) -> Value {
    let form = parse_one(src);
    eval(&form, env).unwrap_or_else(|e| panic!("eval `{src}` failed: {e:?}"))
}

fn extract_counter(map: &Value, key: &str) -> i64 {
    let mut found = 0i64;
    if let Value::Map(m) = map {
        m.for_each(|k, v| {
            if let (Value::Keyword(kw), Value::Long(n)) = (k, v)
                && kw.get().name.as_ref() == key
            {
                found = *n;
            }
        });
    }
    found
}

#[test]
fn run_clojure_compiler_tests() {
    // Source path so `(require 'cljrs.compiler.ir-test)` finds the test files.
    let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test");
    assert!(
        test_dir.is_dir(),
        "test dir not found: {}",
        test_dir.display()
    );

    let _mutator = cljrs_gc::register_mutator();

    let globals = cljrs_stdlib::standard_env_with_paths(vec![test_dir]);

    // `cljrs_stdlib::standard_env` (prebuilt-IR branch) spawns a fire-and-forget
    // thread that loads the compiler namespaces.  If we issue requires before
    // it finishes we get spurious "circular require" errors as the loader
    // crosses paths with itself.  Wait for it to complete.
    while !globals
        .compiler_ready
        .load(std::sync::atomic::Ordering::Acquire)
    {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let mut env = Env::new(globals, "user");

    // The IR-aware dispatch path expects an eval context to be active —
    // some builtins (e.g. `make-lazy-seq`) reach back through
    // `cljrs_env::callback::with_eval_context` to call back into the
    // tree-walking interpreter, and panic if no context is set.
    cljrs_env::callback::push_eval_context(&env);

    eval_str(&mut env, "(require 'clojure.test)");

    let mut total_pass = 0i64;
    let mut total_fail = 0i64;
    let mut total_error = 0i64;
    let mut total_test = 0i64;
    let mut failures: Vec<String> = Vec::new();

    for ns in TEST_NSES {
        eprintln!("[clojure-tests] running {ns}");
        eval_str(&mut env, &format!("(require '{ns})"));
        let result = eval_str(&mut env, &format!("(clojure.test/run-tests '{ns})"));
        let pass = extract_counter(&result, "pass");
        let fail = extract_counter(&result, "fail");
        let error = extract_counter(&result, "error");
        let test = extract_counter(&result, "test");
        total_pass += pass;
        total_fail += fail;
        total_error += error;
        total_test += test;
        if fail > 0 || error > 0 {
            failures.push(format!("{ns}: {fail} fail, {error} error"));
        }
    }

    cljrs_env::callback::pop_eval_context();

    eprintln!(
        "[clojure-tests] totals: {total_test} tests, {total_pass} pass, \
         {total_fail} fail, {total_error} error",
    );

    assert!(total_test > 0, "no tests ran");
    assert!(
        failures.is_empty(),
        "Clojure-side test failures:\n  {}",
        failures.join("\n  "),
    );
}
