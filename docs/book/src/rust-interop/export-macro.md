# The `#[export]` Macro

The `#[export]` attribute (from `cljrs_interop`) is the zero-boilerplate way to
expose Rust functions to Clojure. Annotating a function with it causes the
function to be registered automatically when a `Registry` is created — no
explicit call required.

## Basic usage

```rust
use cljrs_interop::export;

#[export(ns = "math")]
pub fn add(a: i64, b: i64) -> Result<i64, String> {
    Ok(a + b)
}
```

`add` is now visible in Clojure as `math/add` as soon as the shared library is
loaded or the AOT binary starts. No `cljrs_init` is required unless you have
other setup to perform (see [When `cljrs_init` is still needed](#when-cljrs_init-is-still-needed)).

## Attribute options

| Key | Required | Description |
|---|---|---|
| `ns` | **yes** | Clojure namespace, e.g. `"math"` or `"my.project"`. |
| `name` | no | Override the Clojure symbol name. Default: Rust name with `_` replaced by `-`. |
| `variadic_min` | no | Minimum arity for variadic functions (default `0`). Only valid when the function takes a single `&[Value]` parameter. |

## Name mapping

Rust function names are converted to Clojure-style kebab-case: every `_` becomes
`-`. Use `name = "..."` to override:

```rust
#[export(ns = "str.util")]
fn to_upper_case(s: String) -> String {        // → str.util/to-upper-case
    s.to_uppercase()
}

#[export(ns = "str.util", name = "upper")]     // → str.util/upper
fn to_upper_case_v2(s: String) -> String {
    s.to_uppercase()
}
```

## Supported signatures

### Fixed arity

Each parameter must implement `FromValue`. The parameter count sets the arity
enforced by the runtime. There is no upper limit.

**Plain return value** — any type implementing `IntoValue`; wrapped in `Ok`
automatically:

```rust
#[export(ns = "math")]
pub fn pi() -> f64 {
    std::f64::consts::PI
}
```

**`Result<T, E>` return** — `Err` becomes a Clojure exception via `E::to_string()`:

```rust
#[export(ns = "math")]
pub fn safe_sqrt(x: f64) -> Result<f64, String> {
    if x < 0.0 {
        Err(format!("cannot take sqrt of {x}"))
    } else {
        Ok(x.sqrt())
    }
}
```

**No return value** — maps to `nil`:

```rust
#[export(ns = "log")]
pub fn log_info(msg: String) {
    eprintln!("[info] {msg}");
}
```

**Four or more parameters** work identically to two or three:

```rust
#[export(ns = "geom")]
pub fn rect_contains(rx: f64, ry: f64, rw: f64, rh: f64, px: f64, py: f64) -> bool {
    px >= rx && px <= rx + rw && py >= ry && py <= ry + rh
}
```

### Variadic

For functions that take a variable number of arguments, use a single `&[Value]`
parameter. Set `variadic_min` to enforce a minimum argument count:

```rust
use cljrs_interop::{FromValue, IntoValue, export};
use cljrs_value::Value;

#[export(ns = "math", variadic_min = 1)]
pub fn sum(args: &[Value]) -> Result<Value, String> {
    let total: i64 = args
        .iter()
        .map(|v| i64::from_value(v).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .sum();
    Ok(total.into_value())
}
```

```clojure
(math/sum 1 2 3 4 5)   ; => 15
```

## When `cljrs_init` is still needed

`#[export]` handles function registration. A `cljrs_init` is still required when
you need to:

- Call `mark_loaded` so `require` treats a namespace as built-in rather than
  looking for a file on the source path.
- Set namespace aliases or refer bindings via `Registry::env()`.
- Perform any other startup work beyond defining Clojure-visible functions.

```rust
use cljrs_interop::Registry;

#[no_mangle]
pub extern "C" fn cljrs_init(registry: *mut Registry) {
    let r = unsafe { &mut *registry };
    // #[export] functions are already registered — Registry::new ran first.
    r.env().mark_loaded("math");
    r.env().mark_loaded("log");
}
```

> **Note:** The `*mut Registry` passed to `cljrs_init` is the same `Registry`
> created by the runtime before calling your function. All `#[export]` entries
> are already interned when `cljrs_init` is invoked.

## Mixing `#[export]` with manual `define`

Both styles can coexist freely. Use `#[export]` for straightforward functions
and `r.define` / `wrap_fn*` for cases that capture runtime state or need custom
arity logic:

```rust
use cljrs_interop::{Registry, wrap_fn1, export};
use std::sync::{Arc, Mutex};

// Simple stateless function — use #[export].
#[export(ns = "counter")]
pub fn increment(n: i64) -> i64 {
    n + 1
}

#[no_mangle]
pub extern "C" fn cljrs_init(registry: *mut Registry) {
    let r = unsafe { &mut *registry };

    // Stateful closure that captures a value created at init time.
    let total: Arc<Mutex<i64>> = Arc::new(Mutex::new(0));
    let t = total.clone();
    r.define(
        "counter/running-total",
        wrap_fn1("running-total", move |n: i64| {
            let mut guard = t.lock().unwrap();
            *guard += n;
            Ok::<i64, String>(*guard)
        }),
    );
}
```

## How it works

`#[export]` is a proc-macro (in `cljrs-export-macro`) that leaves the original
function intact and emits an [`inventory`](https://docs.rs/inventory) submission:

```rust
// What #[export(ns = "math")] generates alongside your function:
::cljrs_interop::inventory::submit!(::cljrs_interop::ExportEntry {
    qualified: "math/add",
    make_fn: || {
        ::cljrs_interop::NativeFn::with_closure(
            "math/add",
            ::cljrs_interop::Arity::Fixed(2),
            move |args| {
                let __a0 = <i64 as ::cljrs_interop::FromValue>::from_value(&args[0])?;
                let __a1 = <i64 as ::cljrs_interop::FromValue>::from_value(&args[1])?;
                add(__a0, __a1)
                    .map(<i64 as ::cljrs_interop::IntoValue>::into_value)
                    .map_err(|e| ::cljrs_interop::ValueError::Other(e.to_string()))
            },
        )
    },
});
```

`inventory` uses linker constructors to collect all submissions across the
binary at link time. `Registry::new` then iterates them and calls
`Registry::define` for each one.

If you need to register exports into a `Registry` you constructed outside the
normal runtime flow (e.g. in tests), call `register_exports` directly:

```rust
use cljrs_interop::{register_exports, Registry};

let r = Registry::new(env.clone());  // already auto-registered
// …or, if you have a registry from elsewhere:
register_exports(&r);
```
