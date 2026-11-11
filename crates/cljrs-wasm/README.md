# cljrs-wasm

## Purpose

WebAssembly browser REPL for clojurust.  Compiles the tree-walking interpreter to `wasm32-unknown-unknown` and exposes a `Repl` type via `wasm-bindgen`.

## Status

Phase 12-ext — async browser REPL.  The `dom` module was the separate
`cljrs-dom` package until consolidation stage 5; `cljrs-wasm` was its only
consumer and it had no independent artifact, so it is a module now.  Targets `wasm32-unknown-unknown`; no AOT/IR compilation, no interop, no filesystem I/O.  Full `clojure.core.async` support via a Tokio `LocalSet` driven by `wasm-bindgen-futures`.

## File layout

| File | Description |
|------|-------------|
| `src/lib.rs` | `Repl` and `EvalResult` wasm-bindgen exports; bootstraps a `Runtime` in `ExecutionMode::TreeWalk`, initialises `cljrs-async` and `dom`, drives a persistent `LocalSet` pump |
| `src/dom/mod.rs` | `dom` module root: the `DOM_GLOBALS` thread-local, `set_globals()`, `register()` |
| `src/dom/node.rs` | `DomNode` — wraps `web_sys::Node` as a `Value::NativeObject` |
| `src/dom/events.rs` | Event-to-map conversion, `DomListener`, `DomEventChan` |
| `src/dom/fns.rs` | All `cljrs.dom` native functions and their registration |
| `www/index.html` | Self-contained browser REPL UI (pure JS, no bundler required once wasm-pack output is present) |
| `tests/node-smoke.cjs` | Node smoke test of a `--target nodejs` build: evaluates forms that force a GC cycle and use `time` (run in CI) |

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

To smoke-test the wasm build without a browser (as CI does):

```bash
wasm-pack build crates/cljrs-wasm --target nodejs --release --out-dir pkg-node
node crates/cljrs-wasm/tests/node-smoke.cjs crates/cljrs-wasm/pkg-node
```

