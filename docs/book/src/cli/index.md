# Command-line tool

`cljrs` is the command-line entry point for clojurust. It provides subcommands
for running files, starting a REPL, evaluating expressions, compiling to native
binaries, running tests, and managing dependencies.

```
cljrs [GLOBAL OPTIONS] <SUBCOMMAND> [SUBCOMMAND OPTIONS]
```

## Subcommands

| Subcommand | Description |
|---|---|
| [`run`](run.md) | Interpret a `.cljrs` or `.cljc` source file |
| [`repl`](repl.md) | Start an interactive REPL |
| [`eval`](eval.md) | Evaluate a single expression and print the result |
| [`compile`](compile.md) | AOT-compile a source file to a native binary |
| [`test`](test.md) | Run `clojure.test` namespaces |
| [`deps`](deps.md) | Manage project dependencies declared in `cljrs.edn` |
| [`ir-viz`](ir-viz.md) | Render the optimised IR for a source file to HTML |

## Global options

These options are accepted by every subcommand.

### `--stack-size-mb <MB>`

Set the main thread's stack size in megabytes. Defaults to **64 MB**. Increase
this value if you encounter stack overflows in deeply recursive code.

```
cljrs --stack-size-mb 128 run my-program.cljrs
```

### `--debug`

Enable debug-level logging. Prints internal diagnostics to stderr.

### `--trace`

Enable trace-level logging (implies `--debug`). Much more verbose than `--debug`.

### `-X <LEVEL:FEATURES>`

Feature-scoped logging. Enables logging at `LEVEL` for the named comma-separated
`FEATURES` only.

```
cljrs -X debug:gc,reader run my-program.cljrs
cljrs -X trace:jit run my-program.cljrs
```

Available levels: `debug`, `trace`.
Available features: `gc`, `reader`, `jit`, and others.

### `--gc-stats [FILE]`

Print garbage-collector statistics on exit. Pass a file path to write the
report there; omit the path to write to stdout.

Only honoured by `run`, `eval`, and `test`.

```
cljrs --gc-stats run my-program.cljrs       # stats to stdout
cljrs --gc-stats gc.log run my-program.cljrs # stats to file
```

### `--verify-commit-signatures`

Require valid GPG or SSH signatures on every versioned commit before executing
historical code. Off by default. Can also be enabled per-project in `cljrs.edn`
via `:verify-commit-signatures true`.

## Project configuration: `cljrs.edn`

When any of `run`, `repl`, `compile`, or `test` starts, clojurust walks up the
directory tree from the current working directory and loads the nearest
`cljrs.edn` it finds. The `:paths` declared in that file are appended to the
source-path list (CLI `--src-path` values come first).

See [`deps`](deps.md) for the full format of `cljrs.edn`.
