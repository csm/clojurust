//! End-to-end AOT compilation tests.
//!
//! Each test writes a `.cljrs` source file, compiles it to a binary via
//! `compile_file`, runs the binary, and asserts on stdout.

use std::process::Command;
use std::sync::Mutex;

/// Serialize all AOT tests — each invokes `cargo build` in a harness project,
/// and concurrent cargo processes fight over the crates.io index lock.
static AOT_LOCK: Mutex<()> = Mutex::new(());

/// Compile a `.cljrs` source string to a binary, run it, and return stdout.
fn compile_and_run(name: &str, source: &str) -> String {
    let _guard = AOT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let dir = std::env::temp_dir().join("cljrs_aot_tests");
    std::fs::create_dir_all(&dir).unwrap();

    let src_path = dir.join(format!("{name}.cljrs"));
    let bin_path = dir.join(format!("{name}_bin"));

    std::fs::write(&src_path, source).unwrap();

    // compile_file needs a large stack for Clojure eval
    let result = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn({
            let src = src_path.clone();
            let bin = bin_path.clone();
            move || cljrs_compiler::aot::compile_file(&src, &bin, &[])
        })
        .unwrap()
        .join()
        .unwrap();

    result.unwrap_or_else(|e| panic!("compilation failed for {name}: {e:?}"));

    let output = Command::new(&bin_path)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {name} binary: {e}"));

    assert!(
        output.status.success(),
        "{name} binary exited with {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).unwrap()
}

/// Assert that compiled output matches expected lines.
fn assert_output(name: &str, source: &str, expected: &str) {
    let actual = compile_and_run(name, source);
    assert_eq!(
        actual.trim(),
        expected.trim(),
        "\n--- {name} output mismatch ---\nexpected:\n{expected}\nactual:\n{actual}"
    );
}

// ── Constants & arithmetic ─────────────────────────────────────────────────

#[test]
fn test_constants() {
    assert_output(
        "constants",
        r#"
(println 42)
(println 3.14)
(println true)
(println false)
(println nil)
(println "hello")
(println :foo)
(println \a)
"#,
        "42\n3.14\ntrue\nfalse\nnil\nhello\n:foo\na",
    );
}

#[test]
fn test_arithmetic() {
    assert_output(
        "arithmetic",
        r#"
(println (+ 2 3))
(println (- 10 4))
(println (* 6 7))
(println (/ 15 3))
"#,
        "5\n6\n42\n5",
    );
}

#[test]
fn test_comparison() {
    assert_output(
        "comparison",
        r#"
(println (= 1 1))
(println (= 1 2))
(println (< 1 2))
(println (> 3 2))
(println (<= 5 5))
(println (>= 4 5))
"#,
        "true\nfalse\ntrue\ntrue\ntrue\nfalse",
    );
}

// ── Control flow ───────────────────────────────────────────────────────────

#[test]
fn test_if_expression() {
    assert_output(
        "if_expr",
        r#"
(println (if true "yes" "no"))
(println (if false "yes" "no"))
(println (if nil "yes" "no"))
(println (if 0 "truthy" "falsy"))
"#,
        "yes\nno\nno\ntruthy",
    );
}

#[test]
fn test_when_macro() {
    assert_output(
        "when_macro",
        r#"
(defn check [x]
  (when (> x 0)
    (println "positive")))
(check 5)
(check -1)
(println "done")
"#,
        "positive\ndone",
    );
}

#[test]
fn test_cond_macro() {
    assert_output(
        "cond_macro",
        r#"
(defn classify [x]
  (cond
    (< x 0) "negative"
    (= x 0) "zero"
    :else "positive"))
(println (classify -5))
(println (classify 0))
(println (classify 10))
"#,
        "negative\nzero\npositive",
    );
}

#[test]
fn test_and_or() {
    assert_output(
        "and_or",
        r#"
(println (and true true))
(println (and true false))
(println (and nil 42))
(println (or false 42))
(println (or nil false "default"))
(println (or false nil))
"#,
        "true\nfalse\nnil\n42\ndefault\nnil",
    );
}

// ── Let & do ───────────────────────────────────────────────────────────────

#[test]
fn test_let_binding() {
    assert_output(
        "let_binding",
        r#"
(let [x 10
      y (+ x 5)
      z (* y 2)]
  (println z))
"#,
        "30",
    );
}

#[test]
fn test_nested_let() {
    assert_output(
        "nested_let",
        r#"
(let [x 1]
  (let [x 2
        y (+ x 10)]
    (println y))
  (println x))
"#,
        "12\n1",
    );
}

#[test]
fn test_do_block() {
    assert_output(
        "do_block",
        r#"
(do
  (println "a")
  (println "b")
  (println "c"))
"#,
        "a\nb\nc",
    );
}

// ── Loop & recur ───────────────────────────────────────────────────────────

#[test]
fn test_loop_recur() {
    assert_output(
        "loop_recur",
        r#"
(println
  (loop [i 0 acc 0]
    (if (= i 10)
      acc
      (recur (+ i 1) (+ acc i)))))
"#,
        "45",
    );
}

#[test]
fn test_loop_recur_factorial() {
    assert_output(
        "loop_factorial",
        r#"
(defn factorial [n]
  (loop [i n acc 1]
    (if (<= i 1)
      acc
      (recur (- i 1) (* acc i)))))
(println (factorial 10))
"#,
        "3628800",
    );
}

// ── Functions & closures ───────────────────────────────────────────────────

#[test]
fn test_defn_call() {
    assert_output(
        "defn_call",
        r#"
(defn greet [name]
  (str "Hello, " name "!"))
(println (greet "world"))
"#,
        "Hello, world!",
    );
}

#[test]
fn test_closure_capture() {
    assert_output(
        "closure_capture",
        r#"
(defn make-adder [n]
  (fn [x] (+ x n)))
(let [add5 (make-adder 5)]
  (println (add5 10))
  (println (add5 20)))
"#,
        "15\n25",
    );
}

#[test]
fn test_higher_order_function() {
    assert_output(
        "higher_order",
        r#"
(defn apply-twice [f x]
  (f (f x)))
(println (apply-twice (fn [x] (+ x 1)) 0))
(println (apply-twice (fn [x] (* x 2)) 3))
"#,
        "2\n12",
    );
}

#[test]
fn test_recursive_defn() {
    assert_output(
        "recursive_defn",
        r#"
(defn fib [n]
  (if (<= n 1)
    n
    (+ (fib (- n 1)) (fib (- n 2)))))
(println (fib 0))
(println (fib 1))
(println (fib 10))
"#,
        "0\n1\n55",
    );
}

// ── Collections ────────────────────────────────────────────────────────────

#[test]
fn test_vector_ops() {
    assert_output(
        "vector_ops",
        r#"
(let [v [1 2 3]]
  (println (count v))
  (println (nth v 0))
  (println (nth v 2))
  (println (conj v 4)))
"#,
        "3\n1\n3\n[1 2 3 4]",
    );
}

#[test]
fn test_map_ops() {
    assert_output(
        "map_ops",
        r#"
(let [m {:a 1 :b 2}]
  (println (get m :a))
  (println (get m :c))
  (println (count m)))
"#,
        "1\nnil\n2",
    );
}

#[test]
fn test_collection_literals() {
    assert_output(
        "collection_literals",
        r#"
(println (vector 1 2 3))
(println (list 1 2 3))
(println (count (hash-set 1 2 3 2 1)))
"#,
        "[1 2 3]\n(1 2 3)\n3",
    );
}

// ── Destructuring ──────────────────────────────────────────────────────────

#[test]
fn test_vector_destructuring() {
    assert_output(
        "vec_destructure",
        r#"
(let [[a b c] [10 20 30]]
  (println a)
  (println b)
  (println c))
"#,
        "10\n20\n30",
    );
}

#[test]
fn test_map_destructuring() {
    assert_output(
        "map_destructure",
        r#"
(let [{:keys [x y]} {:x 1 :y 2}]
  (println (+ x y)))
"#,
        "3",
    );
}

#[test]
fn test_fn_param_destructuring() {
    assert_output(
        "fn_destructure",
        r#"
(defn sum-pair [[a b]]
  (+ a b))
(println (sum-pair [3 7]))
"#,
        "10",
    );
}

// ── Threading macros ───────────────────────────────────────────────────────

#[test]
fn test_thread_first() {
    assert_output(
        "thread_first",
        r#"
(println (-> 1 (+ 2) (* 3)))
"#,
        "9",
    );
}

#[test]
fn test_thread_last() {
    assert_output(
        "thread_last",
        r#"
(println (->> [1 2 3 4 5]
              (filter (fn [x] (> x 2)))
              (map (fn [x] (* x 10)))
              (reduce +)))
"#,
        "120",
    );
}

// ── Try/catch ──────────────────────────────────────────────────────────────

#[test]
fn test_try_catch() {
    assert_output(
        "try_catch",
        r#"
(println
  (try
    (throw "boom")
    (catch Exception e
      (str "caught: " e))))
"#,
        "caught: boom",
    );
}

#[test]
fn test_try_no_exception() {
    assert_output(
        "try_no_exception",
        r#"
(println
  (try
    42
    (catch Exception e
      "error")))
"#,
        "42",
    );
}

// ── Dynamic vars & binding ─────────────────────────────────────────────────

#[test]
fn test_binding() {
    assert_output(
        "binding",
        r#"
(def ^:dynamic *x* 10)
(defn show [] (println *x*))
(show)
(binding [*x* 42]
  (show))
(show)
"#,
        "10\n42\n10",
    );
}

#[test]
fn test_set_bang() {
    assert_output(
        "set_bang",
        r#"
(def ^:dynamic *counter* 0)
(binding [*counter* 0]
  (set! *counter* 5)
  (println *counter*))
(println *counter*)
"#,
        "5\n0",
    );
}

// ── Letfn ──────────────────────────────────────────────────────────────────

#[test]
fn test_letfn_self_recursion() {
    assert_output(
        "letfn_self",
        r#"
(defn test-fact []
  (letfn [(fact [n]
            (if (= n 0) 1 (* n (fact (- n 1)))))]
    (println (fact 5))))
(test-fact)
"#,
        "120",
    );
}

#[test]
fn test_letfn_mutual_recursion() {
    assert_output(
        "letfn_mutual",
        r#"
(letfn [(my-even? [n]
          (if (= n 0) true (my-odd? (- n 1))))
        (my-odd? [n]
          (if (= n 0) false (my-even? (- n 1))))]
  (println (my-even? 10))
  (println (my-odd? 7)))
"#,
        "true\ntrue",
    );
}

#[test]
fn test_letfn_with_captures() {
    assert_output(
        "letfn_capture",
        r#"
(defn make-counter [start]
  (letfn [(step [n] (+ n start))]
    (println (step 0))
    (println (step 10))))
(make-counter 5)
"#,
        "5\n15",
    );
}

// ── Quote ──────────────────────────────────────────────────────────────────

#[test]
fn test_quote() {
    assert_output(
        "quote",
        r#"
(println (quote hello))
(println '(1 2 3))
(println ':kw)
"#,
        "hello\n(1 2 3)\n:kw",
    );
}

// ── String operations ──────────────────────────────────────────────────────

#[test]
fn test_str_concat() {
    assert_output(
        "str_concat",
        r#"
(println (str "a" "b" "c"))
(println (str 1 "-" 2 "-" 3))
(println (str nil))
"#,
        "abc\n1-2-3\nnil",
    );
}

// ── Multi-arity functions ──────────────────────────────────────────────────

#[test]
fn test_multi_arity_fn() {
    assert_output(
        "multi_arity",
        r#"
(defn greet
  ([name] (greet "Hello" name))
  ([greeting name] (str greeting ", " name "!")))
(println (greet "world"))
(println (greet "Hi" "there"))
"#,
        "Hello, world!\nHi, there!",
    );
}

// ── Nested macros ──────────────────────────────────────────────────────────

#[test]
fn test_nested_macros() {
    assert_output(
        "nested_macros",
        r#"
(defn process [x]
  (when (> x 0)
    (cond
      (> x 100) (println "big")
      (> x 10) (println "medium")
      :else (println "small"))))
(process 200)
(process 50)
(process 3)
(process -1)
"#,
        "big\nmedium\nsmall",
    );
}

// ── If-let macro ───────────────────────────────────────────────────────────

#[test]
fn test_if_let() {
    assert_output(
        "if_let",
        r#"
(defn find-positive [x]
  (if-let [v (when (> x 0) x)]
    (str "found: " v)
    "not found"))
(println (find-positive 5))
(println (find-positive -3))
"#,
        "found: 5\nnot found",
    );
}

// ── Def with initial value ─────────────────────────────────────────────────

#[test]
fn test_def_and_use() {
    assert_output(
        "def_use",
        r#"
(def pi 3.14159)
(def tau (* 2 pi))
(println tau)
"#,
        "6.28318",
    );
}

// ── Sequences ──────────────────────────────────────────────────────────────

#[test]
fn test_first_rest() {
    assert_output(
        "first_rest",
        r#"
(let [xs [10 20 30]]
  (println (first xs))
  (println (rest xs))
  (println (first (rest xs))))
"#,
        "10\n(20 30)\n20",
    );
}

// ── Protocols & multimethods ──────────────────────────────────────────────

#[test]
fn test_protocol_basic() {
    assert_output(
        "protocol_basic",
        r#"
(defprotocol Greet
  (greet [this]))
(extend-type Long Greet
  (greet [this] (str "Hello, number " this)))
(extend-type String Greet
  (greet [this] (str "Hello, " this "!")))
(println (greet 42))
(println (greet "world"))
"#,
        "Hello, number 42\nHello, world!",
    );
}

#[test]
fn test_protocol_multi_method() {
    assert_output(
        "protocol_multi",
        r#"
(defprotocol Shape
  (area [this])
  (describe [this]))
(extend-type Map Shape
  (area [this] (* (:width this) (:height this)))
  (describe [this] (str (:name this) " area=" (area this))))
(let [rect {:name "rect" :width 3 :height 4}]
  (println (describe rect)))
"#,
        "rect area=12",
    );
}

#[test]
fn test_defmulti_basic() {
    assert_output(
        "defmulti_basic",
        r#"
(defmulti animal-sound :type)
(defmethod animal-sound "dog" [a] (str (:name a) " says woof"))
(defmethod animal-sound "cat" [a] (str (:name a) " says meow"))
(defmethod animal-sound :default [a] (str (:name a) " says ???"))
(println (animal-sound {:type "dog" :name "Rex"}))
(println (animal-sound {:type "cat" :name "Whiskers"}))
(println (animal-sound {:type "fish" :name "Nemo"}))
"#,
        "Rex says woof\nWhiskers says meow\nNemo says ???",
    );
}

#[test]
fn test_protocol_in_defn() {
    assert_output(
        "protocol_in_defn",
        r#"
(defprotocol Describable
  (describe [this]))
(extend-type Long Describable
  (describe [this] (str "num:" this)))
(extend-type String Describable
  (describe [this] (str "str:" this)))
(defn describe-all [items]
  (loop [xs items]
    (when (seq xs)
      (println (describe (first xs)))
      (recur (rest xs)))))
(describe-all [1 "hello" 42 "world"])
"#,
        "num:1\nstr:hello\nnum:42\nstr:world",
    );
}

#[test]
fn test_defrecord() {
    assert_output(
        "defrecord",
        r#"
(defrecord Point [x y])
(let [p (->Point 3 4)]
  (println (:x p))
  (println (:y p)))
"#,
        "3\n4",
    );
}

#[test]
fn test_defrecord_with_protocol() {
    assert_output(
        "defrecord_proto",
        r#"
(defprotocol HasArea
  (area [this]))
(defrecord Circle [radius]
  HasArea
  (area [this] (* 3 (* (:radius this) (:radius this)))))
(defrecord Rect [w h]
  HasArea
  (area [this] (* (:w this) (:h this))))
(println (area (->Circle 5)))
(println (area (->Rect 3 4)))
"#,
        "75\n12",
    );
}

#[test]
fn test_protocol_with_defn_impl() {
    // Protocol function called from a compiled defn, with the impl also
    // calling other compiled functions
    assert_output(
        "protocol_defn_impl",
        r#"
(defprotocol Measurable
  (measure [this]))
(defn double [x] (* x 2))
(extend-type Vector Measurable
  (measure [this] (double (count this))))
(extend-type Map Measurable
  (measure [this] (+ (count this) 100)))
(defn report [thing]
  (str "measure=" (measure thing)))
(println (report [1 2 3]))
(println (report {:a 1 :b 2}))
"#,
        "measure=6\nmeasure=102",
    );
}

#[test]
fn test_multimethod_keyword_dispatch() {
    assert_output(
        "multimethod_keyword",
        r#"
(defmulti process :action)
(defmethod process :add [m] (+ (:x m) (:y m)))
(defmethod process :mul [m] (* (:x m) (:y m)))
(defmethod process :default [m] 0)
(println (process {:action :add :x 3 :y 4}))
(println (process {:action :mul :x 5 :y 6}))
(println (process {:action :sub :x 1 :y 2}))
"#,
        "7\n30\n0",
    );
}

#[test]
fn test_multimethod_type_dispatch() {
    // type returns symbols, so dispatch values must be symbols too
    assert_output(
        "multimethod_type",
        r#"
(defmulti format-val type)
(defmethod format-val 'Long [v] (str "int:" v))
(defmethod format-val 'String [v] (str "str:" v))
(defmethod format-val 'Vector [v] (str "vec:" (count v)))
(defmethod format-val :default [v] "other")
(println (format-val 42))
(println (format-val "hi"))
(println (format-val [1 2 3]))
(println (format-val nil))
"#,
        "int:42\nstr:hi\nvec:3\nother",
    );
}

// ── Variadic functions ────────────────────────────────────────────────────

#[test]
fn test_variadic_basic() {
    assert_output(
        "variadic_basic",
        r#"
(defn sum [& nums]
  (loop [xs nums acc 0]
    (if (seq xs)
      (recur (rest xs) (+ acc (first xs)))
      acc)))
(println (sum 1 2 3 4 5))
(println (sum))
(println (sum 10))
"#,
        "15\n0\n10",
    );
}

#[test]
fn test_variadic_with_fixed() {
    assert_output(
        "variadic_fixed",
        r#"
(defn log [level & msgs]
  (println (str "[" level "] " (first msgs))))
(log "INFO" "starting up")
(log "ERR" "something broke")
"#,
        "[INFO] starting up\n[ERR] something broke",
    );
}

#[test]
fn test_variadic_multi_arity() {
    assert_output(
        "variadic_multi",
        r#"
(defn f
  ([x] (str "one:" x))
  ([x y] (str "two:" x "," y))
  ([x y & more] (str "many:" x "," y "+" (count more))))
(println (f 1))
(println (f 1 2))
(println (f 1 2 3 4 5))
"#,
        "one:1\ntwo:1,2\nmany:1,2+3",
    );
}

// ── Multi-file compilation ─────────────────────────────────────────────────

/// Compile a multi-file program. `deps` is a list of (filename, source) pairs
/// for dependency files. The main file requires them via `ns`/`require`.
fn compile_and_run_multi(name: &str, main_source: &str, deps: &[(&str, &str)]) -> String {
    let _guard = AOT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let dir = std::env::temp_dir().join("cljrs_aot_tests");
    std::fs::create_dir_all(&dir).unwrap();

    // Write dependency files into a "src" subdirectory.
    let src_dir = dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    for (filename, source) in deps {
        // Create subdirectories if needed (e.g., "mylib/utils.cljrs").
        let dep_path = src_dir.join(filename);
        if let Some(parent) = dep_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&dep_path, source).unwrap();
    }

    let src_path = dir.join(format!("{name}.cljrs"));
    let bin_path = dir.join(format!("{name}_bin"));
    std::fs::write(&src_path, main_source).unwrap();

    let result = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn({
            let src = src_path.clone();
            let bin = bin_path.clone();
            let src_dir = src_dir.clone();
            move || cljrs_compiler::aot::compile_file(&src, &bin, &[src_dir])
        })
        .unwrap()
        .join()
        .unwrap();

    result.unwrap_or_else(|e| panic!("compilation failed for {name}: {e:?}"));

    let output = std::process::Command::new(&bin_path)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {name} binary: {e}"));

    assert!(
        output.status.success(),
        "{name} binary exited with {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).unwrap()
}

fn assert_output_multi(name: &str, main_source: &str, deps: &[(&str, &str)], expected: &str) {
    let actual = compile_and_run_multi(name, main_source, deps);
    assert_eq!(
        actual.trim(),
        expected.trim(),
        "\n--- {name} output mismatch ---\nexpected:\n{expected}\nactual:\n{actual}"
    );
}

#[test]
fn test_multi_file_basic() {
    assert_output_multi(
        "multi_basic",
        r#"
(ns my-app
  (:require [my-lib :as lib]))
(println (lib/greet "world"))
"#,
        &[(
            "my_lib.cljrs",
            r#"
(ns my-lib)
(defn greet [name]
  (str "Hello, " name "!"))
"#,
        )],
        "Hello, world!",
    );
}

#[test]
fn test_multi_file_transitive() {
    assert_output_multi(
        "multi_transitive",
        r#"
(ns my-app
  (:require [services :as svc]))
(println (svc/process "test"))
"#,
        &[
            (
                "utils.cljrs",
                r#"
(ns utils)
(defn wrap [s]
  (str "[" s "]"))
"#,
            ),
            (
                "services.cljrs",
                r#"
(ns services
  (:require [utils :as u]))
(defn process [x]
  (str "result=" (u/wrap x)))
"#,
            ),
        ],
        "result=[test]",
    );
}

#[test]
fn test_multi_file_refer() {
    assert_output_multi(
        "multi_refer",
        r#"
(ns my-app
  (:require [math-lib :refer [square double-it]]))
(println (square 5))
(println (double-it 7))
"#,
        &[(
            "math_lib.cljrs",
            r#"
(ns math-lib)
(defn square [x] (* x x))
(defn double-it [x] (* 2 x))
"#,
        )],
        "25\n14",
    );
}

// ── Complex integration test ───────────────────────────────────────────────

#[test]
fn test_fizzbuzz() {
    assert_output(
        "fizzbuzz",
        r#"
(defn fizzbuzz [n]
  (loop [i 1]
    (when (<= i n)
      (cond
        (= 0 (rem i 15)) (println "FizzBuzz")
        (= 0 (rem i 3)) (println "Fizz")
        (= 0 (rem i 5)) (println "Buzz")
        :else (println i))
      (recur (+ i 1)))))
(fizzbuzz 15)
"#,
        "1\n2\nFizz\n4\nBuzz\nFizz\n7\n8\nFizz\nBuzz\n11\nFizz\n13\n14\nFizzBuzz",
    );
}

// ── Apply tests ─────────────────────────────────────────────────────────────

#[test]
fn test_apply_basic() {
    assert_output(
        "apply_basic",
        r#"(println (apply + [1 2 3]))"#,
        "6",
    );
}

#[test]
fn test_apply_multi_arg() {
    assert_output(
        "apply_multi_arg",
        r#"(println (apply + 1 2 [3 4]))"#,
        "10",
    );
}

// ── HOF tests ───────────────────────────────────────────────────────────────

#[test]
fn test_map_basic() {
    assert_output(
        "map_basic",
        r#"
(defn double [x] (* x 2))
(println (map double [1 2 3]))
"#,
        "(2 4 6)",
    );
}

#[test]
fn test_filter_basic() {
    assert_output(
        "filter_basic",
        r#"
(defn even? [x] (= 0 (rem x 2)))
(println (filter even? [1 2 3 4 5 6]))
"#,
        "(2 4 6)",
    );
}

#[test]
fn test_reduce_2arg() {
    assert_output(
        "reduce_2arg",
        r#"(println (reduce + [1 2 3 4 5]))"#,
        "15",
    );
}

#[test]
fn test_reduce_3arg() {
    assert_output(
        "reduce_3arg",
        r#"(println (reduce + 10 [1 2 3 4 5]))"#,
        "25",
    );
}

#[test]
fn test_mapv_basic() {
    assert_output(
        "mapv_basic",
        r#"
(defn inc [x] (+ x 1))
(println (mapv inc [10 20 30]))
"#,
        "[11 21 31]",
    );
}

#[test]
fn test_filterv_basic() {
    assert_output(
        "filterv_basic",
        r#"
(defn pos? [x] (> x 0))
(println (filterv pos? [-1 2 -3 4]))
"#,
        "[2 4]",
    );
}

#[test]
fn test_some_basic() {
    assert_output(
        "some_basic",
        r#"
(defn even? [x] (if (= 0 (rem x 2)) x nil))
(println (some even? [1 3 4 5]))
"#,
        "4",
    );
}

#[test]
fn test_every_basic() {
    assert_output(
        "every_basic",
        r#"
(defn pos? [x] (> x 0))
(println (every? pos? [1 2 3]))
(println (every? pos? [1 -2 3]))
"#,
        "true\nfalse",
    );
}

#[test]
fn test_into_basic() {
    assert_output(
        "into_basic",
        r#"(println (into [] (list 1 2 3)))"#,
        "[1 2 3]",
    );
}

// ── Inline expansion tests ──────────────────────────────────────────────────

#[test]
fn test_inc_dec() {
    assert_output(
        "inc_dec",
        r#"(println (inc 41)) (println (dec 43))"#,
        "42\n42",
    );
}

#[test]
fn test_not() {
    assert_output(
        "not_fn",
        r#"(println (not true)) (println (not false)) (println (not nil))"#,
        "false\ntrue\ntrue",
    );
}

#[test]
fn test_not_eq() {
    assert_output(
        "not_eq",
        r#"(println (not= 1 2)) (println (not= 1 1))"#,
        "true\nfalse",
    );
}

#[test]
fn test_predicates() {
    assert_output(
        "predicates",
        r#"
(println (zero? 0))
(println (zero? 1))
(println (pos? 5))
(println (neg? -3))
(println (even? 4))
(println (odd? 3))
(println (empty? []))
(println (empty? [1]))
"#,
        "true\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse",
    );
}

#[test]
fn test_max_min() {
    assert_output(
        "max_min",
        r#"(println (max 3 7)) (println (min 3 7))"#,
        "7\n3",
    );
}

// ── Sequence operation tests ────────────────────────────────────────────────

#[test]
fn test_range() {
    assert_output(
        "range_ops",
        r#"
(println (into [] (range 5)))
(println (into [] (range 2 5)))
(println (into [] (range 0 10 3)))
"#,
        "[0 1 2 3 4]\n[2 3 4]\n[0 3 6 9]",
    );
}

#[test]
fn test_take_drop() {
    assert_output(
        "take_drop",
        r#"
(println (into [] (take 3 [1 2 3 4 5])))
(println (into [] (drop 2 [1 2 3 4 5])))
"#,
        "[1 2 3]\n[3 4 5]",
    );
}

#[test]
fn test_reverse() {
    assert_output(
        "reverse_op",
        r#"(println (into [] (reverse [1 2 3 4 5])))"#,
        "[5 4 3 2 1]",
    );
}

#[test]
fn test_sort() {
    assert_output(
        "sort_op",
        r#"(println (into [] (sort [3 1 4 1 5 9 2 6])))"#,
        "[1 1 2 3 4 5 6 9]",
    );
}

#[test]
fn test_keys_vals() {
    assert_output(
        "keys_vals",
        r#"
(println (sort (keys {:a 1 :b 2 :c 3})))
(println (sort (vals {:a 1 :b 2 :c 3})))
"#,
        "(:a :b :c)\n(1 2 3)",
    );
}

#[test]
fn test_concat() {
    assert_output(
        "concat_op",
        r#"(println (into [] (concat [1 2] [3 4] [5])))"#,
        "[1 2 3 4 5]",
    );
}

// ── Type predicate tests ────────────────────────────────────────────────────

#[test]
fn test_type_predicates() {
    assert_output(
        "type_preds",
        r#"
(println (number? 42))
(println (number? "hi"))
(println (string? "hi"))
(println (keyword? :foo))
(println (symbol? 'bar))
(println (int? 42))
(println (int? 3.14))
"#,
        "true\nfalse\ntrue\ntrue\ntrue\ntrue\nfalse",
    );
}

// ── Atom constructor test ───────────────────────────────────────────────────

#[test]
fn test_atom_constructor() {
    assert_output(
        "atom_ctor",
        r#"
(def a (atom 0))
(swap! a inc)
(swap! a inc)
(println @a)
"#,
        "2",
    );
}

// ── Keyword-as-function tests ───────────────────────────────────────────────

#[test]
fn test_keyword_as_function() {
    assert_output(
        "kw_as_fn",
        r#"
(def m {:name "Alice" :age 30})
(println (:name m))
(println (:age m))
(println (:missing m))
"#,
        "Alice\n30\nnil",
    );
}

#[test]
fn test_keyword_as_function_nested() {
    assert_output(
        "kw_as_fn_nested",
        r#"
(def data [{:name "Alice"} {:name "Bob"} {:name "Carol"}])
(println (map :name data))
"#,
        "(Alice Bob Carol)",
    );
}

// ── Lazy sequence tests ─────────────────────────────────────────────────────

#[test]
fn test_lazy_seq_basic() {
    assert_output(
        "lazy_seq_basic",
        r#"
(defn my-range [n]
  (letfn [(go [i]
            (lazy-seq
              (when (< i n)
                (cons i (go (+ i 1))))))]
    (go 0)))
(println (into [] (my-range 5)))
"#,
        "[0 1 2 3 4]",
    );
}

#[test]
fn test_lazy_seq_infinite() {
    assert_output(
        "lazy_seq_infinite",
        r#"
(defn naturals []
  (letfn [(go [n]
            (lazy-seq (cons n (go (+ n 1)))))]
    (go 0)))
(println (into [] (take 5 (naturals))))
"#,
        "[0 1 2 3 4]",
    );
}

#[test]
fn test_lazy_seq_fibonacci() {
    assert_output(
        "lazy_seq_fib",
        r#"
(defn fibs []
  (letfn [(go [a b]
            (lazy-seq (cons a (go b (+ a b)))))]
    (go 0 1)))
(println (into [] (take 10 (fibs))))
"#,
        "[0 1 1 2 3 5 8 13 21 34]",
    );
}

// ── More HOF tests ──────────────────────────────────────────────────────────

#[test]
fn test_group_by() {
    assert_output(
        "group_by",
        r#"
(defn even? [x] (= 0 (rem x 2)))
(def result (group-by even? [1 2 3 4 5 6]))
(println (get result true))
(println (get result false))
"#,
        "[2 4 6]\n[1 3 5]",
    );
}

#[test]
fn test_frequencies() {
    assert_output(
        "frequencies",
        r#"(println (sort (frequencies [:a :b :a :c :b :a])))"#,
        "([:a 3] [:b 2] [:c 1])",
    );
}

#[test]
fn test_keep() {
    assert_output(
        "keep_fn",
        r#"
(defn pos-or-nil [x] (if (> x 0) x nil))
(println (into [] (keep pos-or-nil [-2 -1 0 1 2 3])))
"#,
        "[1 2 3]",
    );
}

#[test]
fn test_remove() {
    assert_output(
        "remove_fn",
        r#"
(defn even? [x] (= 0 (rem x 2)))
(println (into [] (remove even? [1 2 3 4 5 6])))
"#,
        "[1 3 5]",
    );
}

#[test]
fn test_map_indexed() {
    assert_output(
        "map_indexed_fn",
        r#"
(defn pair [i x] (vector i x))
(println (into [] (map-indexed pair [:a :b :c])))
"#,
        "[[0 :a] [1 :b] [2 :c]]",
    );
}

#[test]
fn test_partition() {
    assert_output(
        "partition_fn",
        r#"(println (into [] (partition 2 [1 2 3 4 5 6])))"#,
        "[(1 2) (3 4) (5 6)]",
    );
}

#[test]
fn test_zipmap() {
    assert_output(
        "zipmap_fn",
        r#"(println (sort (zipmap [:a :b :c] [1 2 3])))"#,
        "([:a 1] [:b 2] [:c 3])",
    );
}

#[test]
fn test_comp() {
    assert_output(
        "comp_fn",
        r#"
(defn double [x] (* x 2))
(def double-inc (comp inc double))
(println (double-inc 5))
"#,
        "11",
    );
}

#[test]
fn test_partial() {
    assert_output(
        "partial_fn",
        r#"
(def add5 (partial + 5))
(println (add5 10))
(println (add5 20))
"#,
        "15\n25",
    );
}

#[test]
fn test_complement() {
    assert_output(
        "complement_fn",
        r#"
(defn even? [x] (= 0 (rem x 2)))
(def odd? (complement even?))
(println (into [] (filter odd? [1 2 3 4 5 6])))
"#,
        "[1 3 5]",
    );
}

#[test]
fn test_juxt() {
    assert_output(
        "juxt_fn",
        r#"
(def stats (juxt count first last))
(println (stats [10 20 30 40]))
"#,
        "[4 10 40]",
    );
}

// ── Direct inter-function calls ─────────────────────────────────────────────

#[test]
fn test_direct_call_basic() {
    assert_output(
        "direct_call_basic",
        r#"
(defn double [x] (* x 2))
(defn quadruple [x] (double (double x)))
(println (quadruple 5))
"#,
        "20",
    );
}

#[test]
fn test_direct_call_mutual() {
    assert_output(
        "direct_call_mutual",
        r#"
(defn add1 [x] (+ x 1))
(defn add2 [x] (add1 (add1 x)))
(defn add4 [x] (add2 (add2 x)))
(println (add4 10))
"#,
        "14",
    );
}

#[test]
fn test_direct_call_multi_arity() {
    assert_output(
        "direct_call_multi_arity",
        r#"
(defn greet
  ([name] (greet "Hello" name))
  ([greeting name] (str greeting ", " name "!")))
(println (greet "World"))
(println (greet "Hi" "Alice"))
"#,
        "Hello, World!\nHi, Alice!",
    );
}

#[test]
fn test_direct_call_with_variadic_fallback() {
    // Variadic calls should still go through rt_call (not direct call).
    assert_output(
        "direct_call_variadic_fallback",
        r#"
(defn fixed [x y] (+ x y))
(defn vararg [& xs] (reduce + 0 xs))
(println (fixed 3 4))
(println (vararg 1 2 3))
"#,
        "7\n6",
    );
}
