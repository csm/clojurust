# cljrs-wasm

## Purpose

WebAssembly browser REPL for clojurust.  Compiles the tree-walking interpreter to `wasm32-unknown-unknown` and exposes a `Repl` type via `wasm-bindgen`.

## Status

Phase 12-ext — async browser REPL.  Targets `wasm32-unknown-unknown`; no AOT/IR compilation, no interop, no filesystem I/O.  Full `clojure.core.async` support via a Tokio `LocalSet` driven by `wasm-bindgen-futures`.

## File layout

| File | Description |
|------|-------------|
| `src/lib.rs` | `Repl` and `EvalResult` wasm-bindgen exports; bootstraps `standard_env_minimal`, initialises `cljrs-async`, drives a persistent `LocalSet` pump |
| `www/index.html` | Self-contained browser REPL UI (pure JS, no bundler required once wasm-pack output is present) |

## Public API

```rust
#[wasm_bindgen]
pub struct Repl;

impl Repl {
    /// Create a new REPL session. Initialises clojure.core.async and
    /// starts a persistent LocalSet pump so goroutines and channel tasks
    /// make progress between eval calls.
    pub fn new() -> Repl;

    /// Evaluate one or more Clojure forms asynchronously.
    /// Returns a JS Promise that resolves to an EvalResult.
    /// Top-level Future/Promise results are implicitly awaited.
    pub async fn eval(&self, input: String) -> EvalResult;
}

#[wasm_bindgen]
pub struct EvalResult;

impl EvalResult {
    pub fn output(&self) -> String;   // captured print/println output
    pub fn result(&self) -> String;   // pr-str of last value, or error message
    pub fn is_error(&self) -> bool;
}
```

From JavaScript, `eval` is an `async` method that returns a `Promise`:

```js
const repl = new Repl();
const r = await repl.eval("(require '[clojure.core.async :refer [chan go put! take!]])");
const r2 = await repl.eval(`
  (def c (chan 1))
  (go (put! c (* 6 7)))
  (await (take! c))
`);
console.log(r2.result); // "42"
```

## Building

```bash
# Install wasm-pack once
cargo install wasm-pack

# Build (outputs to crates/cljrs-wasm/pkg/)
wasm-pack build crates/cljrs-wasm --target web

# Serve the REPL
cp crates/cljrs-wasm/pkg/cljrs_wasm.js crates/cljrs-wasm/pkg/cljrs_wasm_bg.wasm crates/cljrs-wasm/www/
cd crates/cljrs-wasm/www && python3 -m http.server 8080
# Open http://localhost:8080
```

## What works

- All Clojure core forms: `def`, `defn`, `let`, `fn`, `if`, `do`, `loop/recur`, `try/catch`, macros, etc.
- Persistent collections: list, vector, map, set
- `print` / `println` / `prn` — output captured per eval call and returned in `EvalResult.output`
- `require` for built-in namespaces (`clojure.string`, `clojure.set`, etc.) loaded lazily on first use
- **Full `clojure.core.async`**: `^:async` functions, `await`, `chan`, `go`, `put!`, `take!`,
  `timeout`, `alts`, `alt`, `mult`/`tap!`, `join-all`, `async-pmap`, `thread`, etc.
- Top-level `await` is implicit: evaluating `(timeout 500)` at the REPL waits 500 ms and
  returns `nil` rather than an opaque future wrapper.
- Background goroutines and channel tasks persist across eval calls (driven by a long-lived
  `LocalSet` pump).

## What is intentionally excluded

- Versioned symbols (`name@commit`) — no git available in the browser
- `<!!` / `>!!` blocking ops — no OS threads in `wasm32-unknown-unknown`
- Filesystem I/O (`slurp`, `spit`, `load-file`)
- Rust interop (`cljrs-interop`, `#[export]`)
- AOT/IR compilation (`cljrs-compiler`, `cljrs-ir`)
