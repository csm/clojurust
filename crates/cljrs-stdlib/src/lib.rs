//! Built-in standard library namespaces for clojurust.
//!
//! Registers `clojure.string`, `clojure.set`, and `clojure.test` into a
//! runtime so they are available via `(require ...)` without needing source
//! files on disk.
//!
//! ## Entry point
//!
//! ```no_run
//! use cljrs_runtime::{ExecutionMode, Runtime};
//!
//! let runtime = Runtime::builder()
//!     .execution_mode(ExecutionMode::Tiered)
//!     .build()
//!     .expect("bootstrap");
//! cljrs_stdlib::install(&runtime);
//! ```
//!
//! This package is an *extension*: it does not construct runtimes and does not
//! decide execution modes.  [`install`] adds namespaces to a runtime the
//! caller already built — the CLI, an embedding host, or the AOT compiler
//! picks the mode, source paths, and GC limits.
//!
//! [`register`] is the same thing addressed by [`GlobalEnv`], for callers that
//! only hold an environment handle.

use std::sync::Arc;

use cljrs_runtime::Runtime;
use cljrs_runtime::env::env::GlobalEnv;

// io and edn use std::fs which is not available on wasm32-unknown-unknown
#[cfg(not(target_arch = "wasm32"))]
mod edn;
#[cfg(not(target_arch = "wasm32"))]
pub mod io;
mod set;
mod string;
// ── Embedded sources ──────────────────────────────────────────────────────────

const CLOJURE_TEST_SRC: &str = include_str!("clojure/test.cljrs");
const CLOJURE_STRING_SRC: &str = include_str!("clojure/string.cljrs");
const CLOJURE_SET_SRC: &str = include_str!("clojure/set.cljrs");
const CLOJURE_TEMPLATE_SRC: &str = include_str!("clojure/template.cljrs");
#[cfg(not(target_arch = "wasm32"))]
const CLOJURE_RUST_IO_SRC: &str = include_str!("clojure/rust/io.cljrs");
#[cfg(not(target_arch = "wasm32"))]
const CLOJURE_EDN_SRC: &str = include_str!("clojure/edn.cljrs");
const CLOJURE_WALK_SRC: &str = include_str!("clojure/walk.cljrs");
const CLOJURE_PPRINT_SRC: &str = include_str!("clojure/pprint.cljrs");
const CLOJURE_DATA_SRC: &str = include_str!("clojure/data.cljrs");
const COLJURE_ZIP_SRC: &str = include_str!("clojure/zip.cljrs");
const CLOJURE_SPEC_ALPHA_SRC: &str = include_str!("clojure/spec/alpha.cljrs");
const CLOJURE_SPEC_GEN_ALPHA_SRC: &str = include_str!("clojure/spec/gen/alpha.cljrs");
const CLOJURE_SPEC_TEST_ALPHA_SRC: &str = include_str!("clojure/spec/test/alpha.cljrs");

// ── Macro: register a batch of native fns into a namespace ───────────────────

/// Register a slice of `(name, arity, fn)` triples as `NativeFunction` values
/// in `$globals` under namespace `$ns`.
macro_rules! register_fns {
    ($globals:expr, $ns:expr, [ $( ($name:expr, $arity:expr, $func:expr) ),* $(,)? ]) => {{
        use cljrs_gc::GcPtr;
        use cljrs_value::{NativeFn, Value};
        let ns: &str = $ns;
        $(
            {
                let nf = NativeFn::new($name, $arity, $func);
                $globals.intern(ns, std::sync::Arc::from($name), Value::NativeFunction(GcPtr::new(nf)));
            }
        )*
    }};
}

pub(crate) use register_fns;

// ── Public API ────────────────────────────────────────────────────────────────

/// Register all built-in stdlib namespaces into `globals`.
///
/// This is idempotent: calling it again does not re-evaluate sources
/// (already-loaded guard in `load_ns` prevents that), but it will
/// overwrite native fn registrations in the namespace tables.
/// In practice, call it once right after `standard_env_minimal()`.
pub fn register(globals: &Arc<GlobalEnv>) {
    // clojure.string ─ pre-register native fns, then register source for
    // the lazy (ns clojure.string) form to run on first require.
    string::register(globals, "clojure.string");
    globals.register_builtin_source("clojure.string", CLOJURE_STRING_SRC);

    // clojure.set ─ same pattern.
    set::register(globals, "clojure.set");
    globals.register_builtin_source("clojure.set", CLOJURE_SET_SRC);

    // clojure.template ─ pure Clojure, no native helpers.
    globals.register_builtin_source("clojure.template", CLOJURE_TEMPLATE_SRC);

    // clojure.test ─ pure Clojure, no native helpers.
    globals.register_builtin_source("clojure.test", CLOJURE_TEST_SRC);

    // clojure.rust.io and clojure.edn use std::fs, unavailable on wasm32.
    #[cfg(not(target_arch = "wasm32"))]
    {
        io::register(globals, "clojure.rust.io");
        globals.register_builtin_source("clojure.rust.io", CLOJURE_RUST_IO_SRC);

        edn::register(globals, "clojure.edn");
        globals.register_builtin_source("clojure.edn", CLOJURE_EDN_SRC);
    }

    // clojure.walk ─ pure Clojure, no native helpers.
    globals.register_builtin_source("clojure.walk", CLOJURE_WALK_SRC);

    // clojure.data ─ pure Clojure, no native helpers.
    globals.register_builtin_source("clojure.data", CLOJURE_DATA_SRC);

    // clojure.pprint ─ pure Clojure; data dispatch only, no cl-format.
    globals.register_builtin_source("clojure.pprint", CLOJURE_PPRINT_SRC);

    // clojure.zip
    globals.register_builtin_source("clojure.zip", COLJURE_ZIP_SRC);

    // clojure.spec.alpha ─ pure Clojure, no native helpers.
    globals.register_builtin_source("clojure.spec.alpha", CLOJURE_SPEC_ALPHA_SRC);

    // clojure.spec.gen.alpha ─ pure Clojure, throwing generator stubs.
    globals.register_builtin_source("clojure.spec.gen.alpha", CLOJURE_SPEC_GEN_ALPHA_SRC);

    // clojure.spec.test.alpha ─ pure Clojure, instrument/unstrument.
    globals.register_builtin_source("clojure.spec.test.alpha", CLOJURE_SPEC_TEST_ALPHA_SRC);
}

