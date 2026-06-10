# cljrs-env

Environments for running programs in.

## gc_roots module

The `gc_roots` module manages GC root registration for the interpreter's Rust call stack. Public API includes:

- `push_env_root(env: &Env) -> EnvRootGuard` — registers an `Env` pointer as a GC root; guard removes on drop
- `root_value(val: &Value) -> ValueRootGuard` — registers a single `Value` pointer as a GC root
- `root_values(vals: &[Value]) -> ValueRootGuard` — registers a slice of `Value` pointers as GC roots
- `root_option_values(vals: &[Option<Value>]) -> OptionValueRootGuard` — registers an `Option<Value>` slice (e.g. IR register file)
- `gc_safepoint(env: &Env)` — interpreter-level safepoint: parks if collection in progress, or initiates collection on memory pressure
- `force_collect(env: &Env)` — immediately initiates a GC collection bypassing memory-pressure threshold
- `async_gc_collect()` — services a pending GC request from a Tokio `LocalSet` task at a cooperative yield point; safe to call when no other tasks are polling, so thread-local root stacks are stable and fully describe all suspended-task `GcPtr`s
- `set_stw_reclaim_hook(f)` — (Phase 10.2) installs a stop-the-world reclaim hook the JIT uses to free superseded native code; runs inside the STW guard at the tail of every collection (`force_collect`, `gc_safepoint`, `async_gc_collect`), when all mutator threads are parked
