# cljrs-dom

Clean DOM interaction API for WASM targets, exposed as the `cljrs.dom` namespace.

## Purpose

Provides idiomatic Clojure-flavoured wrappers around the browser DOM: kebab-case function names, `!`-suffixed mutations, events delivered as Clojure maps, and `core.async` channel support for event streams. Intended as an alternative to raw JS interop syntax for code running in the `cljrs-wasm` browser REPL.

## Status

Phase: WASM integration. Fully implemented; all DOM functions are active on `wasm32-unknown-unknown` builds. On other targets the namespace is registered but contains no functions.

## File layout

| File | Description |
|------|-------------|
| `src/lib.rs` | Crate root; `DOM_GLOBALS` thread-local, `set_globals()`, `register()` |
| `src/node.rs` | `DomNode` struct — wraps `web_sys::Node` as `Value::NativeObject` |
| `src/events.rs` | Event-to-map conversion, `DomListener`, `DomEventChan` |
| `src/fns.rs` | All `cljrs.dom` native functions and `register()` |

## Public API

### Wiring (called from `cljrs-wasm`)

```rust
cljrs_dom::set_globals(globals.clone());  // must be called before any eval
cljrs_dom::register(&globals);            // registers cljrs.dom namespace
```

### `cljrs.dom` namespace

#### Selection
```clojure
(dom/document)         ; => DomNode (the document itself)
(dom/body)             ; => DomNode
(dom/head)             ; => DomNode
(dom/by-id "id")       ; => DomNode | nil
(dom/query "css")      ; => DomNode | nil  (querySelector)
(dom/query-all "css")  ; => [DomNode ...]  (querySelectorAll)
```

#### Creation
```clojure
(dom/create "div")        ; => DomNode
(dom/create-text "hello") ; => DomNode (text node)
```

#### Tree manipulation
```clojure
(dom/append!  parent child) ; => parent
(dom/prepend! parent child) ; => parent
(dom/remove!  el)           ; => nil
(dom/replace! old new)      ; => nil
(dom/parent   el)           ; => DomNode | nil
(dom/children el)           ; => [DomNode ...]
```

#### Attributes
```clojure
(dom/attr         el "name")        ; => String | nil
(dom/set-attr!    el "name" val)    ; => el
(dom/remove-attr! el "name")        ; => el
```

#### Classes
```clojure
(dom/add-class!    el "name") ; => el
(dom/remove-class! el "name") ; => el
(dom/has-class?    el "name") ; => boolean
(dom/toggle-class! el "name") ; => el
```

#### Content
```clojure
(dom/text      el)      ; => String  (textContent)
(dom/set-text! el str)  ; => el
(dom/html      el)      ; => String  (innerHTML)
(dom/set-html! el str)  ; => el
```

#### Style & form values
```clojure
(dom/style      el "prop")       ; => String
(dom/set-style! el "prop" val)   ; => el
(dom/value      el)              ; => String  (input/select/textarea)
(dom/set-value! el val)          ; => el
```

#### Events
```clojure
; Managed callback — returns a DomListener that keeps the handler alive
(dom/listen!   el "click" handler-fn) ; => DomListener
(dom/unlisten! listener)              ; => nil  (removes handler immediately)

; Channel-based — returns a core.async channel; listener is leaked
(dom/event-chan el "input")           ; => channel
```

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

#### Hiccup renderer
```clojure
(dom/render! parent
  [:div {:id "app" :class "container"}
    [:h1 {} "Hello"]
    [:p  {:style {:color "blue"}} "World"]
    [:button {:on-click (fn [_] (println "clicked!"))} "Click me"]])
; => parent  (all existing children replaced)
```

`:style` map values set individual CSS properties. `:on-*` attributes attach event listeners (closure leaked — no handle returned). Children may be strings, nested hiccup vectors, or `DomNode` values.
