use cljrs_env::env::Env;
use cljrs_eval::eval;
use cljrs_interop::Registry;
use cljrs_value::Value;
use std::path::PathBuf;

const TEST_NSES: &[&str] = &["cljrs.base64-test"];

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
fn run_clojure_base64_tests() {
    let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test");
    assert!(
        test_dir.is_dir(),
        "test dir not found: {}",
        test_dir.display()
    );

    let _mutator = cljrs_gc::register_mutator();

    let globals = cljrs_stdlib::standard_env_with_paths(vec![test_dir]);

    // Wait for the background compiler-namespace loader to finish so that
    // `require` doesn't race against it and trigger spurious circular-require errors.
    while !globals
        .compiler_ready
        .load(std::sync::atomic::Ordering::Acquire)
    {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Register base64 native functions.
    let mut registry = Registry::new(globals.clone());
    cljrs_base64::register(&mut registry);

    let mut env = Env::new(globals, "user");

    cljrs_env::callback::push_eval_context(&env);

    eval_str(&mut env, "(require 'clojure.test)");

    let mut total_pass = 0i64;
    let mut total_fail = 0i64;
    let mut total_error = 0i64;
    let mut total_test = 0i64;
    let mut failures: Vec<String> = Vec::new();

    for ns in TEST_NSES {
        eprintln!("[base64-tests] running {ns}");
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
        "[base64-tests] totals: {total_test} tests, {total_pass} pass, \
         {total_fail} fail, {total_error} error",
    );

    assert!(total_test > 0, "no tests ran");
    assert!(
        failures.is_empty(),
        "Clojure-side test failures:\n  {}",
        failures.join("\n  "),
    );
}
