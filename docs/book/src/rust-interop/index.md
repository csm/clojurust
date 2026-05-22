# Rust Interop

clojurust lets Clojure code call Rust functions with full type safety and GC
integration. The interop layer has two modes that work together:

- **Interpreter mode** — the Rust crate is compiled to a shared library
  (`.so`/`.dylib`/`.dll`) and loaded by `cljrs run`/`cljrs repl` at startup via
  `cljrs build-native`.
- **AOT mode** — `cljrs compile` statically links the Rust crate into the
  generated binary; the native init function is called before any Clojure code
  runs.

Both modes use the same API: a `Registry` object that maps Clojure-visible
names to Rust functions.

## When to use Rust interop

- Wrapping an existing Rust library (e.g. a database driver, image codec, or
  systems API) for use from Clojure.
- Hot paths where Clojure performance is insufficient.
- Exposing mutable or OS-level state (file descriptors, sockets, GPU buffers)
  as opaque `NativeObject` values that participate in protocol dispatch.

## Chapter overview

| Page | Contents |
|---|---|
| [Project setup](project-setup.md) | `cljrs.edn` config, Cargo setup, crate layout |
| [Registry API](registry.md) | `Registry`, `wrap_fn*`, type marshalling, `NativeObject` |
| [The `#[export]` macro](export-macro.md) | Zero-boilerplate function registration |
| [Interpreter mode](interpreter.md) | `cljrs build-native`, auto-loading, hot-reload workflow |
| [AOT mode](aot.md) | How `cljrs compile` wires native init into the binary |

## Quick example

**`cljrs.edn`:**
```clojure
{:paths ["src"]
 :rust  {:crate "."
         :init  "my_project::cljrs_init"}}
```

**`Cargo.toml` (user crate):**
```toml
[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
cljrs-interop = { path = "/path/to/cljrs/crates/cljrs-interop" }
```

**`src/lib.rs`:**
```rust
use cljrs_interop::{Registry, wrap_fn2};

#[no_mangle]
pub extern "C" fn cljrs_init(registry: *mut Registry) {
    let r = unsafe { &mut *registry };
    r.define("my.project/add",
        wrap_fn2("add", |a: i64, b: i64| Ok::<i64, String>(a + b)));
}
```

**`src/main.cljrs`:**
```clojure
(ns my.project.core
  (:require [my.project :as native]))

(println (native/add 3 4))   ; => 7
```

**Workflow:**
```
cljrs build-native            # compile lib → target/debug/libmy_project.so
cljrs run src/main.cljrs      # auto-loads the .so, then runs Clojure
```
