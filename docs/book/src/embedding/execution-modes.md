# Execution modes

An `ExecutionMode` is chosen once, when the runtime is built, and never changes.
It selects which path a Clojure function call takes:

```rust
pub enum ExecutionMode {
    TreeWalk,
    Tiered,          // default
    TieredNoJit,
    NoGcTransaction,
}
```

| Mode | Calls dispatch through | Promoted to | Use it for |
|---|---|---|---|
| `TreeWalk` | the tree-walking interpreter | `TreeWalk` | short-lived runtimes: one-shot evaluation, tests, config loading, per-request sandboxes |
| `Tiered` | the IR-aware dispatcher | `Jit` | long-running hosts — the default, and what `cljrs run`/`repl` use |
| `TieredNoJit` | the IR-aware dispatcher | `Ir` | long-running hosts that must not generate native code, or that want reproducible timing |
| `NoGcTransaction` | the tree walker, behind a call-depth cap | `TreeWalk` | bounded evaluation of untrusted pure functions (this is what `cljrs-tx` builds) |

## Tier state: what is live right now

`ExecutionMode` is the *target*; `TierState` is the current reality.

```rust
pub enum TierState { TreeWalk = 0, Ir = 1, Jit = 2 }
```

Every runtime starts at `TreeWalk` — nothing can be lowered to IR before
`clojure.core` exists — and `build()` raises it once, at the very end of
bootstrap, to the mode's target tier. It only ever moves up, and only that
once; `runtime.tier_state()` reports where it landed.

The distinction matters because "not tree-walking" is not one thing:
`TieredNoJit` stops at Tier 1 and stays there **even if a JIT backend is linked
into your binary**, which a single "compiler ready" flag could not express.

```rust
let runtime = Runtime::builder()
    .execution_mode(ExecutionMode::TieredNoJit)
    .build()?;
assert_eq!(runtime.tier_state(), TierState::Ir);   // after bootstrap
```

## How a function heats up in `Tiered` mode

1. **Tree walk.** Every function starts here. Calls are counted.
2. **Tier 1 — IR.** After roughly 50 calls the arity is queued for background
   lowering on the `cljrs-ir-lower` worker thread; once its IR is published,
   calls run on the register-based IR interpreter instead.
3. **Tier 2 — native.** After roughly 1,000 calls the arity is queued for JIT
   compilation, and subsequent calls jump straight to native code. Long-running
   loops can transfer mid-execution through on-stack replacement.

Lowering and compilation happen off the evaluation thread, so a call that is
"hot enough" does not block while its faster version is built — it keeps running
at the current tier until the new one is published.

## Attaching the JIT

`Tiered` mode only *targets* native dispatch. Tier 2 requires a JIT backend,
which lives in `cljrs-compiler` and is not linked in unless you install it:

```rust
let runtime = Runtime::builder()
    .execution_mode(ExecutionMode::Tiered)
    .build()?;

cljrs_compiler::jit::install(&runtime);   // install BEFORE any guest code runs
cljrs_stdlib::install(&runtime);
```

Without that call, `Tiered` behaves like `TieredNoJit`.

**Install it before evaluating anything.** An arity that reaches the JIT
threshold with no backend attached is marked as queued and never re-enqueued —
that flag is what stops hot dispatch from re-queueing the same arity on every
call — so a host that evaluates first and installs afterwards silently gets no
JIT for whatever ran in between. `cljrs_compiler::jit::install_on(&globals)` is
the same call for a host holding the `Arc<GlobalEnv>` rather than the `Runtime`.

## Tuning

Thresholds are **process-global**, not per runtime. Set them before building:

```rust
use cljrs_runtime::tiered::{set_ir_threshold, set_jit_threshold, set_osr_threshold};

set_ir_threshold(10);      // bring IR lowering in sooner
set_jit_threshold(500);    // and native compilation with it
set_osr_threshold(5_000);  // loop iterations before on-stack replacement
```

| Variable | Effect |
|---|---|
| `CLJRS_NO_IR` | Pins any runtime built afterwards at `TierState::TreeWalk`. The escape hatch when you suspect a lowering bug. |
| `CLJRS_IR_THRESHOLD` | Calls before background IR lowering (default 50). `u32::MAX` disables lowering. |
| `CLJRS_JIT_THRESHOLD` | Calls before JIT compilation (default 1,000). |
| `CLJRS_OSR_THRESHOLD` | Loop iterations before on-stack replacement. |
| `CLJRS_EAGER_LOWER=1` | Lower every function at definition time instead of when it gets hot. Expensive; useful for reproducing lowering bugs. |
| `CLJRS_IR_CACHE_TTL` | Seconds an idle lowered arity survives before the cold-IR sweep drops it (default 600). |

The programmatic setters take precedence over the environment variables, which
in turn override the defaults. Note that `CLJRS_NO_JIT` is read by the **CLI**,
not by the runtime: in a host program, whether a JIT is attached is decided by
whether you call `jit::install`, so check the variable yourself if you want to
honour it.

## Pre-lowered IR

`cljrs ir build` can serialise lowered IR to a bundle, and a host can replay it
into a runtime's cache:

```rust
let loaded = cljrs_runtime::tiered::load_prebuilt_ir(&globals, &bundle);
```

It walks the namespaces, matches each function arity against a bundle key, and
returns how many arities it installed — skipping the warm-up for those
functions. No `cljrs` runtime path loads a bundle today; the command exists as a
lowerer diagnostic, and this entry point is what an embedding host would use if
it wanted to ship one.

## `NoGcTransaction` and the depth cap

`NoGcTransaction` routes calls through a depth-capped tree walker. Interpreted
recursion consumes real Rust stack, so an unbounded recursion in guest code
would overflow the host thread's stack and abort the *process* — not something a
host can catch. The cap turns that into an ordinary evaluation error.

```rust
use cljrs_runtime::env::depth::DepthGuard;

let runtime = Runtime::builder()
    .execution_mode(ExecutionMode::NoGcTransaction)
    .build()?;

let _depth = DepthGuard::install(1_000);   // scoped to this invocation
let result = eval(&form, &mut env);        // deeper nesting → EvalError::Runtime
```

The budget is thread-local and clears when the guard drops, including on
unwind. With no guard installed the mode is exactly the plain tree walker.

The name comes from its other half: built with the workspace's `no-gc` feature,
this mode runs inside a region-allocated arena that is discarded wholesale when
the invocation ends, with no collector and no write barriers. `cljrs-tx` packages
that whole profile — see [Limits & sandboxing](sandboxing.md#running-untrusted-code).

## Choosing

- **Long-running host, trusted code** → `Tiered` plus `cljrs_compiler::jit::install`.
  Pay the warm-up once, run fast forever.
- **Long-running host, no native codegen** (a policy forbidding W^X pages, a
  platform without a JIT, or a need for predictable timing) → `TieredNoJit`.
- **Short-lived or one-shot** (evaluate a config file, run a test, handle one
  request and drop the runtime) → `TreeWalk`. Populating an IR cache you will
  throw away is pure overhead.
- **Untrusted input** → `NoGcTransaction` with a depth cap and a gas meter, or
  `cljrs-tx` for the packaged version.
