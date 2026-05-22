# cljrs compile

AOT-compile a source file to a standalone native binary.

```
cljrs compile [OPTIONS] <FILE> --out <OUT>
```

The compiler lowers the source file through the IR pipeline and emits a native
binary via Cranelift. The resulting binary statically links the clojurust
runtime, GC, and standard library; it has no runtime dependency on the `cljrs`
tool.

## Arguments

| Argument | Description |
|---|---|
| `<FILE>` | Source file to compile (or a directory when `--test` is used) |

## Required options

### `-o, --out <OUT>`

Output path for the compiled binary.

## Optional options

### `--src-path <DIR>`

Add `DIR` to the source path for `require` resolution. May be repeated.

### `--test`

Compile a test harness instead of a regular program. When this flag is set,
`FILE` should be a directory; the compiler discovers all `.cljrs`/`.cljc` files
in that directory and builds a binary that runs all `clojure.test` tests found
in them.

### `--gc-soft-limit-mb <MB>` / `--gc-hard-limit-mb <MB>`

GC memory limits baked into the compiled binary, not the compilation process
itself.

## Examples

```
# Compile a single file
cljrs compile src/myapp/core.cljrs --out myapp

# Compile and run
cljrs compile src/myapp/core.cljrs --out myapp && ./myapp

# Compile a test binary
cljrs compile --test test/ --out run-tests && ./run-tests
```

## Notes

AOT compilation is based on Cranelift. Not all language features are yet
supported in AOT mode; in particular, features that rely on dynamic dispatch
or late binding may fall back to interpreted execution within the compiled binary.

## Native Rust code

If `cljrs.edn` contains a `:rust` key, `cljrs compile` links the declared Rust
crate into the binary and calls its `cljrs_init` function before any Clojure
code runs. See [AOT mode](../rust-interop/aot.md) for details.
