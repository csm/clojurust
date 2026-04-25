# cljrs (Clojurust CLI)

The `cljrs` binary — command-line interface for running, compiling, and
interactively exploring clojurust programs.

---

## File layout

```
src/
  main.rs   — CLI entry point: Clap structs, miette error hook, subcommand dispatch
```

---

## CLI reference

```
cljrs <SUBCOMMAND>

Subcommands:
  run      Interpret a .cljrs or .cljc source file
  repl     Start an interactive REPL
  compile  AOT-compile a source file to a native binary
  eval     Evaluate a single Clojure expression and print the result
  test     Run clojure.test tests for one or more namespaces```

`--src-path` may be repeated to add multiple source directories searched by
`require` when resolving namespace names to files.

### Examples

```bash
cljrs run hello.cljrs
cljrs run main.cljrs --src-path src --src-path lib
cljrs repl --src-path src
cljrs compile app.cljrs -o app
cljrs eval '(+ 1 2)'
cljrs test --src-path src/ --src-path test/ my-ns.my-tests
```

---

## Implementation notes

- Argument parsing uses [Clap](https://docs.rs/clap) derive macros (`Parser`,
  `Subcommand`).
- The miette error hook is installed at startup so that any `CljxError`
  propagated to `main` renders with terminal-linked source snippets.
- All subcommand handlers currently print `[not yet implemented]` to stderr;
  they will be filled in as `cljrs-reader`, `cljrs-eval`, `cljrs-compiler`, and
  `cljrs-runtime` reach their respective phases.

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-types` (workspace) | `CljxError` — error type for `miette::Result` propagation |
| `cljrs-gc` (workspace) | GC (initialisation at startup, future phases) |
| `cljrs-reader` (workspace) | Lexer + parser (future phases) |
| `cljrs-eval` (workspace) | Interpreter (future phases) |
| `cljrs-runtime` (workspace) | Standard library (future phases) |
| `cljrs-compiler` (workspace) | JIT/AOT (future phases) |
| `cljrs-interop` (workspace) | Rust interop (future phases) |
| `clap` (workspace) | CLI argument parsing |
| `miette` (workspace) | Rich terminal error rendering |
