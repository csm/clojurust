//! Example: exposing a Rust struct to clojurust via NativeObject + protocols.
//!
//! Demonstrates:
//! - Defining a Rust struct that implements `NativeObject`
//! - Wrapping it as a `Value::NativeObject` and registering native functions
//! - Using protocol dispatch from Clojure to call methods on the Rust object
//! - Type marshalling with `FromValue` / `IntoValue`

use std::sync::atomic::{AtomicI64, Ordering};

use cljrs_eval::{Env, eval};
use cljrs_gc::{GcPtr, MarkVisitor, Trace};
use cljrs_interop::{FromValue, IntoValue, NativeObject, gc_native_object, wrap_result};
use cljrs_stdlib::standard_env;
use cljrs_value::{Arity, NativeFn, Value, ValueError, ValueResult};

// ── Step 1: Define a Rust struct ─────────────────────────────────────────────

/// A simple thread-safe counter exposed to Clojure.
#[derive(Debug)]
struct Counter {
    name: String,
    value: AtomicI64,
}

impl Counter {
    fn new(name: &str, initial: i64) -> Self {
        Self {
            name: name.to_string(),
            value: AtomicI64::new(initial),
        }
    }

    fn get(&self) -> i64 {
        self.value.load(Ordering::SeqCst)
    }

    fn increment(&self, n: i64) -> i64 {
        self.value.fetch_add(n, Ordering::SeqCst) + n
    }

    fn reset(&self, n: i64) -> i64 {
        self.value.swap(n, Ordering::SeqCst)
    }
}

// ── Step 2: Implement NativeObject + Trace ───────────────────────────────────

impl NativeObject for Counter {
    fn type_tag(&self) -> &str {
        "Counter"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// Counter holds no GcPtr/Value fields, so Trace is a no-op.
impl Trace for Counter {
    fn trace(&self, _visitor: &mut MarkVisitor) {}
}

// ── Step 3: Define native functions that operate on Counter ──────────────────

/// Helper: downcast a Value to &Counter or return an error.
fn as_counter(v: &Value) -> ValueResult<&Counter> {
    match v {
        Value::NativeObject(obj) => {
            obj.get()
                .downcast_ref::<Counter>()
                .ok_or_else(|| ValueError::WrongType {
                    expected: "Counter",
                    got: obj.get().type_tag().to_string(),
                })
        }
        other => Err(ValueError::WrongType {
            expected: "Counter",
            got: other.type_name().to_string(),
        }),
    }
}

/// `(make-counter name initial-value)` → Counter
fn builtin_make_counter(args: &[Value]) -> ValueResult<Value> {
    let name = String::from_value(&args[0])?;
    let initial = i64::from_value(&args[1])?;
    Ok(Value::NativeObject(gc_native_object(Counter::new(
        &name, initial,
    ))))
}

/// `(counter-get c)` → Long
fn builtin_counter_get(args: &[Value]) -> ValueResult<Value> {
    let c = as_counter(&args[0])?;
    Ok(c.get().into_value())
}

/// `(counter-inc! c n)` → Long (new value)
fn builtin_counter_inc(args: &[Value]) -> ValueResult<Value> {
    let c = as_counter(&args[0])?;
    let n = i64::from_value(&args[1])?;
    Ok(c.increment(n).into_value())
}

/// `(counter-reset! c n)` → Long (old value)
fn builtin_counter_reset(args: &[Value]) -> ValueResult<Value> {
    let c = as_counter(&args[0])?;
    let n = i64::from_value(&args[1])?;
    Ok(c.reset(n).into_value())
}

/// `(counter-name c)` → String
fn builtin_counter_name(args: &[Value]) -> ValueResult<Value> {
    let c = as_counter(&args[0])?;
    wrap_result(Ok::<_, std::fmt::Error>(c.name.clone()))
}

// ── Step 4: Register everything and run Clojure code ─────────────────────────

fn register_counter_fns(env: &mut Env) {
    let globals = &env.globals;
    let ns = "counter";

    type NativeFnEntry = (&'static str, Arity, fn(&[Value]) -> ValueResult<Value>);
    let fns: &[NativeFnEntry] = &[
        ("make-counter", Arity::Fixed(2), builtin_make_counter),
        ("counter-get", Arity::Fixed(1), builtin_counter_get),
        ("counter-inc!", Arity::Fixed(2), builtin_counter_inc),
        ("counter-reset!", Arity::Fixed(2), builtin_counter_reset),
        ("counter-name", Arity::Fixed(1), builtin_counter_name),
    ];

    for &(name, ref arity, func) in fns {
        globals.intern(
            ns,
            std::sync::Arc::from(name),
            Value::NativeFunction(GcPtr::new(NativeFn {
                name: std::sync::Arc::from(format!("{ns}/{name}")),
                arity: arity.clone(),
                func,
            })),
        );
    }

    // Mark as loaded so `require` doesn't try to find it on the filesystem.
    globals.mark_loaded(ns);
}

fn main() {
    let globals = standard_env();
    let mut env = Env::new(globals, "user");

    // Register our Counter native functions in the "counter" namespace.
    register_counter_fns(&mut env);

    // Evaluate Clojure code that uses the Counter.
    let clojure_code = r#"
        ;; Require our native namespace
        (require '[counter :as c])

        ;; Create a counter
        (def my-counter (c/make-counter "hits" 0))
        (println "Created counter:" (c/counter-name my-counter))
        (println "Type:" (type my-counter))
        (println "native-object?:" (native-object? my-counter))
        (println "native-type:" (native-type my-counter))
        (println "instance? Counter:" (instance? "Counter" my-counter))

        ;; Use it
        (println "\nInitial value:" (c/counter-get my-counter))
        (c/counter-inc! my-counter 1)
        (c/counter-inc! my-counter 1)
        (c/counter-inc! my-counter 5)
        (println "After 3 increments (1+1+5):" (c/counter-get my-counter))

        ;; Reset
        (let [old (c/counter-reset! my-counter 100)]
          (println "Reset from" old "to 100"))
        (println "Current:" (c/counter-get my-counter))

        ;; Define a protocol and extend it to Counter
        (defprotocol ICounter
          (get-count [this])
          (inc-count! [this n]))

        (extend-type Counter ICounter
          (get-count [this] (c/counter-get this))
          (inc-count! [this n] (c/counter-inc! this n)))

        ;; Now use protocol dispatch
        (println "\nVia protocol:")
        (println "get-count:" (get-count my-counter))
        (inc-count! my-counter 10)
        (println "After inc-count! 10:" (get-count my-counter))

        ;; Demonstrate identity semantics
        (def c2 my-counter)
        (println "\nIdentity: (= my-counter c2) =>" (= my-counter c2))
        (println "Identity: (identical? my-counter c2) =>" (identical? my-counter c2))
        (def c3 (c/make-counter "other" 0))
        (println "Different: (= my-counter c3) =>" (= my-counter c3))

        (println "\nDone!")
    "#;

    let mut parser =
        cljrs_reader::Parser::new(clojure_code.to_string(), "<interop-example>".to_string());
    let forms = parser.parse_all().expect("parse error");
    for form in &forms {
        match eval(form, &mut env) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("Error: {e:?}");
                std::process::exit(1);
            }
        }
    }
}
