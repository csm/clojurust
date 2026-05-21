# cljrs repl

Start an interactive read–eval–print loop.

```
cljrs repl [OPTIONS]
```

The REPL reads one expression at a time, evaluates it, and prints the result.
Multi-line input is supported: the REPL continues reading until all open
brackets are closed before evaluating.

Type `:quit` or press `Ctrl-D` to exit.

## Options

### `--src-path <DIR>`

Add `DIR` to the source-path list for `require`. May be repeated.

### `--gc-soft-limit-mb <MB>` / `--gc-hard-limit-mb <MB>`

GC memory limits. See [`run`](run.md) for details.

## REPL behaviour

- The initial namespace is `user`.
- The standard library (`clojure.core` and friends) is pre-loaded.
- `*1`, `*2`, `*3` hold the last three non-nil results.
- `*e` holds the last exception.
- `nil` results are printed as `nil`.

## Line editing

When clojurust is built with the `enable-rustyline` feature, the REPL uses
[rustyline](https://github.com/kkawakam/rustyline) for line editing, including
history and readline-style key bindings.

Without that feature, a simpler line reader is used that still supports
multi-line input but has no history or key bindings.

## Example session

```
$ cljrs repl
clojurust REPL (type :quit to exit)

=> (+ 1 2)
3
=> (def x 42)
=> x
42
=> (map inc [1 2 3])
(2 3 4)
=> :quit
Bye.
```
