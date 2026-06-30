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

### `--target <TARGET>`

Select the code-generation backend. Defaults to `native` (a Cranelift native
binary). `wasm` emits a WebAssembly module instead — see
[Targeting WebAssembly](#targeting-webassembly).

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

## Targeting WebAssembly

```
cljrs compile src/myapp/core.cljrs --target wasm -o myapp.wasm
```

With `--target wasm` the compiler runs the same IR pipeline but emits a
`.wasm` module — the entry namespace and every lowerable required namespace
bundled together — instead of a native binary. The emitted module is validated
with `wasmparser`. `--target wasm` cannot be combined with `--test` (there is no
wasm test harness yet).

> **Status: code generation complete; runtime linking in progress.** The emitted
> module imports its runtime bridge, linear memory, and function table from a
> `"rt"` module that the wasm runtime must satisfy — that linking step, and
> wiring the IR interpreter in as the dynamic-code tier, is not yet done. See the
> [WebAssembly](../wasm/index.md) chapter.

## Notes

AOT compilation is based on Cranelift (native) or `wasm-encoder` (wasm). Not all
language features are yet supported in AOT mode; in particular, features that rely
on dynamic dispatch or late binding may fall back to interpreted execution within
the compiled program.

## Native Rust code

If `cljrs.edn` contains a `:rust` key, `cljrs compile` links the declared Rust
crate into the binary and calls its `cljrs_init` function before any Clojure
code runs. See [AOT mode](../rust-interop/aot.md) for details.
