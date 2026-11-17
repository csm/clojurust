//! Issue #516: local `compare-and-set!` compares by identity, like Clojure's
//! `Atom.compareAndSet`, and never realizes lazy values under the atom's lock.
#![allow(clippy::result_large_err)]

use std::sync::mpsc;
use std::time::Duration;

use cljrs_reader::Parser;
use cljrs_runtime::{ExecutionMode, Runtime};

/// Evaluate `source` on a fresh runtime in a worker thread, failing the test
/// (instead of hanging) if it does not finish within the timeout.
fn eval_with_timeout(mode: ExecutionMode, source: &'static str) -> String {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let runtime = Runtime::builder().execution_mode(mode).build().unwrap();
            let mut env = runtime.env("user");
            let forms = Parser::new(source.into(), "<cas-identity>".into())
                .parse_all()
                .unwrap();
            let mut result = String::new();
            for form in forms {
                result = match cljrs_runtime::tiered::eval(&form, &mut env) {
                    Ok(v) => format!("{v}"),
                    Err(e) => format!("error: {e}"),
                };
            }
            let _ = tx.send(result);
        })
        .unwrap();
    rx.recv_timeout(Duration::from_secs(20))
        .unwrap_or_else(|_| panic!("{mode:?}: evaluation deadlocked or timed out"))
}

const MODES: [ExecutionMode; 2] = [ExecutionMode::TreeWalk, ExecutionMode::Tiered];

#[test]
fn equal_but_distinct_expected_values_do_not_match() {
    for mode in MODES {
        let got = eval_with_timeout(
            mode,
            r#"
            (let [x (vector 1 2)
                  y (vector 1 2)
                  a (atom x)
                  s1 (str "ab" "c")
                  s2 (str "a" "bc")
                  b (atom s1)
                  m1 (hash-map :k 1)
                  c (atom m1)]
              [(identical? x y)
               (compare-and-set! a y :changed)
               (apply compare-and-set! [a y :changed])
               @a
               (compare-and-set! b s2 :changed)
               @b
               (compare-and-set! c (hash-map :k 1) :changed)
               @c])
            "#,
        );
        assert_eq!(
            got, r#"[false false false [1 2] false "abc" false {:k 1}]"#,
            "{mode:?}"
        );
    }
}

#[test]
fn exact_retained_snapshot_matches() {
    for mode in MODES {
        let got = eval_with_timeout(
            mode,
            r#"
            (let [a (atom (vector 1 2))
                  snap @a
                  b (atom "abc")
                  s @b
                  c (atom {:k 1})
                  m @c]
              [(compare-and-set! a snap :v)
               @a
               (apply compare-and-set! [b s :s])
               @b
               (compare-and-set! c m :m)
               @c
               (compare-and-set! (atom 7) 7 8)
               (compare-and-set! (atom :kw) :kw 8)
               (compare-and-set! (atom nil) nil 8)])
            "#,
        );
        assert_eq!(got, "[true :v true :s true :m true true true]", "{mode:?}");
    }
}

#[test]
fn lazy_value_reentering_the_atom_does_not_deadlock_or_realize() {
    for mode in MODES {
        let got = eval_with_timeout(
            mode,
            r#"
            (let [a (atom nil)
                  realized (atom false)
                  lz (lazy-seq (reset! realized true) (reset! a 0) nil)]
              (reset! a lz)
              [(compare-and-set! a [] 1)
               (apply compare-and-set! [a () 1])
               (identical? lz @a)
               @realized])
            "#,
        );
        assert_eq!(got, "[false false true false]", "{mode:?}");
    }
}

#[test]
fn lazy_value_reading_the_atom_as_snapshot_commits() {
    for mode in MODES {
        let got = eval_with_timeout(
            mode,
            r#"
            (let [a (atom nil)
                  lz (lazy-seq (cons @a nil))]
              (reset! a lz)
              [(compare-and-set! a lz :done) @a])
            "#,
        );
        assert_eq!(got, "[true :done]", "{mode:?}");
    }
}
