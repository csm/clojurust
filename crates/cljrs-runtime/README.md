# cljrs-runtime

Core standard library for clojurust — the `clojure.core` equivalent. Provides
built-in functions, macros, and concurrency primitives that Clojure programs
expect to exist at startup.

**Phase:** 5 (core functions) + Phase 7 (concurrency) — stub only, not yet implemented.

---

## File layout

```
src/
  lib.rs    — doc-comment stub describing planned implementation
```

---

## Planned public API (Phase 5 + 7)

The runtime registers native Rust functions into the `clojure.core` namespace
at interpreter startup. End-user Clojure code calls these transparently; no
Rust API is public beyond the registration entry point:

```rust
/// Register all core functions and macros into `env`.
/// Called once during interpreter initialisation.
pub fn register_core(env: &mut cljrs_eval::Env)
```

Planned function groups:
- **Arithmetic & comparison** — `+`, `-`, `*`, `/`, `=`, `<`, `>`, `mod`, …
- **Type predicates** — `nil?`, `number?`, `string?`, `seq?`, `fn?`, …
- **Persistent collections** — `conj`, `assoc`, `dissoc`, `get`, `count`,
  `first`, `rest`, `cons`, `into`, …
- **Seq abstractions** — `map`, `filter`, `reduce`, `take`, `drop`, `flatten`, …
- **Lazy sequences** — `lazy-seq`, `range`, `repeat`, `iterate`, …
- **String & I/O** — `str`, `println`, `print`, `slurp`, `spit`, …
- **Core macros** — `when`, `when-not`, `cond`, `->`, `->>`, `and`, `or`,
  `doto`, `for`, `dotimes`, `while`, …
- **Concurrency (Phase 7)** — `atom`, `swap!`, `reset!`, `deref`; `ref`, `dosync`,
  `alter`, `commute`; `agent`, `send`; `future`, `promise`, `deliver`

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-types` (workspace) | `CljxError`, `CljxResult` |
| `cljrs-gc` (workspace) | `GcPtr<Value>` for all runtime values |
| `cljrs-eval` (workspace) | `Env` — namespace registry for function registration |
