# cljrs-builtins

Built-in functions for clojurust (the `clojure.core`-equivalent runtime
implemented in Rust, registered into a name → fn dispatch table).

Includes the `unchecked-*` integer arithmetic family — `unchecked-add`,
`unchecked-subtract`, `unchecked-multiply`, `unchecked-inc`, `unchecked-dec`,
`unchecked-negate` (and their `-int` aliases) — which wrap on overflow, in
contrast to the checked `+`/`-`/`*` (which throw on overflow at the IR/compiled
tiers and promote to BigInt in the tree-walk tier).

## Phase B3 — `shared-atom` (cross-isolate, two-tier atom ADR)

`shared-atom` is the cross-isolate tier of the two-tier atom design in
`docs/async-worker-pool-plan.md`.  Unlike `atom` (isolate-local, GC-backed),
its contents are promoted to a `Send + Sync` `SharedValue`
(`cljrs_value::shared`) behind a lock-free `ArcSwap`, so the reference can cross
the isolate boundary and be mutated concurrently:

- `(shared-atom x)` — construct, promoting `x` (non-promotable values such as
  closures and native resources are rejected here).
- `(shared-atom? x)` — predicate.
- `deref` / `reset!` / `swap!` / `compare-and-set!` — dispatch on
  `Value::SharedAtom` alongside the local `atom` path; writes promote, reads
  demote, and `swap!`/`compare-and-set!` use a single lock-free CAS with retry.
