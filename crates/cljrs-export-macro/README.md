# cljrs-export-macro

Proc-macro crate backing `#[cljrs_interop::export]`. Exposes Rust functions as
Clojurust native functions with zero hand-written registration boilerplate.

**Do not depend on this crate directly.** Add `cljrs-interop` instead, which
re-exports the macro. The compiler also uses the internal ABI boundary attribute directly.

**Phase:** 9 — Rust Interop (implemented).

---

## File layout

```
src/
  lib.rs  — #[export] and #[catch_realization] attribute implementations
```

---

## Public API

### `#[export]`

```rust
#[proc_macro_attribute]
pub fn export(attr: TokenStream, item: TokenStream) -> TokenStream
```

Attribute macro that:
1. Leaves the original function unchanged.
2. Generates an [`inventory`] submission so the function is collected at link time.
3. Inlines `FromValue` / `IntoValue` marshalling for each parameter and the
   return value — no `wrap_fn*` calls required.

### Attribute arguments

| Key | Type | Required | Description |
|-----|------|----------|-------------|
| `ns` | string literal | **yes** | Clojure namespace, e.g. `"math"` |
| `name` | string literal | no | Override the Clojure symbol name (default: fn name with `_` → `-`) |
| `variadic_min` | integer | no | Minimum arity for variadic functions (default 0) |
| `blocking` | flag | no | Run owned, Send-only native work on the worker pool when a fiber calls it |

### Supported signatures

**Fixed arity** — each parameter must implement `FromValue`:
```rust
#[export(ns = "math")]
pub fn add(a: i64, b: i64) -> Result<i64, String> { Ok(a + b) }
```

**Plain return value** — any type implementing `IntoValue` (wrapped in `Ok` automatically):
```rust
#[export(ns = "math")]
pub fn pi() -> f64 { std::f64::consts::PI }
```

**No return value** — maps to `Value::Nil`:
```rust
#[export(ns = "log")]
pub fn log_info(msg: String) { println!("{msg}"); }
```

**Variadic** — single `&[Value]` parameter:
```rust
#[export(ns = "math", variadic_min = 1)]
pub fn sum(args: &[Value]) -> Result<Value, String> {
    let total: i64 = args.iter()
        .map(|v| i64::from_value(v).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter().sum();
    Ok(total.into_value())
}
```

### Blocking exports (virtual threads Phase 2)

```rust
#[export(ns = "files", blocking)]
fn read_text(path: String) -> Result<String, std::io::Error> {
    std::fs::read_to_string(path)
}
```

The wrapper converts arguments on the isolate before it starts the worker.
The worker runs the Rust function while the fiber parks. The wrapper converts its result on the isolate after the worker returns.
Inputs, results, and error types must be owned, `Send`, and `'static`.
Variadic `&[Value]` exports cannot use `blocking` because managed values cannot cross threads.
Outside a fiber, the wrapper calls the Rust function directly.

Fixed-arity exports also generate a separate adapter for the Clojure `blocking` form.
This adapter does not require the `blocking` attribute.
It offloads only inputs and outputs that implement `Send + 'static`.
Unsupported signatures remain valid for ordinary calls and return an error from `blocking` before native execution.
Variadic exports do not provide this adapter.

### Name conversion

Rust `fn my_cool_fn` becomes Clojure `my-cool-fn` by default.
Override with `name = "..."`.

---

## Generated code (example)

```rust
#[export(ns = "math")]
pub fn add(a: i64, b: i64) -> Result<i64, String> { Ok(a + b) }
```

The macro preserves this function and registers a `NativeFn::with_blocking` factory.
Its ordinary adapter converts arguments, calls `add`, and converts the result.
Its blocking adapter uses the same conversions around an owned worker call.
The generated worker selection is equivalent to:

```rust,ignore
use cljrs_interop::blocking::{BlockingCall, BlockingDispatch};
let call = BlockingCall::new(
    "math/add",
    (a, b),
    |(a, b)| add(a, b),
    |result| result.map(cljrs_interop::IntoValue::into_value)
        .map_err(|error| cljrs_interop::ValueError::Other(error.to_string())),
);
(&call).run_blocking()
```

The input tuple and output type determine worker eligibility before execution.
The fallback rejects a signature that cannot cross threads.
The conversion callback stays on the isolate and needs no `Send` bound.

---

## Dependencies

| Crate | Role |
|-------|------|
| `syn 2` | Parse Rust function signatures |
| `quote 1` | Generate token streams |
| `proc-macro2 1` | Span-aware token manipulation |

### `#[catch_realization(handler[, cancel_handler])]`

`catch_realization(handler: TokenStream, item: TokenStream) -> TokenStream`
wraps a function in `cljrs_value::realization::catch`.
The handler takes an error string and returns the function result type.
An optional second handler also wraps the body in `cljrs_value::parking::catch_cancelled`
and receives the resulting `ValueError::Cancelled`.
The compiler uses `#[catch_realization(throw_str, signal_error)]` on boxed runtime ABI bridges.
A realization error becomes a pending Clojure exception before the function returns across the C ABI.
Other panics continue unwinding.
