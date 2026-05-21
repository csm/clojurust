# Interpreter Mode

In interpreter mode (`cljrs run` / `cljrs repl`), the Rust crate is compiled to
a shared library and loaded at startup via `dlopen`. No recompilation of
`cljrs` itself is required.

## Workflow

```
# 1. Build the shared library (once, or after Rust changes)
cljrs build-native

# 2. Run Clojure code — the .so is loaded automatically
cljrs run src/main.cljrs

# 3. Start the REPL — also auto-loads
cljrs repl
```

`cljrs build-native` is just `cargo build` run inside the crate directory
declared by `:rust :crate` in `cljrs.edn`. It inherits your normal Rust
toolchain, `RUSTFLAGS`, and `cargo` configuration.

## How auto-loading works

When `cljrs run` (or `repl`) starts:

1. It locates `cljrs.edn` by walking up the directory tree.
2. If `:rust` is present, it derives the library path from the crate name and
   profile (`debug` by default):
   - Linux: `<crate_dir>/target/debug/lib<crate_name>.so`
   - macOS: `<crate_dir>/target/debug/lib<crate_name>.dylib`
   - Windows: `<crate_dir>/target/debug/<crate_name>.dll`
3. It opens the library with `dlopen` and looks up the symbol named after the
   last `::` segment of `:rust :init` (e.g. `"cljrs_init"`).
4. It calls the symbol with a `*mut Registry`, which registers all native
   functions into the global namespace table.
5. The library is kept loaded for the lifetime of the process.

All of this happens before any Clojure source is evaluated, so native functions
are visible to `ns`/`require` forms and to top-level code.

## Missing library

If the shared library does not exist yet, `cljrs` prints a warning and
continues:

```
cljrs: native library not found at target/debug/libmy_project.so
       — run `cljrs build-native` first
```

Clojure code that calls unregistered native functions will get a runtime error;
code that doesn't call them runs normally. This is intentional: you can develop
pure-Clojure parts of a mixed project without running `cljrs build-native` on
every edit.

## Release builds

Pass `--release` to build an optimised library:

```
cljrs build-native --release
```

The release library is placed at `target/release/libmy_project.so` (and so on).
Note that `cljrs run` always looks for the **debug** build; to use the release
library you currently need to copy it to the debug path or symlink it. (A future
`--release` flag on `run`/`repl` will automate this.)

## Development tips

- **Fast iteration:** `cljrs build-native` compiles only the Rust crate, not the
  whole clojurust workspace. On a warm build cache it typically takes a few
  seconds.
- **Separate processes:** Because the library is opened once at startup,
  changes to Rust code require restarting `cljrs run`. There is no hot-reload
  of native code within a single process.
- **Cargo features:** Pass `CARGO_FLAGS` or set `[profile.*]` in your
  `Cargo.toml` as usual; `cljrs build-native` inherits the environment.
- **Debugging:** Use `RUST_LOG`, `RUST_BACKTRACE=1`, and your normal Rust
  debugging tools. The library is a standard shared object; `gdb`/`lldb`
  can attach to the `cljrs` process and set breakpoints in native code.
