# cljrs-interp

Self-contained tree-walking interpreter for Clojure.

**Phase:** Core interpreter — implemented.  `no-gc` region/static-sink support (Phases 4–5), blacklist integration (Phase 6), and integration tests (Phase 8) of `docs/no-gc-plan.md` — implemented.

---

## Purpose

Evaluates Clojure `Form` ASTs produced by `cljrs-reader`, managing lexical
environments, special forms, function application, and the recur trampoline.
Under the `no-gc` Cargo feature, applies the allocation-context stack protocol
(scratch regions for function/loop scopes; `StaticArena` for static-sink
expressions).

---

## File layout

```
src/
  lib.rs         — crate entry point; re-exports Interpreter
  eval.rs        — top-level eval dispatch; symbol/keyword/collection eval
  special.rs     — special form evaluators: def, defn, defmacro, fn*, if, let*,
                   loop*, recur, quote, var, set!, throw, try, do, ns, require,
                   letfn, in-ns, alias, defprotocol, extend-type, extend-protocol,
                   defmulti, defmethod, defrecord, reify, binding, with-out-str
  apply.rs       — eval_call: macro expansion, native-fn dispatch, CljxFn
                   application, recur trampoline; special env-needing handlers
                   (apply, atom, reset!, swap!, volatile!, vreset!, vswap!,
                   agent, send/send-off, with-bindings*, alter-var-root,
                   vary-meta, find-ns, all-ns, create-ns, ns-aliases, remove-ns,
                   alter-meta!, ns-resolve, resolve, intern, bound-fn*)
  arity.rs       — fresh arity ID generator
  destructure.rs — pattern destructuring (vector, map, & rest)
  macros.rs      — macro expansion helpers
  syntax_quote.rs — syntax-quote (backtick) expansion
  virtualize.rs  — let-chain virtualization: assoc/conj chains → transients
tests/
  no_gc_eval.rs  — (no-gc mode) integration tests: arithmetic, def/defn provenance,
                   function-call region stack, loop/recur accumulation,
                   atom/reset!/swap! static-sink correctness
```

---

## Public API

### `eval(form, env) -> EvalResult`

Evaluate a single `Form` in `env`.  Entry point for the interpreter.

### `eval_call(func_form, arg_forms, env) -> EvalResult`

Evaluate a function-call form.  Handles macros, native-function special cases,
and user-defined `CljxFn` application with the recur trampoline.

### `eval_body(forms, env) -> EvalResult`

Evaluate a sequence of forms, returning the value of the last one.

### `eval_loop(args, env) -> EvalResult`

Evaluate a `loop*` form.  Under `no-gc`, pushes a `ScratchGuard` on each
iteration and pops it before the tail expression (recur args or return value)
so intermediate allocations are freed per iteration.

### `eval_defn(args, env) -> EvalResult`

Evaluate a `defn` form.  Under `no-gc`, wraps fn creation in `StaticCtxGuard`
so the `CljxFn` object lands in the `StaticArena`.

### Special handlers in `apply.rs`

Each handler evaluates its key expressions under the correct allocation context:

| Handler | Static-sink guard coverage |
|---|---|
| `handle_atom_call` | initial value |
| `handle_reset_bang` | new value |
| `handle_swap_call` | function return value |
| `handle_volatile` | initial value |
| `handle_vreset` | new value |
| `handle_vswap` | function return value |
| `handle_agent_call` | initial value |
| `handle_alter_var_root` | function return value |
| `handle_intern` | value expression (3-arg form) |

### Value-level special form helpers (IR interpreter API)

The IR interpreter receives already-evaluated `Vec<Value>` arguments rather than
`&[Form]` AST nodes.  These public functions mirror the `handle_*` form-level
handlers but accept pre-evaluated args, allowing the IR interpreter to
implement sentinel operations without hitting the stub errors registered in
`clojure.core`:

| Function | Operation |
|---|---|
| `eval_swap_bang(args, env)` | `swap!` — apply f to atom, store result |
| `eval_volatile(args)` | `volatile!` — create a new volatile |
| `eval_vreset_bang(args)` | `vreset!` — reset volatile value |
| `eval_vswap_bang(args, env)` | `vswap!` — apply f to volatile value, store result |
| `make_delay_from_fn(f, globals, ns)` | `make-delay` — wrap zero-arg fn in a `Delay` |
| `eval_alter_var_root(args, env)` | `alter-var-root` — apply f to var root, store result |
| `eval_vary_meta(args, env)` | `vary-meta` — apply f to obj metadata |
| `eval_with_bindings_star(args, env)` | `with-bindings*` — push binding frame, call f |
| `eval_send_to_agent(args, env)` | `send` / `send-off` — dispatch action to agent |

`make_lazy_seq_from_fn(f, globals, ns)` (already public) creates a `LazySeq`
from a zero-arg callable; the above `make_delay_from_fn` is the analogous
helper for `Delay`.
