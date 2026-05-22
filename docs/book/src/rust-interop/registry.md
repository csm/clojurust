# Registry API

The `Registry` type (from `cljrs_interop`) is the handle passed to `cljrs_init`
for registering Rust functions as Clojure-visible values.

## `Registry` methods

```rust
// Register f under "my.ns/my-fn".
// Panics if `qualified` contains no '/'.
pub fn define(&self, qualified: &str, f: NativeFn);

// Register f into an explicit namespace with a plain name.
// Equivalent to define("ns/name", f).
pub fn define_in(&self, ns: &str, name: &str, f: NativeFn);

// Access the underlying GlobalEnv for advanced operations
// (registering builtin sources, setting namespace aliases, etc.).
pub fn env(&self) -> &Arc<GlobalEnv>;
```

## Wrapping Rust functions

The `wrap_fn*` family converts idiomatic Rust signatures into `NativeFn` values.
Arguments and return values are marshalled automatically via the `FromValue` and
`IntoValue` traits.

```rust
use cljrs_interop::{wrap_fn0, wrap_fn1, wrap_fn2, wrap_fn3, wrap_fn_variadic};

// Zero arguments
wrap_fn0("my.ns/timestamp", || Ok::<i64, String>(epoch_millis()))

// One argument
wrap_fn1("my.ns/double", |x: i64| Ok::<i64, String>(x * 2))

// Two arguments
wrap_fn2("my.ns/add", |a: i64, b: i64| Ok::<i64, String>(a + b))

// Three arguments
wrap_fn3("my.ns/clamp", |lo: i64, hi: i64, x: i64| Ok::<i64, String>(x.clamp(lo, hi)))

// Variadic — receives &[Value] directly; minimum argument count enforced
wrap_fn_variadic("my.ns/sum", 0, |args: &[Value]| {
    let total: i64 = args.iter().filter_map(|v| i64::from_value(v).ok()).sum();
    Ok::<i64, String>(total)
})
```

All wrappers accept closures (not just bare `fn` pointers), so they can capture
Rust state:

```rust
let multiplier = Arc::new(AtomicI64::new(3));
let m = multiplier.clone();
r.define("my.ns/scale",
    wrap_fn1("scale", move |x: i64| {
        Ok::<i64, String>(x * m.load(Ordering::Relaxed))
    }));
```

## Type marshalling

### Built-in conversions

| Clojure type | Rust type |
|---|---|
| `nil` | `()` or `Option<T>` (`None`) |
| `true` / `false` | `bool` |
| Long | `i64` |
| Double | `f64` (also accepts Long) |
| String | `String` |
| Any value | `Value` (pass-through) |
| Nil or any | `Option<T>` |
| BigInt | `num_bigint::BigInt` |

Implement `FromValue` and/or `IntoValue` on your own types to add new
conversions:

```rust
use cljrs_interop::{FromValue, IntoValue};
use cljrs_value::{Value, ValueResult};

struct Point { x: f64, y: f64 }

impl IntoValue for Point {
    fn into_value(self) -> Value {
        // encode as a two-element vector
        Value::vector(vec![self.x.into_value(), self.y.into_value()])
    }
}
```

### Error bridging

All `wrap_fn*` closures return `Result<R, E>` where `E: Display`. Any `Err`
value is converted to a Clojure exception and re-thrown; `Ok` values are
marshalled via `IntoValue`.

For explicit control, use `wrap_result`:

```rust
use cljrs_interop::wrap_result;

fn my_fn(args: &[Value]) -> ValueResult<Value> {
    let n = i64::from_value(&args[0])?;
    wrap_result(std::fs::read_to_string(format!("/tmp/{n}.txt")))
}
```

## Opaque Rust objects (`NativeObject`)

Arbitrary Rust structs can be wrapped as Clojure values using the `NativeObject`
trait. The value appears in Clojure as an opaque object that can be passed
around, stored in collections, and dispatched on via protocols.

```rust
use cljrs_interop::{NativeObject, gc_native_object};
use cljrs_gc::{MarkVisitor, Trace};
use cljrs_value::Value;

#[derive(Debug)]
struct Connection { /* ... */ }

impl NativeObject for Connection {
    fn type_tag(&self) -> &str { "Connection" }
    fn as_any(&self) -> &dyn std::any::Any { self }
}

// Connection holds no GcPtr fields, so Trace is a no-op.
impl Trace for Connection {
    fn trace(&self, _: &mut MarkVisitor) {}
}

// Create a Value::NativeObject wrapping a Connection.
fn make_conn(_args: &[Value]) -> ValueResult<Value> {
    let conn = Connection { /* ... */ };
    Ok(Value::NativeObject(gc_native_object(conn)))
}
```

To downcast back to the concrete type in a Rust function:

```rust
fn use_conn(args: &[Value]) -> ValueResult<Value> {
    let Value::NativeObject(obj) = &args[0] else {
        return Err(ValueError::WrongType { expected: "Connection", got: "…".into() });
    };
    let conn = obj.get().downcast_ref::<Connection>()
        .ok_or_else(|| ValueError::WrongType { expected: "Connection", got: obj.get().type_tag().into() })?;
    // use conn…
    Ok(Value::Nil)
}
```

### Protocol dispatch on native objects

`extend-type` can be used in Clojure to implement protocols for native objects.
The type tag (the string returned by `type_tag()`) is used for dispatch:

```clojure
(defprotocol IConn
  (query [conn sql])
  (close! [conn]))

(extend-type Connection IConn
  (query  [conn sql]  (native/db-query conn sql))
  (close! [conn]      (native/db-close conn)))
```

### GC integration

If your `NativeObject` contains `GcPtr<T>` fields, implement `Trace` properly
so the GC can follow references:

```rust
use cljrs_gc::{GcPtr, MarkVisitor, Trace};
use cljrs_value::Value;

struct Cache { entries: Vec<GcPtr<Value>> }

impl Trace for Cache {
    fn trace(&self, visitor: &mut MarkVisitor) {
        for entry in &self.entries {
            entry.trace(visitor);
        }
    }
}
```

If your struct holds no `GcPtr` fields (only plain Rust data), a no-op `Trace`
impl is sufficient.
