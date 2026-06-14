# cljrs-builtins

Built-in functions for clojurust (the `clojure.core`-equivalent runtime
implemented in Rust, registered into a name → fn dispatch table).

Includes the `unchecked-*` integer arithmetic family — `unchecked-add`,
`unchecked-subtract`, `unchecked-multiply`, `unchecked-inc`, `unchecked-dec`,
`unchecked-negate` (and their `-int` aliases) — which wrap on overflow, in
contrast to the checked `+`/`-`/`*` (which throw on overflow at the IR/compiled
tiers and promote to BigInt in the tree-walk tier).
