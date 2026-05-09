# cljrs-builtins

Built-in (`clojure.core`) function implementations for clojurust.

## Status

Phases 4–8 — implemented.  Roughly 300+ Rust-implemented functions covering
arithmetic, collections, sequences, I/O, regex, atoms/refs, futures/promises,
agents, transients, records, multimethods, and protocols.

## Cargo features

| Feature | Default | Effect |
|---|---|---|
| `no-gc` | off | Propagate `no-gc` to `cljrs-gc`. |
| `async` | off | Pull in `tokio`; future home for the Phase B `await` special-form dispatch helpers. |

## File layout

```
src/
  lib.rs           — module declarations and re-exports
  builtins.rs      — bulk of the implementations and the registry that
                     installs them into `GlobalEnv` (one entry per Clojure name).
  array_list.rs    — Java-compatible java.util.ArrayList helpers.
  bitops.rs        — bit-and / bit-or / bit-xor / shifts / bit-test / etc.
  form.rs          — form-construction helpers used by special forms.
  new.rs           — `(new ...)` constructor dispatch.
  regex.rs         — re-pattern / re-find / re-matches / re-seq.
  special.rs       — special-form helpers exposed to the IR interpreter.
  taps.rs          — tap> / add-tap / remove-tap.
  transients.rs    — transient!/ persistent! / conj!/ assoc!/ disj!/ dissoc!/ pop!.
  util.rs          — small shared helpers.
  bootstrap.cljrs  — Clojure source for built-ins that are easier to write in
                     Clojure than in Rust.
  clojure_test.cljrs — pure-Clojure clojure.test compatibility shim.
```

## Notable APIs and naming

- `(await-agent agent ...)` — block until the listed agents have processed all
  pending actions.  This is Clojure's `clojure.core/await` renamed; Phase A
  reserves the bare name `await` for the upcoming async special form.
- `(future-cancel f)`, `(future-done? f)`, `(future-cancelled? f)` — wrappers
  over `CljxFuture::{cancel, is_done, is_cancelled}`.
- `(deref f)` / `@f` on a future calls `CljxFuture::blocking_deref` (or
  `blocking_deref_timeout` with a `(deref f ms timeout-val)`); it parks the OS
  thread, never the tokio executor.
