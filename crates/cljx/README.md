# cljx

The `cljx` binary — command-line interface for running, compiling, and
interactively exploring clojurust programs.

**Phase:** 1 (CLI scaffold) — argument parsing and subcommand dispatch are
implemented; all subcommand bodies are stubs pending downstream crates.

---

## File layout

```
src/
  main.rs   — CLI entry point: Clap structs, miette error hook, subcommand dispatch
```

---

## CLI reference

```
cljx <SUBCOMMAND>

Subcommands:
  run      <file> [--src-path DIR]...   Interpret a .cljrs or .cljc source file
  repl     [--src-path DIR]...          Start an interactive REPL
  compile  <file> -o <out>              AOT-compile a source file to a native binary
  eval     <expr>                       Evaluate a single Clojure expression and print the result
```

`--src-path` may be repeated to add multiple source directories searched by
`require` when resolving namespace names to files.

### Examples

```bash
cljx run hello.cljrs
cljx run main.cljrs --src-path src --src-path lib
cljx repl --src-path src
cljx compile app.cljrs -o app
cljx eval '(+ 1 2)'
```

---

## Implementation notes

- Argument parsing uses [Clap](https://docs.rs/clap) derive macros (`Parser`,
  `Subcommand`).
- The miette error hook is installed at startup so that any `CljxError`
  propagated to `main` renders with terminal-linked source snippets.
- All subcommand handlers currently print `[not yet implemented]` to stderr;
  they will be filled in as `cljx-reader`, `cljx-eval`, `cljx-compiler`, and
  `cljx-runtime` reach their respective phases.

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljx-types` (workspace) | `CljxError` — error type for `miette::Result` propagation |
| `cljx-gc` (workspace) | GC (initialisation at startup, future phases) |
| `cljx-reader` (workspace) | Lexer + parser (future phases) |
| `cljx-eval` (workspace) | Interpreter (future phases) |
| `cljx-runtime` (workspace) | Standard library (future phases) |
| `cljx-compiler` (workspace) | JIT/AOT (future phases) |
| `cljx-interop` (workspace) | Rust interop (future phases) |
| `clap` (workspace) | CLI argument parsing |
| `miette` (workspace) | Rich terminal error rendering |
