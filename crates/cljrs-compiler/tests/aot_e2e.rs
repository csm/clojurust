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
