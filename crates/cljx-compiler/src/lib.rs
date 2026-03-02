//! JIT and AOT compiler for clojurust.
//!
//! Phase 10 (JIT) and Phase 11 (AOT) will implement:
//! - IR lowering from `Form` AST
//! - Cranelift-based native code generation
//! - Inline caches for protocol dispatch and keyword lookup
//! - On-stack replacement (OSR) from interpreter to JIT
//! - AOT whole-program analysis and static linking
