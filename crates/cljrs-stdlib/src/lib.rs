//! Built-in standard library namespaces for clojurust.
//!
//! Registers `clojure.string`, `clojure.set`, and `clojure.test` into a
//! [`GlobalEnv`] so they are available via `(require ...)` without needing
//! source files on disk.
//!
//! ## Entry points
//!
//! - [`standard_env()`] — full environment for the `cljrs` binary
//! - [`standard_env_with_paths()`] — same, plus user source paths
//! - [`register()`] — add stdlib to an existing env (e.g. for testing)

use std::sync::Arc;

use cljrs_eval::GlobalEnv;

mod edn;
pub mod io;
mod set;
mod string;
mod core_async;
// ── Embedded sources ──────────────────────────────────────────────────────────

const CLOJURE_TEST_SRC: &str = include_str!("clojure/test.cljrs");
const CLOJURE_STRING_SRC: &str = include_str!("clojure/string.cljrs");
const CLOJURE_SET_SRC: &str = include_str!("clojure/set.cljrs");
const CLOJURE_TEMPLATE_SRC: &str = include_str!("clojure/template.cljrs");
const CLOJURE_RUST_IO_SRC: &str = include_str!("clojure/rust/io.cljrs");
const CLOJURE_EDN_SRC: &str = include_str!("clojure/edn.cljrs");
const CLOJURE_WALK_SRC: &str = include_str!("clojure/walk.cljrs");
const CLOJURE_DATA_SRC: &str = include_str!("clojure/data.cljrs");
const COLJURE_ZIP_SRC: &str = include_str!("clojure/zip.cljrs");

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

    // clojure.rust.io ─ I/O resources.
    io::register(globals, "clojure.rust.io");
    globals.register_builtin_source("clojure.rust.io", CLOJURE_RUST_IO_SRC);

    // clojure.edn ─ EDN reader.
    edn::register(globals, "clojure.edn");
    globals.register_builtin_source("clojure.edn", CLOJURE_EDN_SRC);

    // clojure.walk ─ pure Clojure, no native helpers.
    globals.register_builtin_source("clojure.walk", CLOJURE_WALK_SRC);

    // clojure.data ─ pure Clojure, no native helpers.
    globals.register_builtin_source("clojure.data", CLOJURE_DATA_SRC);

    // clojure.zip
    globals.register_builtin_source("clojure.zip", COLJURE_ZIP_SRC);
}

/// Create a `GlobalEnv` with all built-ins and stdlib registered.
///
/// Prefer this over `cljrs_eval::standard_env()` in the `cljrs` binary so that
/// stdlib namespaces are loaded lazily (only on first `require`) instead of
/// eagerly at startup.
pub fn standard_env() -> Arc<GlobalEnv> {
    let globals = cljrs_eval::standard_env_minimal();
    register(&globals);
    globals
}

/// Like [`standard_env()`] but also sets user source paths for `require`.
pub fn standard_env_with_paths(source_paths: Vec<std::path::PathBuf>) -> Arc<GlobalEnv> {
    let globals = standard_env();
    globals.set_source_paths(source_paths);
    globals
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cljrs_eval::{Env, EvalResult, eval};
    use cljrs_reader::Parser;
    use cljrs_value::Value;

    fn make_env() -> (Arc<GlobalEnv>, Env) {
        let globals = standard_env();
        let env = Env::new(globals.clone(), "user");
        (globals, env)
    }

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
        let (_, mut env) = make_env();
        // clojure.test is NOT pre-loaded in standard_env_minimal();
        // it should load lazily from the registry.
        run(
            "(require '[clojure.test :refer [is deftest run-tests]])",
            &mut env,
        )
        .unwrap();
        let v = run("(is (= 1 1))", &mut env).unwrap();
        assert_eq!(v, Value::Bool(true));
    }
}
