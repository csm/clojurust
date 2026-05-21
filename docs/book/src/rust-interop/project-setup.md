# Project Setup

A mixed Rust/Clojure project needs three things: a `cljrs.edn` that points to
the Rust crate, a `Cargo.toml` with the right crate type, and a `cljrs_init`
entry point.

## Directory layout

```
my-project/
├── cljrs.edn          # Clojure project config (source paths, :rust key)
├── Cargo.toml         # Rust crate manifest
├── src/
│   ├── lib.rs         # Rust source — defines cljrs_init and native fns
│   └── main.cljrs     # Clojure entry point
```

The Rust crate and the `cljrs.edn` file can live in the same directory (`:crate
"."`) or in a subdirectory (`:crate "native"`).

## `cljrs.edn`

Add a `:rust` map to the top-level config:

```clojure
{:paths ["src"]

 :rust {:crate "."                       ; path to Cargo.toml directory
        :init  "my_project::cljrs_init"} ; Rust path to the init function
}
```

| Key | Required | Description |
|---|---|---|
| `:crate` | yes | Path to the directory containing the user's `Cargo.toml`. Relative to `cljrs.edn`. |
| `:init` | yes | Fully-qualified Rust path to the init function, e.g. `"my_crate::cljrs_init"`. The first `::` segment is used as the crate name. |

## `Cargo.toml`

The user crate must be a library with `cdylib` output (for interpreter-mode
dynamic loading) and, optionally, `rlib` output (for AOT static linking):

```toml
[package]
name    = "my_project"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
cljrs-interop = { path = "/path/to/cljrs/crates/cljrs-interop" }
```

> **Note:** `cdylib` produces the `.so`/`.dylib`/`.dll` loaded by `cljrs run`.
> `rlib` allows `cljrs compile` to link the crate statically into the AOT
> binary. Both can coexist in `crate-type`.

## The `cljrs_init` entry point

The init function receives a `*mut Registry` pointer and registers all native
functions. It must have C linkage so the dynamic linker can find it by name:

```rust
use cljrs_interop::{Registry, wrap_fn1, wrap_fn2};

#[no_mangle]
pub extern "C" fn cljrs_init(registry: *mut Registry) {
    let r = unsafe { &mut *registry };

    r.define("my.project/greet",
        wrap_fn1("greet", |name: String| {
            Ok::<String, String>(format!("Hello, {name}!"))
        }));

    r.define("my.project/add",
        wrap_fn2("add", |a: i64, b: i64| Ok::<i64, String>(a + b)));
}
```

The function name in `:rust :init` (`"my_project::cljrs_init"`) must match the
Rust function name used in `#[no_mangle]` (`cljrs_init`). The crate prefix
(`my_project`) is used when generating the AOT harness; it must match the
`[package] name` in `Cargo.toml` with hyphens replaced by underscores.

## Calling native functions from Clojure

Native functions registered under `"my.project/greet"` are visible in Clojure
as `my.project/greet`. No `require` is needed unless you want a namespace alias:

```clojure
; Direct qualified call
(my.project/greet "world")       ; => "Hello, world!"

; With a require alias
(ns my.app
  (:require [my.project :as native]))

(native/add 3 4)                 ; => 7
```

The namespace `my.project` is created automatically when `cljrs_init` is called;
you do not need to create or load a Clojure file for it.