Code linked into this crate must not call `std::time::Instant::now` or
`SystemTime::now`: on `wasm32-unknown-unknown` they panic ("time not
implemented on this platform") and trap the instance. Use `web_time` instead.

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

## `dom` — the browser DOM API

`Repl::new` calls `dom::set_globals` and `dom::register`, so every session has
the `cljrs.dom` namespace bound.  The function bodies are
`#[cfg(target_arch = "wasm32")]`; on a native build `register` is a no-op and
the namespace is empty, which is how the workspace compiles and tests this
crate outside a browser target.

```rust
pub mod dom {
    /// Install the `GlobalEnv` used by DOM event callbacks that fire outside
    /// the normal eval context.  Must be called before any eval.
    pub fn set_globals(globals: Arc<GlobalEnv>);
    /// Register every `cljrs.dom` native function into `globals`.
    pub fn register(globals: &Arc<GlobalEnv>);
}
```

## The `cljrs.dom` namespace

### Selection
```clojure
(dom/document)         ; => DomNode (the document itself)
(dom/body)             ; => DomNode
(dom/head)             ; => DomNode
(dom/by-id "id")       ; => DomNode | nil
(dom/query "css")      ; => DomNode | nil  (querySelector)
(dom/query-all "css")  ; => [DomNode ...]  (querySelectorAll)
```

### Creation
```clojure
(dom/create "div")            ; => DomNode
(dom/create-text "hello")     ; => DomNode (text node)
(dom/create-ns ns "tag")      ; => DomNode (createElementNS, e.g. SVG)
```

### Tree manipulation
```clojure
(dom/append!        parent child)            ; => parent
(dom/prepend!       parent child)            ; => parent
(dom/insert-before! parent child ref-or-nil) ; => parent
(dom/remove!        el)                      ; => nil
(dom/replace!       old new)                 ; => nil
(dom/parent         el)                      ; => DomNode | nil
(dom/children       el)                      ; => [DomNode ...]
(dom/child-at       el idx)                  ; => DomNode | nil  (O(1), unlike `children`)
(dom/child-count    el)                      ; => Long
(dom/connected?     el)                      ; => boolean  (Node.isConnected)
```

### Attributes
```clojure
(dom/attr            el "name")             ; => String | nil
(dom/set-attr!       el "name" val)         ; => el
(dom/remove-attr!    el "name")             ; => el
(dom/set-attr-ns!    el ns "name" val)      ; => el  (setAttributeNS, e.g. xlink:href)
(dom/remove-attr-ns! el ns "name")          ; => el
```

### Classes
```clojure
(dom/add-class!    el "name") ; => el
(dom/remove-class! el "name") ; => el
(dom/has-class?    el "name") ; => boolean
(dom/toggle-class! el "name") ; => el
```

### Content
```clojure
(dom/text      el)      ; => String  (textContent)
(dom/set-text! el str)  ; => el
(dom/html      el)      ; => String  (innerHTML)
(dom/set-html! el str)  ; => el
```

### Style & form values
```clojure
(dom/style          el "prop")       ; => String
(dom/set-style!     el "prop" val)   ; => el
(dom/remove-style!  el "prop")       ; => el  (style.removeProperty)
(dom/computed-style el "prop")       ; => String  (getComputedStyle(el).getPropertyValue)
(dom/value          el)              ; => String  (input/select/textarea)
(dom/set-value!     el val)          ; => el
(dom/set-checked!   el bool)         ; => el  (HtmlInputElement.checked property)
(dom/set-selected!  el bool)         ; => el  (HtmlOptionElement.selected property)
(dom/set-prop!      el "name" val)   ; => el  (generic DOM property setter, via Reflect.set)
(dom/get-prop       el "name")       ; => String | Double | boolean | nil
```

### Events
```clojure
; Managed callback — returns a DomListener that keeps the handler alive
; opts is an optional map: {:capture bool :passive bool :once bool}
(dom/listen!   el "click" handler-fn)       ; => DomListener
(dom/listen!   el "click" handler-fn opts)  ; => DomListener
(dom/unlisten! listener)                    ; => nil  (removes handler immediately)

; Channel-based — returns a core.async channel; listener is leaked
(dom/event-chan el "input")           ; => channel
```

### Scheduling
```clojure
(dom/request-animation-frame f) ; => Long (request id); calls (f) on the next frame
```

### Node memory
```clojure
(dom/remember! node value) ; => node  (associate an arbitrary value with a node)
(dom/recall    node)       ; => value | nil
```
Identity is tracked via an expando id stamped onto the node; unlike a true
`WeakMap`, entries are not released when the node itself becomes unreachable.

Event maps delivered to callbacks:
```clojure
{:type        "click"
 :target      <DomNode>
 :bubbles     true
 :cancelable  true
 :prevent-default  #<NativeFn>  ; call ((:prevent-default event)) to cancel
 :stop-propagation #<NativeFn>
 ;; MouseEvent extras:
 :client-x 0  :client-y 0  :button 0
 ;; KeyboardEvent extras:
 :key "Enter"  :code "Enter"
 :ctrl-key false  :alt-key false  :shift-key false  :meta-key false}
```

### Hiccup renderer
```clojure
(dom/render! parent
  [:div {:id "app" :class "container"}
    [:h1 {} "Hello"]
    [:p  {:style {:color "blue"}} "World"]
    [:button {:on-click (fn [_] (println "clicked!"))} "Click me"]])
; => parent  (all existing children replaced)
```

`:style` map values set individual CSS properties. `:on-*` attributes attach event listeners (closure leaked — no handle returned). Children may be strings, nested hiccup vectors, or `DomNode` values.

---

## Features

| Feature | Default | Effect |
|---|---|---|
| `regex-full` | **on** | Forwards `regex-full` to this crate's workspace dependencies — `Value::Pattern` uses the `regex` crate. |
| `small-regex` | off | Forwards `small-regex` instead: `regex-lite`, which trades Unicode character classes for ~277 KB of text. |
| `deps` | **on** | Pass-through for `cljrs-runtime/deps` — git-backed dependency and versioned-var support. |

Every workspace dependency of this crate is taken with default features off (see
the note in the root `Cargo.toml`), so these pass-throughs are what put back what
those crates' defaults used to provide. `default` enables all of them, so a plain
build is unchanged.

`regex-full` wins when both regex features are enabled, so selecting the small
engine means turning default features off **on this crate** and re-adding what
you want:

```toml
cljrs-wasm = { version = "0.1", default-features = false, features = ["small-regex"] }
```

Adding a second, direct dependency on `cljrs-runtime` with
`default-features = false` would not undo it — Cargo unions features across every
edge to a package, so one edge left at its defaults re-enables `regex-full` for
the whole graph. `deps` has to be off as well for the size win to land, since
`cljrs-project/vcs` pulls `regex` in through `pgp`. See
[cljrs-value's README](../cljrs-value/README.md#features).
