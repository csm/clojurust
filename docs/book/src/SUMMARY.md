# Summary

[Introduction](introduction.md)

# The CLI

- [Command-line tool](cli/index.md)
  - [run](cli/run.md)
  - [repl](cli/repl.md)
  - [eval](cli/eval.md)
  - [compile](cli/compile.md)
  - [test](cli/test.md)
  - [deps](cli/deps.md)
  - [build-native](cli/build-native.md)
  - [trust](cli/trust.md)
  - [ir](cli/ir.md)

# The Language

- [Language overview](language/index.md)
- [Reader conditionals](language/reader-conditionals.md)
- [Versioned symbols](language/versioned-symbols.md)
- [Differences from Clojure](language/differences.md)
- [New built-in functions](language/builtins.md)

# Async & I/O

- [Overview](async-io/index.md)
- [core.async](async-io/async.md)
- [Worker isolation](async-io/isolation.md)
- [Asynchronous I/O](async-io/io.md)
- [Charset encoding](async-io/charset.md)
- [Networking](async-io/net.md)

# Rust Interop

- [Overview](rust-interop/index.md)
- [Project setup](rust-interop/project-setup.md)
- [Registry API](rust-interop/registry.md)
- [The `#[export]` macro](rust-interop/export-macro.md)
- [Interpreter mode](rust-interop/interpreter.md)
- [AOT mode](rust-interop/aot.md)

# Embedding

- [Embedding clojurust](embedding/index.md)
- [The runtime builder](embedding/runtime-builder.md)
- [Execution modes](embedding/execution-modes.md)
- [Extensions & native code](embedding/extensions.md)
- [Evaluating code](embedding/evaluating.md)
- [Limits & sandboxing](embedding/sandboxing.md)

# Memory Management

- [Overview](memory/index.md)
- [The bump allocator](memory/bump-allocator.md)
- [JIT & tiered execution](memory/jit.md)

# WebAssembly

- [Overview](wasm/index.md)
- [The AOT backend](wasm/aot-backend.md)
