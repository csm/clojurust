# AOT Mode

When `cljrs compile` detects a `:rust` key in `cljrs.edn`, it statically links
the user's Rust crate into the generated binary. The compiled binary is fully
self-contained: no shared library or `cljrs` installation is needed at runtime.

## How it works

`cljrs compile` generates a temporary Cargo harness project that:

1. Depends on the clojurust runtime crates (`cljrs-stdlib`, `cljrs-eval`, etc.)
2. Also depends on the user's Rust crate (via `path = "<crate_dir>"`)
3. Also depends on `cljrs-interop` (for `Registry`)
4. Contains a generated `main.rs` that:
   a. Initialises the standard environment
   b. Creates a `Registry` and calls the user's init function
   c. Evaluates the Clojure preamble (interpreted forms: `ns`, `require`, `defmacro`, …)
   d. Calls the AOT-compiled `__cljrs_main` function

The harness is compiled with `cargo build --release` and the resulting binary
is copied to the output path.

## Generated `main.rs` (simplified)

```rust
fn main() {
    cljrs_compiler::rt_abi::anchor_rt_symbols();

    let globals = cljrs_stdlib::standard_env();

    // Native init — registered before any Clojure code runs
    let mut registry = cljrs_interop::Registry::new(globals.clone());
    my_project::cljrs_init(&mut registry);

    let mut env = cljrs_eval::Env::new(globals, "user");
    cljrs_env::callback::push_eval_context(&env);

    // Interpreted preamble (ns, require, defmacro, …)
    // …

    // AOT-compiled body
    unsafe { __cljrs_main() };
}
```

## Crate type requirement

For AOT static linking the user crate must produce an `rlib` (the default for
library crates). If you also need interpreter-mode dynamic loading, declare
both:

```toml
[lib]
crate-type = ["cdylib", "rlib"]
```

If you only need AOT (no `cljrs run` native loading), `crate-type = ["rlib"]`
is sufficient.

## Building an AOT binary

```
# Build once (debug or release)
cljrs build-native          # optional — only needed for cljrs run

# Compile to a native binary
cljrs compile src/main.cljrs --out myapp

# Run anywhere — no cljrs required
./myapp
```

The `cljrs compile` step does **not** require a pre-built `.so`; it statically
links the Rust crate directly. You do not need to run `cljrs build-native`
before `cljrs compile`.

## Native functions in the preamble

Because the native init call happens before the interpreted preamble is
evaluated, native functions are visible to `ns`, `require`, macros, and
`defprotocol`/`extend-type` forms that run at startup:

```clojure
(ns my.app
  (:require [my.project]))   ; my.project namespace already populated

(defprotocol IWidget
  (render [w]))

(extend-type my_project.Widget IWidget   ; native type tag
  (render [w] (my.project/widget-render w)))
```

## Offline builds

The AOT harness uses `cargo build --release --offline` by default. All
dependencies (the clojurust crates and the user crate) must be resolvable
without network access. Point to local paths in `Cargo.toml` as shown in
[Project setup](project-setup.md).

## Memory management

AOT-compiled binaries use the **bump allocator** fast path: escape analysis
promotes short-lived, non-escaping objects into bump-allocated regions,
leaving the tracing GC to handle everything else. (Since JIT phase 10.5 the
same machinery also runs under `cljrs run`/`repl`/`eval`; AOT still sees the
most opportunities because the whole program is analyzed as one unit.) See
[Memory Management](../memory/index.md) and
[The bump allocator](../memory/bump-allocator.md) for details.
