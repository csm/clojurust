# cljx-eval

Tree-walking interpreter for clojurust. Evaluates `Form` AST nodes produced by
`cljx-reader` within a namespace-aware lexical environment.

**Phase:** 4 — stub only, not yet implemented.

---

## File layout

```
src/
  lib.rs    — doc-comment stub describing planned implementation
```

---

## Planned public API (Phase 4)

```rust
/// Evaluate a single top-level form in `env` and return the resulting Value.
pub fn eval(form: &Form, env: &mut Env) -> CljxResult<Value>

/// A lexical environment: bindings, namespace registry, macro table.
pub struct Env { /* private */ }

impl Env {
    pub fn new() -> Self
    pub fn with_namespace(ns: &str) -> Self
}
```

Planned features:
- All Clojure special forms: `def`, `let*`, `fn*`, `if`, `do`, `quote`,
  `var`, `loop*`, `recur`, `letfn*`, `throw`, `try`
- Macro expansion pipeline (`macroexpand-1`, `macroexpand`)
- Tail-call optimization via `recur`
- Sequential and associative destructuring in `let`/`fn`/`loop`

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljx-types` (workspace) | `Span`, `CljxError`, `CljxResult` |
| `cljx-gc` (workspace) | `GcPtr<Value>` for all runtime values |
| `cljx-reader` (workspace) | `Form` AST nodes consumed by the evaluator |