/// Install every built-in stdlib namespace into `runtime`.
///
/// Idempotent: calling it again does not re-evaluate sources (`load_ns`'s
/// already-loaded guard prevents that), but it does overwrite the native fn
/// registrations in each namespace.
///
/// Namespaces are registered as embedded *sources*, so each one is parsed and
/// evaluated on its first `require` rather than at install time.
pub fn install(runtime: &Runtime) {
    register(runtime.globals());
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cljrs_reader::Parser;
    use cljrs_runtime::tiered::{Env, EvalResult, eval};
    use cljrs_runtime::{ExecutionMode, Runtime};
    use cljrs_value::{Keyword, Value};

    fn make_env() -> (Arc<GlobalEnv>, Env) {
        let runtime = Runtime::builder()
            .execution_mode(ExecutionMode::Tiered)
            .build()
            .expect("runtime");
        install(&runtime);
        let globals = runtime.globals().clone();
        let env = Env::new(globals.clone(), "user");
        (globals, env)
    }

    #[allow(clippy::result_large_err)]
    fn run(src: &str, env: &mut Env) -> EvalResult {
        let mut parser = Parser::new(src.to_string(), "<test>".to_string());
        let forms = parser.parse_all().expect("parse error");
        let mut result = Value::Nil;
        for form in forms {
            result = eval(&form, env)?;
        }
        Ok(result)
    }

    // ── clojure.string ────────────────────────────────────────────────────────

    #[test]
    fn test_string_upper_lower() {
        let (_, mut env) = make_env();
        run("(require '[clojure.string :as str])", &mut env).unwrap();
        assert_eq!(
            run("(str/upper-case \"hello\")", &mut env).unwrap(),
            Value::string("HELLO")
        );
        assert_eq!(
            run("(str/lower-case \"WORLD\")", &mut env).unwrap(),
            Value::string("world")
        );
    }

    #[test]
    fn test_string_trim() {
        let (_, mut env) = make_env();
        run("(require '[clojure.string :as str])", &mut env).unwrap();
        assert_eq!(
            run("(str/trim \"  hello  \")", &mut env).unwrap(),
            Value::string("hello")
        );
        assert_eq!(
            run("(str/triml \"  hi\")", &mut env).unwrap(),
            Value::string("hi")
        );
        assert_eq!(
            run("(str/trimr \"hi  \")", &mut env).unwrap(),
            Value::string("hi")
        );
    }

    #[test]
    fn test_string_predicates() {
        let (_, mut env) = make_env();
        run("(require '[clojure.string :as str])", &mut env).unwrap();
        assert_eq!(
            run("(str/blank? \"  \")", &mut env).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            run("(str/blank? \"x\")", &mut env).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            run("(str/starts-with? \"hello\" \"hel\")", &mut env).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            run("(str/ends-with? \"hello\" \"llo\")", &mut env).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            run("(str/includes? \"hello\" \"ell\")", &mut env).unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn test_string_replace() {
        let (_, mut env) = make_env();
        run("(require '[clojure.string :as str])", &mut env).unwrap();
        assert_eq!(
            run("(str/replace \"aabbcc\" \"bb\" \"XX\")", &mut env).unwrap(),
            Value::string("aaXXcc")
        );
        assert_eq!(
            run("(str/replace-first \"aabbcc\" \"a\" \"X\")", &mut env).unwrap(),
            Value::string("Xabbcc")
        );
        // regex match (issue #188)
        assert_eq!(
            run("(str/replace \"--host\" #\"^--\" \"\")", &mut env).unwrap(),
            Value::string("host")
        );
        assert_eq!(
            run("(str/replace \"aaa\" #\"a\" \"b\")", &mut env).unwrap(),
            Value::string("bbb")
        );
        assert_eq!(
            run("(str/replace-first \"aaa\" #\"a\" \"b\")", &mut env).unwrap(),
            Value::string("baa")
        );
        assert_eq!(
            run("(str/replace-first \"--host\" #\"^--\" \"\")", &mut env).unwrap(),
            Value::string("host")
        );
    }

    #[test]
    fn test_string_split_join() {
        let (_, mut env) = make_env();
        run("(require '[clojure.string :as str])", &mut env).unwrap();
        let v = run("(str/split \"a,b,c\" \",\")", &mut env).unwrap();
        assert!(matches!(v, Value::Vector(_)));
        assert_eq!(
            run("(str/join \"-\" [\"a\" \"b\" \"c\"])", &mut env).unwrap(),
            Value::string("a-b-c")
        );
    }

    #[test]
    fn test_string_join_char_elements() {
        let (_, mut env) = make_env();
        run("(require '[clojure.string :as str])", &mut env).unwrap();
        // Characters must render as their string value, not reader syntax.
        assert_eq!(
            run(r"(str/join [\8 \0])", &mut env).unwrap(),
            Value::string("80")
        );
        assert_eq!(
            run(r"(str/join \- [\8 \0])", &mut env).unwrap(),
            Value::string("8-0")
        );
        // nil elements are treated as empty string (same as (str nil) = "").
        assert_eq!(
            run(r#"(str/join "-" [nil "a" nil])"#, &mut env).unwrap(),
            Value::string("-a-")
        );
    }

    #[test]
    fn test_string_capitalize() {
        let (_, mut env) = make_env();
        run("(require '[clojure.string :as str])", &mut env).unwrap();
        assert_eq!(
            run("(str/capitalize \"hello world\")", &mut env).unwrap(),
            Value::string("Hello world")
        );
    }

    #[test]
    fn test_string_split_lines() {
        let (_, mut env) = make_env();
        run("(require '[clojure.string :as str])", &mut env).unwrap();
        let v = run("(str/split-lines \"a\\nb\\nc\")", &mut env).unwrap();
        assert!(matches!(v, Value::Vector(_)));
    }

    // ── clojure.set ───────────────────────────────────────────────────────────

    #[test]
    fn test_set_union() {
        let (_, mut env) = make_env();
        run("(require '[clojure.set :as s])", &mut env).unwrap();
        let v = run("(s/union #{1 2} #{2 3})", &mut env).unwrap();
        match v {
            Value::Set(s) => assert_eq!(s.count(), 3),
            other => panic!("expected set, got {other:?}"),
        }
    }

    #[test]
    fn test_set_intersection() {
        let (_, mut env) = make_env();
        run("(require '[clojure.set :as s])", &mut env).unwrap();
        let v = run("(s/intersection #{1 2 3} #{2 3 4})", &mut env).unwrap();
        match v {
            Value::Set(s) => assert_eq!(s.count(), 2),
            other => panic!("expected set, got {other:?}"),
        }
    }

    #[test]
    fn test_set_difference() {
        let (_, mut env) = make_env();
        run("(require '[clojure.set :as s])", &mut env).unwrap();
        let v = run("(s/difference #{1 2 3} #{2 3})", &mut env).unwrap();
        match v {
            Value::Set(s) => assert_eq!(s.count(), 1),
            other => panic!("expected set, got {other:?}"),
        }
    }

    #[test]
    fn test_set_subset_superset() {
        let (_, mut env) = make_env();
        run("(require '[clojure.set :as s])", &mut env).unwrap();
        assert_eq!(
            run("(s/subset? #{1 2} #{1 2 3})", &mut env).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            run("(s/superset? #{1 2 3} #{1 2})", &mut env).unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn test_set_map_invert() {
        let (_, mut env) = make_env();
        run("(require '[clojure.set :as s])", &mut env).unwrap();
        let v = run("(s/map-invert {:a 1 :b 2})", &mut env).unwrap();
        assert!(matches!(v, Value::Map(_)));
    }

    // ── clojure.test (via stdlib registry) ───────────────────────────────────

    #[test]
    fn test_clojure_test_lazy_load() {
        // Run on a thread with adequate stack: the `is` macro expansion
        // triggers eager IR lowering, which calls the Clojure compiler
        // (deeply recursive — needs more than the default 2MB test thread stack).
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                let (_, mut env) = make_env();
                // clojure.test is NOT pre-loaded by the builder; it should
                // load lazily from the registry `install` populated.
                run(
                    "(require '[clojure.test :refer [is deftest run-tests]])",
                    &mut env,
                )
                .unwrap();
                let v = run("(is (= 1 1))", &mut env).unwrap();
                assert_eq!(v, Value::Bool(true));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    // ── clojure.spec.alpha (M1: skeleton + predicate specs + and/or) ─────────

    /// Runs `body` on a thread with a 16MB stack (macro-heavy `spec` loading
    /// triggers eager IR lowering, which recurses deeply — see
    /// `test_clojure_test_lazy_load` above for the same pattern).
    fn run_with_big_stack<F: FnOnce() + Send + 'static>(body: F) {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(body)
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn test_spec_def_and_valid_conform() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            assert_eq!(
                run("(s/def ::x even?)", &mut env).unwrap(),
                Value::keyword(Keyword::qualified("user", "x"))
            );
            assert_eq!(
                run("(s/valid? ::x 4)", &mut env).unwrap(),
                Value::Bool(true)
            );
            assert_eq!(
                run("(s/valid? ::x 3)", &mut env).unwrap(),
                Value::Bool(false)
            );
            assert_eq!(run("(s/conform ::x 4)", &mut env).unwrap(), Value::Long(4));
            assert_eq!(
                run("(s/invalid? (s/conform ::x 3))", &mut env).unwrap(),
                Value::Bool(true)
            );
        });
    }

    #[test]
    fn test_spec_set_and_keyword_ref() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(s/def ::color #{:red :green})", &mut env).unwrap();
            assert_eq!(
                run("(s/valid? ::color :red)", &mut env).unwrap(),
                Value::Bool(true)
            );
            assert_eq!(
                run("(s/valid? ::color :blue)", &mut env).unwrap(),
                Value::Bool(false)
            );

            // keyword-registry-ref spec: ::y re-resolves ::x live.
            run("(s/def ::x even?)", &mut env).unwrap();
            run("(s/def ::y ::x)", &mut env).unwrap();
            assert_eq!(
                run("(s/valid? ::y 4)", &mut env).unwrap(),
                Value::Bool(true)
            );
            assert_eq!(
                run("(s/valid? ::y 3)", &mut env).unwrap(),
                Value::Bool(false)
            );
        });
    }

    #[test]
    fn test_spec_forward_reference() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            // ::a refers to ::b before ::b is defined.
            run("(s/def ::a ::b)", &mut env).unwrap();
            run("(s/def ::b string?)", &mut env).unwrap();
            assert_eq!(
                run(r#"(s/conform ::a "hi")"#, &mut env).unwrap(),
                Value::string("hi")
            );
            assert_eq!(
                run("(s/invalid? (s/conform ::a 5))", &mut env).unwrap(),
                Value::Bool(true)
            );
        });
    }

    #[test]
    fn test_spec_and_or() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            assert_eq!(
                run("(s/valid? (s/and int? even?) 4)", &mut env).unwrap(),
                Value::Bool(true)
            );
            assert_eq!(
                run("(s/valid? (s/and int? even?) 3)", &mut env).unwrap(),
                Value::Bool(false)
            );
            let v = run("(s/conform (s/or :i int? :s string?) 5)", &mut env).unwrap();
            match v {
                Value::Vector(vec) => {
                    let items = vec.get().iter().cloned().collect::<Vec<_>>();
                    assert_eq!(items.len(), 2);
                    assert_eq!(items[0], Value::keyword(Keyword::simple("i")));
                    assert_eq!(items[1], Value::Long(5));
                }
                other => panic!("expected [:i 5], got {other:?}"),
            }
        });
    }

    #[test]
    fn test_spec_spec_and_registry_introspection() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            // Bare predicates are directly usable but not `spec?`; our own
            // record-based compound specs are.
            assert_eq!(run("(s/spec? even?)", &mut env).unwrap(), Value::Nil);
            let is_spec = run("(s/spec? (s/and int? even?))", &mut env).unwrap();
            assert!(
                !matches!(is_spec, Value::Nil),
                "expected a truthy spec object"
            );

            run("(s/def ::x even?)", &mut env).unwrap();
            assert!(!matches!(
                run("(s/get-spec ::x)", &mut env).unwrap(),
                Value::Nil
            ));
            assert!(matches!(
                run("(s/get-spec ::does-not-exist)", &mut env).unwrap(),
                Value::Nil
            ));

            run("(s/def ::pos-even (s/and int? even? pos?))", &mut env).unwrap();
            let form_v = run("(s/form ::pos-even)", &mut env).unwrap();
            assert_eq!(format!("{form_v}"), "(and int? even? pos?)");
            let describe_v = run("(s/describe ::pos-even)", &mut env).unwrap();
            assert_eq!(format!("{describe_v}"), "(and int? even? pos?)");
        });
    }

    #[test]
    fn test_spec_conform_unregistered_keyword_throws() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            let err = run("(s/conform ::does-not-exist 1)", &mut env).unwrap_err();
            let msg = format!("{err}");
            assert!(
                msg.contains("Unable to resolve spec"),
                "expected 'Unable to resolve spec' in error, got: {msg}"
            );
        });
    }

    // ── clojure.spec.alpha (M2: explain + keys/merge) ────────────────────────

    /// Asserts that a Clojure expression evaluates to exactly `true`/`false`.
    #[track_caller]
    fn assert_bool(src: &str, expected: bool, env: &mut Env) {
        assert_eq!(
            run(src, env).unwrap(),
            Value::Bool(expected),
            "expression: {src}"
        );
    }

    #[test]
    fn test_spec_keys_req_opt() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(s/def ::a int?) (s/def ::b string?)", &mut env).unwrap();
            run("(s/def ::m (s/keys :req [::a] :opt [::b]))", &mut env).unwrap();
            assert_bool("(s/valid? ::m {::a 1})", true, &mut env);
            assert_bool("(s/valid? ::m {::a 1 ::b \"x\"})", true, &mut env);
            assert_bool("(s/valid? ::m {::b \"x\"})", false, &mut env); // missing req
            assert_bool("(s/valid? ::m {::a \"no\"})", false, &mut env); // bad req value
            assert_bool("(s/valid? ::m {::a 1 ::b 2})", false, &mut env); // bad opt value
            assert_bool("(s/valid? ::m 42)", false, &mut env); // not a map
            // undeclared-but-registered key is still validated (upstream semantics)
            assert_bool(
                "(s/valid? (s/keys :req [::a]) {::a 1 ::b :not-a-string})",
                false,
                &mut env,
            );
            // connectives in :req
            run("(s/def ::c boolean?)", &mut env).unwrap();
            run(
                "(s/def ::conn (s/keys :req [(or ::a (and ::b ::c))]))",
                &mut env,
            )
            .unwrap();
            assert_bool("(s/valid? ::conn {::a 1})", true, &mut env);
            assert_bool("(s/valid? ::conn {::b \"x\" ::c true})", true, &mut env);
            assert_bool("(s/valid? ::conn {::b \"x\"})", false, &mut env);
            assert_bool("(s/valid? ::conn {})", false, &mut env);
        });
    }

    #[test]
    fn test_spec_keys_un_variants() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(s/def ::a int?) (s/def ::b string?)", &mut env).unwrap();
            run(
                "(s/def ::mu (s/keys :req-un [::a] :opt-un [::b]))",
                &mut env,
            )
            .unwrap();
            assert_bool("(s/valid? ::mu {:a 1})", true, &mut env);
            assert_bool("(s/valid? ::mu {:a 1 :b \"x\"})", true, &mut env);
            assert_bool("(s/valid? ::mu {:b \"x\"})", false, &mut env); // missing req-un
            assert_bool("(s/valid? ::mu {:a \"no\"})", false, &mut env); // bad value
            assert_bool("(s/valid? ::mu {:a 1 :b 2})", false, &mut env); // bad opt-un value
            assert_bool("(= {:a 1} (s/conform ::mu {:a 1}))", true, &mut env);
        });
    }

    #[test]
    fn test_spec_keys_record() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(defrecord Point [x y])", &mut env).unwrap();
            run("(s/def ::x number?) (s/def ::y number?)", &mut env).unwrap();
            run("(s/def ::point (s/keys :req-un [::x ::y]))", &mut env).unwrap();
            assert_bool("(s/valid? ::point (->Point 1 2))", true, &mut env);
            assert_bool("(s/valid? ::point (->Point \"a\" 2))", false, &mut env);
            // conform on a record returns a record (assoc preserves type tag)
            assert_bool(
                "(record? (s/conform ::point (->Point 1 2)))",
                true,
                &mut env,
            );
        });
    }

    #[test]
    fn test_spec_explain_data_shape() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(s/def ::a int?) (s/def ::b string?)", &mut env).unwrap();
            run("(s/def ::m (s/keys :req [::a] :opt [::b]))", &mut env).unwrap();

            // Missing-req problem: (contains? % :user/a)-style pred, path/in [].
            run("(def ed1 (s/explain-data ::m {::b \"x\"}))", &mut env).unwrap();
            assert_bool(
                "(pos? (count (:clojure.spec.alpha/problems ed1)))",
                true,
                &mut env,
            );
            let pred = run(
                "(pr-str (:pred (first (:clojure.spec.alpha/problems ed1))))",
                &mut env,
            )
            .unwrap();
            assert_eq!(pred, Value::string("(contains? % :user/a)"));
            assert_bool(
                "(= [] (:path (first (:clojure.spec.alpha/problems ed1))))",
                true,
                &mut env,
            );

            // Bad-value problem: path/in extended by the key, via extended by
            // both the named keys spec and the failing key's spec.
            run("(def ed2 (s/explain-data ::m {::a \"no\"}))", &mut env).unwrap();
            run(
                "(def p2 (first (:clojure.spec.alpha/problems ed2)))",
                &mut env,
            )
            .unwrap();
            assert_bool("(= [::a] (:path p2))", true, &mut env);
            assert_bool("(= [::a] (:in p2))", true, &mut env);
            assert_bool("(= [::m ::a] (:via p2))", true, &mut env);
            assert_bool("(= \"no\" (:val p2))", true, &mut env);

            // Valid value → nil.
            assert_bool("(nil? (s/explain-data ::m {::a 4}))", true, &mut env);

            // Whole-map value and spec recorded on the explain-data map.
            assert_bool(
                "(= {::a \"no\"} (:clojure.spec.alpha/value ed2))",
                true,
                &mut env,
            );
        });
    }

    #[test]
    fn test_spec_merge() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(s/def ::a int?) (s/def ::b string?)", &mut env).unwrap();
            run("(s/def ::ma (s/keys :req [::a]))", &mut env).unwrap();
            run("(s/def ::mb (s/keys :req [::b]))", &mut env).unwrap();
            run("(s/def ::mab (s/merge ::ma ::mb))", &mut env).unwrap();
            assert_bool("(s/valid? ::mab {::a 1 ::b \"x\"})", true, &mut env);
            assert_bool("(s/valid? ::mab {::a 1})", false, &mut env);
            assert_bool(
                "(= {::a 1 ::b \"x\"} (s/conform ::mab {::a 1 ::b \"x\"}))",
                true,
                &mut env,
            );
            assert_bool(
                "(pos? (count (:clojure.spec.alpha/problems
                               (s/explain-data ::mab {::a 1}))))",
                true,
                &mut env,
            );
        });
    }

    #[test]
    fn test_spec_explain_str_and_printer() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(s/def ::a int?)", &mut env).unwrap();
            run("(s/def ::m (s/keys :req [::a]))", &mut env).unwrap();
            let estr = match run("(s/explain-str ::m {::a \"no\"})", &mut env).unwrap() {
                Value::Str(s) => s.get().clone(),
                other => panic!("expected string from explain-str, got {other:?}"),
            };
            assert!(
                estr.contains("failed") && estr.contains("int?"),
                "unexpected explain-str output: {estr}"
            );
            let ok = run("(s/explain-str ::m {::a 1})", &mut env).unwrap();
            assert_eq!(ok, Value::string("Success!\n"));
        });
    }

    #[test]
    fn test_spec_keys_unform_roundtrip() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(s/def ::id (s/or :i int? :s string?))", &mut env).unwrap();
            run("(s/def ::rm (s/keys :req-un [::id]))", &mut env).unwrap();
            // conform tags the or-valued key; unform untags it.
            assert_bool("(= {:id [:i 5]} (s/conform ::rm {:id 5}))", true, &mut env);
            assert_bool(
                "(= {:id 5} (s/unform ::rm (s/conform ::rm {:id 5})))",
                true,
                &mut env,
            );
        });
    }

    // ── clojure.spec.alpha (M3: regex engine — cat/alt/*/+/?/&) ─────────────

    #[test]
    fn test_spec_regex_cat_and_nesting() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            assert_bool(
                "(= {:a 1 :b \"x\"} (s/conform (s/cat :a int? :b string?) [1 \"x\"]))",
                true,
                &mut env,
            );
            // Nested regex splices into the same flat sequence.
            assert_bool(
                "(= {:a 1 :b [\"x\" \"y\"]}
                    (s/conform (s/cat :a int? :b (s/* string?)) [1 \"x\" \"y\"]))",
                true,
                &mut env,
            );
            // Wrong element type / too short / too long are all invalid.
            assert_bool(
                "(s/invalid? (s/conform (s/cat :a int? :b string?) [1 2]))",
                true,
                &mut env,
            );
            assert_bool(
                "(s/invalid? (s/conform (s/cat :a int? :b string?) [1]))",
                true,
                &mut env,
            );
            assert_bool(
                "(s/invalid? (s/conform (s/cat :a int?) [1 2]))",
                true,
                &mut env,
            );
            // Non-sequential input is invalid, not an error.
            assert_bool("(s/invalid? (s/conform (s/cat :a int?) 5))", true, &mut env);
        });
    }

    #[test]
    fn test_spec_regex_alt_star_plus_maybe() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            // alt tags its branch.
            assert_bool(
                "(= [:i 5] (s/conform (s/alt :i int? :s string?) [5]))",
                true,
                &mut env,
            );
            assert_bool(
                "(s/invalid? (s/conform (s/alt :i int? :s string?) [:kw]))",
                true,
                &mut env,
            );
            // * matches empty and many; fails on a bad element.
            assert_bool("(= [] (s/conform (s/* int?) []))", true, &mut env);
            assert_bool("(s/valid? (s/* int?) [])", true, &mut env);
            assert_bool("(= [1 2 3] (s/conform (s/* int?) [1 2 3]))", true, &mut env);
            assert_bool(
                "(s/invalid? (s/conform (s/* int?) [1 :a 3]))",
                true,
                &mut env,
            );
            // + requires at least one.
            assert_bool("(= [1] (s/conform (s/+ int?) [1]))", true, &mut env);
            assert_bool("(s/invalid? (s/conform (s/+ int?) []))", true, &mut env);
            // ? is optional: value when present, nil when absent.
            assert_bool("(= 5 (s/conform (s/? int?) [5]))", true, &mut env);
            assert_bool("(nil? (s/conform (s/? int?) []))", true, &mut env);
            assert_bool("(s/valid? (s/? int?) [])", true, &mut env);
            assert_bool("(s/invalid? (s/conform (s/? int?) [1 2]))", true, &mut env);
        });
    }

    #[test]
    fn test_spec_regex_amp_and_regex_inside_and() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(def even-count? (fn [xs] (even? (count xs))))", &mut env).unwrap();
            // & applies post-predicates to the conformed regex result.
            assert_bool(
                "(= [1 2] (s/conform (s/& (s/* int?) even-count?) [1 2]))",
                true,
                &mut env,
            );
            assert_bool(
                "(s/invalid? (s/conform (s/& (s/* int?) even-count?) [1 2 3]))",
                true,
                &mut env,
            );
            // A regex op nested in s/and conforms the whole seq first, then
            // threads the conformed value through the remaining preds.
            run("(def all-even? (fn [xs] (every? even? xs)))", &mut env).unwrap();
            assert_bool(
                "(= [2 4] (s/conform (s/and (s/* int?) all-even?) [2 4]))",
                true,
                &mut env,
            );
            assert_bool(
                "(s/valid? (s/and (s/* int?) all-even?) [1 2])",
                false,
                &mut env,
            );
        });
    }

    #[test]
    fn test_spec_regex_keyword_ref_children() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            // A keyword ref to a NON-regex spec consumes exactly one element.
            run("(s/def ::a int?)", &mut env).unwrap();
            assert_bool(
                "(= {:x 1 :y \"x\"} (s/conform (s/cat :x ::a :y string?) [1 \"x\"]))",
                true,
                &mut env,
            );
            // A keyword ref to a REGEX spec splices (upstream reg-resolve!
            // semantics).
            run("(s/def ::r (s/* int?))", &mut env).unwrap();
            assert_bool(
                "(= {:nums [1 2] :tail \"x\"}
                    (s/conform (s/cat :nums ::r :tail string?) [1 2 \"x\"]))",
                true,
                &mut env,
            );
            // s/spec wrapping forces a nested one-element boundary instead.
            assert_bool(
                "(= {:nums [1 2] :tail \"x\"}
                    (s/conform (s/cat :nums (s/spec (s/* int?)) :tail string?)
                               [[1 2] \"x\"]))",
                true,
                &mut env,
            );
            // Registered regex works at top level via the keyword.
            assert_bool("(= [1 2 3] (s/conform ::r [1 2 3]))", true, &mut env);
        });
    }

    #[test]
    fn test_spec_regex_explain_reasons() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            // Premature end of input → "Insufficient input" at the missing key.
            run(
                "(def p-insuff (first (:clojure.spec.alpha/problems
                                       (s/explain-data (s/cat :a int? :b string?) [1]))))",
                &mut env,
            )
            .unwrap();
            assert_bool(
                "(= \"Insufficient input\" (:reason p-insuff))",
                true,
                &mut env,
            );
            assert_bool("(= [:b] (:path p-insuff))", true, &mut env);
            // Leftover input → "Extra input" with :in pointing at the index.
            run(
                "(def p-extra (first (:clojure.spec.alpha/problems
                                      (s/explain-data (s/cat :a int?) [1 2]))))",
                &mut env,
            )
            .unwrap();
            assert_bool("(= \"Extra input\" (:reason p-extra))", true, &mut env);
            assert_bool("(= [1] (:in p-extra))", true, &mut env);
            // Element failure mid-sequence: path names the cat key, in the index.
            run(
                "(def p-elem (first (:clojure.spec.alpha/problems
                                     (s/explain-data (s/cat :a int? :b string?) [1 :bad]))))",
                &mut env,
            )
            .unwrap();
            assert_bool("(= [:b] (:path p-elem))", true, &mut env);
            assert_bool("(= [1] (:in p-elem))", true, &mut env);
            assert_bool("(= :bad (:val p-elem))", true, &mut env);
        });
    }

    #[test]
    fn test_spec_regex_unform_roundtrips() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(def even-count? (fn [xs] (even? (count xs))))", &mut env).unwrap();
            for (spec, input) in [
                ("(s/cat :a int? :b string?)", "[1 \"x\"]"),
                ("(s/cat :a int? :b (s/* string?))", "[1 \"x\" \"y\"]"),
                ("(s/alt :i int? :s string?)", "[5]"),
                ("(s/* int?)", "[1 2 3]"),
                ("(s/* int?)", "[]"),
                ("(s/+ int?)", "[1 2]"),
                ("(s/? int?)", "[5]"),
                ("(s/? int?)", "[]"),
                ("(s/& (s/* int?) even-count?)", "[1 2]"),
            ] {
                assert_bool(
                    &format!(
                        "(let [re {spec}] (= {input} (vec (s/unform re (s/conform re {input})))))"
                    ),
                    true,
                    &mut env,
                );
            }
        });
    }

    #[test]
    fn test_spec_plain_map_is_not_a_spec() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            let err = run("(s/conform {:a 1} 5)", &mut env).unwrap_err();
            let msg = format!("{err}");
            assert!(
                msg.contains("not a valid spec"),
                "expected 'not a valid spec' in error, got: {msg}"
            );
        });
    }

    // ── clojure.spec.alpha (M4: collections + leaf specs) ────────────────────

    #[test]
    fn test_spec_coll_of_options() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            // happy / sad
            assert_bool(
                "(= [1 2 3] (s/conform (s/coll-of int?) [1 2 3]))",
                true,
                &mut env,
            );
            assert_bool(
                "(s/invalid? (s/conform (s/coll-of int?) [1 \"x\"]))",
                true,
                &mut env,
            );
            // :kind
            assert_bool(
                "(s/invalid? (s/conform (s/coll-of int? :kind vector?) '(1 2 3)))",
                true,
                &mut env,
            );
            // :min-count / :max-count
            assert_bool(
                "(s/invalid? (s/conform (s/coll-of int? :min-count 3) [1 2]))",
                true,
                &mut env,
            );
            assert_bool(
                "(s/invalid? (s/conform (s/coll-of int? :max-count 2) [1 2 3]))",
                true,
                &mut env,
            );
            // :distinct
            assert_bool(
                "(= [1 2 3] (s/conform (s/coll-of int? :distinct true) [1 2 3]))",
                true,
                &mut env,
            );
            assert_bool(
                "(s/invalid? (s/conform (s/coll-of int? :distinct true) [1 1 2]))",
                true,
                &mut env,
            );
            // :into
            assert_bool(
                "(= #{1 2 3} (s/conform (s/coll-of int? :into #{}) [1 2 3]))",
                true,
                &mut env,
            );
            // default (no :into) preserves order for list input; explicit
            // :into '() conjes raw (reverses) — see EverySpec doc comment.
            assert_bool(
                "(= '(1 2 3) (s/conform (s/coll-of int?) '(1 2 3)))",
                true,
                &mut env,
            );
            assert_bool(
                "(= '(3 2 1) (s/conform (s/coll-of int? :into '()) '(1 2 3)))",
                true,
                &mut env,
            );
        });
    }

    #[test]
    fn test_spec_map_of_key_handling() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            // default: keys must be valid but ORIGINAL keys are kept
            assert_bool(
                "(= {:a 1 :b 2} (s/conform (s/map-of keyword? int?) {:a 1 :b 2}))",
                true,
                &mut env,
            );
            assert_bool(
                "(s/invalid? (s/conform (s/map-of keyword? int?) {:a \"x\"}))",
                true,
                &mut env,
            );
            assert_bool(
                "(s/invalid? (s/conform (s/map-of keyword? int?) {\"not-kw\" 1}))",
                true,
                &mut env,
            );
            // :conform-keys true swaps in the conformed key
            assert_bool(
                r#"(= {"a" 1} (s/conform (s/map-of (s/conformer name) int? :conform-keys true) {:a 1}))"#,
                true,
                &mut env,
            );
        });
    }

    #[test]
    fn test_spec_every_vs_coll_of_conform_difference() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(def elem (s/or :i int? :s string?))", &mut env).unwrap();
            // coll-of conforms every element (tagged vectors)
            assert_bool(
                "(= [[:i 1] [:s \"x\"]] (s/conform (s/coll-of elem) [1 \"x\"]))",
                true,
                &mut env,
            );
            // every only validates — returns x unchanged
            assert_bool(
                "(= [1 \"x\"] (s/conform (s/every elem) [1 \"x\"]))",
                true,
                &mut env,
            );
            assert_bool(
                "(s/invalid? (s/conform (s/every elem) [1 :bad]))",
                true,
                &mut env,
            );
            // every-kv validates both k/v but never rebuilds
            assert_bool(
                "(= {:a 1} (s/conform (s/every-kv keyword? int?) {:a 1}))",
                true,
                &mut env,
            );
            assert_bool(
                "(s/invalid? (s/conform (s/every-kv keyword? int?) {:a \"x\"}))",
                true,
                &mut env,
            );
        });
    }

    #[test]
    fn test_spec_tuple_conform_and_explain() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            assert_bool(
                "(= [1 \"x\"] (s/conform (s/tuple int? string?) [1 \"x\"]))",
                true,
                &mut env,
            );
            assert_bool(
                "(s/invalid? (s/conform (s/tuple int? string?) [1]))",
                true,
                &mut env,
            );
            assert_bool(
                "(s/invalid? (s/conform (s/tuple int? string?) 5))",
                true,
                &mut env,
            );
            run(
                "(def tp (first (:clojure.spec.alpha/problems (s/explain-data (s/tuple int? string?) [1 :bad]))))",
                &mut env,
            )
            .unwrap();
            assert_bool("(= [1] (:path tp))", true, &mut env);
            assert_bool("(= [1] (:in tp))", true, &mut env);
            assert_bool("(= :bad (:val tp))", true, &mut env);
            // round-trip unform
            assert_bool(
                "(= [1 \"x\"] (s/unform (s/tuple int? string?) (s/conform (s/tuple int? string?) [1 \"x\"])))",
                true,
                &mut env,
            );
        });
    }

    #[test]
    fn test_spec_nilable_both_branches_and_explain_shape() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            assert_bool("(= nil (s/conform (s/nilable int?) nil))", true, &mut env);
            assert_bool("(= 5 (s/conform (s/nilable int?) 5))", true, &mut env);
            assert_bool(
                "(s/invalid? (s/conform (s/nilable int?) \"x\"))",
                true,
                &mut env,
            );
            run(
                "(def np (:clojure.spec.alpha/problems (s/explain-data (s/nilable int?) \"x\")))",
                &mut env,
            )
            .unwrap();
            assert_bool("(= 2 (count np))", true, &mut env);
            assert_bool(
                "(some (fn [p] (= [:clojure.spec.alpha/nil] (:path p))) np)",
                true,
                &mut env,
            );
            assert_bool(
                "(some (fn [p] (= [:clojure.spec.alpha/pred] (:path p))) np)",
                true,
                &mut env,
            );
        });
    }

    #[test]
    fn test_spec_multi_spec_conform_and_no_method() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(defmulti event-type :type)", &mut env).unwrap();
            run("(s/def :evt/type keyword?)", &mut env).unwrap();
            run("(s/def :evt/a (s/keys :req-un [:evt/type]))", &mut env).unwrap();
            run("(defmethod event-type :a [_] :evt/a)", &mut env).unwrap();
            run(
                "(s/def :evt/event (s/multi-spec event-type :type))",
                &mut env,
            )
            .unwrap();
            assert_bool(
                "(= {:type :a} (s/conform :evt/event {:type :a}))",
                true,
                &mut env,
            );
            assert_bool(
                "(s/invalid? (s/conform :evt/event {:type :unknown}))",
                true,
                &mut env,
            );
            run(
                "(def mp (first (:clojure.spec.alpha/problems (s/explain-data :evt/event {:type :unknown}))))",
                &mut env,
            )
            .unwrap();
            assert_bool("(= \"no method\" (:reason mp))", true, &mut env);
        });
    }

    #[test]
    fn test_spec_conformer_transforms_and_threads_through_and() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            assert_bool(
                "(= 10 (s/conform (s/conformer #(* 2 %)) 5))",
                true,
                &mut env,
            );
            // threads the transformed value through the rest of s/and
            assert_bool(
                "(= 10 (s/conform (s/and int? (s/conformer #(* 2 %))) 5))",
                true,
                &mut env,
            );
            assert_bool(
                "(= 5 (s/unform (s/conformer #(* 2 %) #(/ % 2)) 10))",
                true,
                &mut env,
            );
            // no unf given -> unform is identity
            assert_bool(
                "(= 10 (s/unform (s/conformer #(* 2 %)) 10))",
                true,
                &mut env,
            );
        });
    }

    #[test]
    fn test_spec_int_in_and_double_in_bounds() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            assert_bool("(s/valid? (s/int-in 0 10) 5)", true, &mut env);
            assert_bool("(s/valid? (s/int-in 0 10) 10)", false, &mut env); // end exclusive
            assert_bool("(s/valid? (s/int-in 0 10) 5.0)", false, &mut env); // not an int?
            assert_bool(
                "(s/valid? (s/double-in :min 0.0 :max 10.0) 5.0)",
                true,
                &mut env,
            );
            assert_bool(
                "(s/valid? (s/double-in :min 0.0 :max 10.0) 20.0)",
                false,
                &mut env,
            );
            // NaN produced via arithmetic (NOT the ##NaN reader literal — that
            // literal hangs this runtime indefinitely on eval, a pre-existing
            // bug unrelated to spec.alpha; see M4 report).
            run("(def nan (/ 0.0 0.0))", &mut env).unwrap();
            run("(def pos-inf (/ 1.0 0.0))", &mut env).unwrap();
            assert_bool("(s/valid? (s/double-in :NaN? false) nan)", false, &mut env);
            assert_bool("(s/valid? (s/double-in) nan)", true, &mut env);
            assert_bool(
                "(s/valid? (s/double-in :infinite? false) pos-inf)",
                false,
                &mut env,
            );
            assert_bool("(s/valid? (s/double-in) pos-inf)", true, &mut env);
            // nonconforming: validates but returns x unconformed
            assert_bool(
                "(= [1 \"x\"] (s/conform (s/nonconforming (s/cat :a int? :b string?)) [1 \"x\"]))",
                true,
                &mut env,
            );
            // inst-in: not implemented, throws clearly
            let err = run("(s/inst-in 0 1)", &mut env).unwrap_err();
            let msg = format!("{err}");
            assert!(
                msg.contains("not implemented"),
                "expected 'not implemented' in error, got: {msg}"
            );
        });
    }

    // ── clojure.spec.alpha / .test.alpha / .gen.alpha (M5: fdef/instrument) ──

    #[test]
    fn test_spec_fdef_registers_qualified_symbol_and_get_spec() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(defn my-add [a b] (+ a b))", &mut env).unwrap();
            assert_bool(
                "(= 'user/my-add (s/fdef my-add :args (s/cat :a int? :b int?) :ret int?))",
                true,
                &mut env,
            );
            assert_bool("(s/fspec? (s/get-spec 'user/my-add))", true, &mut env);
            assert_bool("(some? (:args (s/get-spec 'user/my-add)))", true, &mut env);
            // syntax-quote auto-qualification against the current ns resolves
            // to the same registry entry as the explicit qualified symbol.
            assert_bool(
                "(= (s/get-spec 'user/my-add) (s/get-spec `my-add))",
                true,
                &mut env,
            );
        });
    }

    #[test]
    fn test_spec_instrument_valid_and_invalid_calls() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(require '[clojure.spec.test.alpha :as stest])", &mut env).unwrap();
            run("(defn my-add [a b] (+ a b))", &mut env).unwrap();
            run(
                "(s/fdef my-add :args (s/cat :a int? :b int?) :ret int?)",
                &mut env,
            )
            .unwrap();
            run("(stest/instrument 'user/my-add)", &mut env).unwrap();
            // valid call still returns the normal value through the wrapper.
            assert_eq!(run("(my-add 2 3)", &mut env).unwrap(), Value::Long(5));
            // invalid call throws with the expected message and ex-data.
            run(
                r#"(def caught
                     (try (my-add 2 "x") :did-not-throw
                          (catch Exception e {:msg (ex-message e) :data (ex-data e)})))"#,
                &mut env,
            )
            .unwrap();
            assert_bool(
                r#"(= "Call to user/my-add did not conform to spec." (:msg caught))"#,
                true,
                &mut env,
            );
            assert_bool(
                "(= :instrument (:clojure.spec.alpha/failure (:data caught)))",
                true,
                &mut env,
            );
            assert_bool(
                "(contains? (:data caught) :clojure.spec.test.alpha/caller)",
                true,
                &mut env,
            );
        });
    }

    #[test]
    fn test_spec_unstrument_restores_raw_fn() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(require '[clojure.spec.test.alpha :as stest])", &mut env).unwrap();
            run("(defn my-pair [a b] [a b])", &mut env).unwrap();
            run("(s/fdef my-pair :args (s/cat :a int? :b int?))", &mut env).unwrap();
            run("(stest/instrument 'user/my-pair)", &mut env).unwrap();
            assert!(run("(my-pair 1 \"x\")", &mut env).is_err());
            run("(stest/unstrument 'user/my-pair)", &mut env).unwrap();
            // no longer wrapped -- the raw fn has no type constraints, so an
            // "invalid by spec" call now succeeds instead of throwing.
            assert_bool("(= [1 \"x\"] (my-pair 1 \"x\"))", true, &mut env);
        });
    }

    #[test]
    fn test_spec_instrument_affects_call_sites_evaluated_before_instrument() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(require '[clojure.spec.test.alpha :as stest])", &mut env).unwrap();
            run("(defn my-mul [a b] (* a b))", &mut env).unwrap();
            run("(s/fdef my-mul :args (s/cat :a int? :b int?))", &mut env).unwrap();
            // Warm up this call site (and any inline-cached/lowered code for
            // it) BEFORE instrumenting.
            assert_eq!(run("(my-mul 2 3)", &mut env).unwrap(), Value::Long(6));
            run("(stest/instrument 'user/my-mul)", &mut env).unwrap();
            // A stale inline cache pointing at the pre-instrument var root
            // would silently skip the spec check here.
            assert!(run("(my-mul 2 \"x\")", &mut env).is_err());
        });
    }

    #[test]
    fn test_spec_instrumentable_syms_and_idempotent_double_instrument() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(require '[clojure.spec.test.alpha :as stest])", &mut env).unwrap();
            run("(defn my-pair2 [a b] [a b])", &mut env).unwrap();
            run("(s/fdef my-pair2 :args (s/cat :a int? :b int?))", &mut env).unwrap();
            assert_bool(
                "(boolean (some #{'user/my-pair2} (stest/instrumentable-syms)))",
                true,
                &mut env,
            );
            run("(stest/instrument 'user/my-pair2)", &mut env).unwrap();
            run("(stest/instrument 'user/my-pair2)", &mut env).unwrap(); // idempotent
            assert!(run("(my-pair2 1 \"x\")", &mut env).is_err());
            // A SINGLE unstrument must fully restore -- if double-instrument
            // had double-wrapped, one layer would still be active and the
            // call below would still throw.
            run("(stest/unstrument 'user/my-pair2)", &mut env).unwrap();
            assert_bool("(= [1 \"x\"] (my-pair2 1 \"x\"))", true, &mut env);
        });
    }

    #[test]
    fn test_spec_with_instrument_disabled_bypasses_checks() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(require '[clojure.spec.test.alpha :as stest])", &mut env).unwrap();
            run("(defn my-pair3 [a b] [a b])", &mut env).unwrap();
            run("(s/fdef my-pair3 :args (s/cat :a int? :b int?))", &mut env).unwrap();
            run("(stest/instrument 'user/my-pair3)", &mut env).unwrap();
            assert!(run("(my-pair3 1 \"x\")", &mut env).is_err());
            assert_bool(
                "(stest/with-instrument-disabled (= [1 \"x\"] (my-pair3 1 \"x\")))",
                true,
                &mut env,
            );
            // instrumentation resumes outside the dynamic extent.
            assert!(run("(my-pair3 1 \"x\")", &mut env).is_err());
        });
    }

    #[test]
    fn test_spec_assert_happy_sad_and_check_asserts_toggle() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            assert_eq!(run("(s/assert even? 4)", &mut env).unwrap(), Value::Long(4));
            assert!(run("(s/assert even? 5)", &mut env).is_err());
            run("(s/check-asserts false)", &mut env).unwrap();
            // a freshly (re-)macroexpanded s/assert form reads
            // *compile-asserts* at macroexpansion time and passes an invalid
            // value through unchecked.
            assert_eq!(run("(s/assert even? 5)", &mut env).unwrap(), Value::Long(5));
            run("(s/check-asserts true)", &mut env).unwrap();
            assert!(run("(s/assert even? 5)", &mut env).is_err());
        });
    }

    #[test]
    fn test_spec_gen_and_check_throw_not_implemented_and_gen_ns_loads() {
        run_with_big_stack(|| {
            let (_, mut env) = make_env();
            run("(require '[clojure.spec.alpha :as s])", &mut env).unwrap();
            run("(require '[clojure.spec.test.alpha :as stest])", &mut env).unwrap();
            run("(require '[clojure.spec.gen.alpha :as gen])", &mut env).unwrap();
            for src in [
                "(s/gen even?)",
                "(s/exercise even?)",
                "(s/exercise-fn 'user/foo)",
                "(stest/check)",
                "(stest/check 'user/foo)",
                "(stest/check-fn (fn [x] x) even?)",
                "(gen/generate :whatever)",
                "(gen/sample :whatever)",
                "(gen/elements [1 2 3])",
            ] {
                assert!(run(src, &mut env).is_err(), "expected {src} to throw");
            }
        });
    }
}
