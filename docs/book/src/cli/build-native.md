# cljrs build-native

Compile the project's embedded Rust crate to a shared library so that
`cljrs run` and `cljrs repl` can load native functions at startup.

```
cljrs build-native [--release]
```

This command reads the `:rust` key from the nearest `cljrs.edn`, runs
`cargo build` inside the declared crate directory, and prints the path of the
resulting library to stdout.

## Options

### `--release`

Build in release mode (`cargo build --release`) instead of the default debug
mode. Use this when profiling or shipping.

## What it does

1. Locates `cljrs.edn` by walking up the directory tree.
2. Reads `:rust :crate` (the directory containing the user's `Cargo.toml`)
   and `:rust :init` (the fully-qualified Rust path to the init function).
3. Derives the crate name from the first `::` segment of the init path —
   e.g. `"my_project::cljrs_init"` → `my_project`.
4. Runs `cargo build [--release]` in the crate directory.
5. Prints the output library path on success:
   - Linux: `target/debug/libmy_project.so`
   - macOS: `target/debug/libmy_project.dylib`
   - Windows: `target/debug/my_project.dll`

## Auto-loading

Once the library exists, `cljrs run` and `cljrs repl` load it automatically on
startup — no flags required. If the library is absent (not yet built or
deleted), a warning is printed but the interpreter continues; calls to
unregistered native functions will produce a runtime error.

## Example

```
# Build the native library
cljrs build-native

# Now run Clojure code that calls native functions
cljrs run src/main.cljrs
```

See [Rust Interop](../rust-interop/index.md) for a complete walkthrough.
