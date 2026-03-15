# cljrs-compiler

JIT and AOT compiler backend for clojurust. Lowers `Form` AST nodes to native
code via a Cranelift-based code generator. Hot interpreter paths are promoted
to compiled code at runtime (JIT); `cljx compile` produces standalone binaries
(AOT).

**Phase:** 10 (JIT) + Phase 11 (AOT) — stub only, not yet implemented.

---

## File layout

```
src/
  lib.rs    — doc-comment stub describing planned implementation
```

---

## Planned public API (Phase 10 + 11)

```rust
/// JIT-compile a function and return a callable native function pointer.
/// The function must already have been interpreted at least once so its
/// call-site profile is available.
pub fn jit_compile(func: &Value, env: &Env) -> CljxResult<NativeFn>

/// AOT-compile `file` to a self-contained native binary at `output`.
pub fn aot_compile(file: &Path, output: &Path, env: &Env) -> CljxResult<()>
```

Planned features:
- IR lowering from `Form` AST (SSA-based intermediate representation)
- [Cranelift](https://cranelift.dev/) native code generation (x86-64 and AArch64)
- Inline caches for keyword lookup and protocol dispatch
- On-stack replacement (OSR): seamlessly promote an in-progress interpreter
  frame to compiled code
- AOT whole-program analysis, dead-code elimination, and static linking

Both JIT and AOT share the same `Value` representation as the interpreter — no
separate boxed/unboxed split at the API boundary; specialization is internal to
the compiler.

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-types` (workspace) | `Span`, `CljxError`, `CljxResult` |
| `cljrs-gc` (workspace) | `GcPtr<Value>` — GC interaction during compilation |
| `cljrs-eval` (workspace) | `Env`, `Form`, `Value` — input to the compiler pipeline |
