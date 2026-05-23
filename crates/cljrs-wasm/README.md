# cljrs-wasm

## Purpose

WebAssembly browser REPL for clojurust.  Compiles the tree-walking interpreter to `wasm32-unknown-unknown` and exposes a `Repl` type via `wasm-bindgen`.

## Status

Phase 12-ext — browser REPL.  Targets `wasm32-unknown-unknown`; no AOT/IR compilation, no interop, no threading, no filesystem I/O.

## File layout

| File | Description |
|------|-------------|
| `src/lib.rs` | `Repl` and `EvalResult` wasm-bindgen exports; bootstraps `standard_env_minimal` |
| `www/index.html` | Self-contained browser REPL UI (pure JS, no bundler required once wasm-pack output is present) |

## Public API

```rust
#[wasm_bindgen]
pub struct Repl;

impl Repl {
    pub fn new() -> Repl;
    pub fn eval(&self, input: &str) -> EvalResult;
}

#[wasm_bindgen]
pub struct EvalResult;

impl EvalResult {
    pub fn output(&self) -> String;   // captured print/println output
    pub fn result(&self) -> String;   // pr-str of last value, or error message
    pub fn is_error(&self) -> bool;
}
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

## What is intentionally excluded

- Versioned symbols (`name@commit`) — no git available in the browser
- `agent`, `future`, `promise` — no threads in `wasm32-unknown-unknown`
- Filesystem I/O (`slurp`, `spit`, `load-file`)
- Rust interop (`cljrs-interop`, `#[export]`)
- AOT/IR compilation (`cljrs-compiler`, `cljrs-ir`)
