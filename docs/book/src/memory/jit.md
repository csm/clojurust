# JIT & tiered execution

> **Status: planned (Phase 10).** This page describes work that is designed but
> not yet implemented. The full architecture and roadmap live in
> [`docs/jit-plan.md`](https://github.com/csm/clojurust/blob/main/docs/jit-plan.md).

clojurust runs code through a series of *tiers*, each faster than the last and
selected automatically based on how hot the code is:

| Tier | Engine | When it runs |
|---|---|---|
| **0** | Tree-walking interpreter (`cljrs-interp`) | Always available; the universal fallback |
| **1** | IR register interpreter (`cljrs-eval`) | When a function's ANF/SSA IR is cached |
| **2** | AOT native code (`cljrs-compiler`) | After an explicit `cljrs compile` |
| **JIT** | In-process native code (`cljrs-jit`) | *Planned* — when a function or loop gets hot at runtime |

All tiers meet at one seam — the `call_cljrs_fn` dispatch hook — and a function
transparently moves up the tiers as it warms up, falling back when a form isn't
yet supported.

The **JIT tier** brings native speed to *ad-hoc* code — scripts run with
`cljrs run` and expressions typed at the REPL — without any explicit compile
step. Its design covers:

- **Hot-path detection** via per-arity invocation counters and loop back-edge
  counters, with **on-stack replacement (OSR)** so a single long-running loop
  promotes to native mid-execution.
- **Background compilation** on a worker thread, with the finished code swapped
  in atomically so a hot call never stalls.
- **Code unloading** that reclaims native code when the REPL redefines a
  function, tied to the [garbage collector's](index.md) stop-the-world
  safepoints so there is no unload-vs-execute race.
- **Context-driven [bump allocation](bump-allocator.md):** the JIT specializes a
  function's allocation strategy to the context it is called from, threading the
  caller's region through proven-non-escaping calls — extending the bump
  allocator beyond AOT code into the default GC build.

See [`docs/jit-plan.md`](https://github.com/csm/clojurust/blob/main/docs/jit-plan.md)
for the complete design and the phased Phase 10 roadmap.
