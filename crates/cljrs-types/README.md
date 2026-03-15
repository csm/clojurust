# cljrs-types

Core types shared across every cljx crate: source spans, the unified error/diagnostic
type, and the `CljxResult` alias.

**Phase:** 1 (Project Infrastructure) — fully implemented.

**Used by:** all other crates in the workspace.

---

## File layout

```
src/
  lib.rs      — module declarations: `pub mod error; pub mod span;`
  error.rs    — CljxError enum and CljxResult type alias
  span.rs     — Span struct (byte-range + location in a named source file)
```

---

## Public API

### `span::Span`

A half-open byte range `[start, end)` within a named source file.

```rust
pub struct Span {
    pub file:  Arc<String>,  // file path, or "<repl>"
    pub start: usize,        // inclusive byte offset
    pub end:   usize,        // exclusive byte offset
    pub line:  u32,          // 1-based line number of `start`
    pub col:   u32,          // 1-based byte column of `start`
}

impl Span {
    pub fn new(file: Arc<String>, start: usize, end: usize, line: u32, col: u32) -> Self
    pub fn len(&self) -> usize
    pub fn is_empty(&self) -> bool
}
```

**Trait impls:**
- `From<&Span> for miette::SourceSpan`
- `From<Span> for miette::SourceSpan`

### `error::CljxError`

Unified error type for all clojurust subsystems. Derives
[`miette::Diagnostic`](https://docs.rs/miette) so errors render with source
snippets and labels in the terminal.

```rust
pub enum CljxError {
    ReadError {
        message: String,
        span:    Option<miette::SourceSpan>,  // label position in source
        src:     miette::NamedSource<String>, // full source text + file name
    },
    EvalError {
        message: String,
        span:    Option<miette::SourceSpan>,
        src:     miette::NamedSource<String>,
    },
    Io(#[from] std::io::Error),
}
```

### `error::CljxResult<T>`

```rust
pub type CljxResult<T> = Result<T, CljxError>;
```

---

## Dependencies

| Crate | Role |
|-------|------|
| `miette` (workspace) | Diagnostic trait + `NamedSource`, `SourceSpan` |
| `thiserror` (workspace) | `#[derive(Error)]` codegen |
