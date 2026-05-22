# cljrs-export-macro

Proc-macro crate backing `#[cljrs_interop::export]`. Exposes Rust functions as
Clojurust native functions with zero hand-written registration boilerplate.

**Do not depend on this crate directly.** Add `cljrs-interop` instead, which
re-exports the macro.

**Phase:** 9 — Rust Interop (implemented).

---

## File layout

```
src/
  lib.rs  — #[export] proc-macro attribute implementation
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

### Name conversion

Rust `fn my_cool_fn` becomes Clojure `my-cool-fn` by default.
Override with `name = "..."`.

---

## Generated code (example)

```rust
#[export(ns = "math")]
pub fn add(a: i64, b: i64) -> Result<i64, String> { Ok(a + b) }
```

Expands to (approximately):

```rust
pub fn add(a: i64, b: i64) -> Result<i64, String> { Ok(a + b) }

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
            }
        )
    },
});
```

---

## Dependencies

| Crate | Role |
|-------|------|
| `syn 2` | Parse Rust function signatures |
| `quote 1` | Generate token streams |
| `proc-macro2 1` | Span-aware token manipulation |
