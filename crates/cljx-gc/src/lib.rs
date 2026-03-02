//! Tracing garbage collector for clojurust.
//!
//! Phase 8 will implement:
//! - Mark-and-sweep GC with generational promotion
//! - `GcPtr<T>` smart pointer opaque to Rust's borrow checker
//! - Write barriers, weak references, and finalization hooks
//! - Safepoints integrated with the eval loop and JIT frames
