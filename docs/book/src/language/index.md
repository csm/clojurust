# Language overview

clojurust is a dialect of Clojure. Its syntax, data model, and core library are
designed to be as compatible with Clojure as possible, with a small number of
deliberate extensions and a small number of features that are not yet
implemented.

## What is the same

- All Clojure literal syntax: symbols, keywords, numbers (long, double, ratio,
  BigInt, BigDecimal), strings, characters, booleans, nil.
- All collection literals: list `(...)`, vector `[...]`, map `{...}`, set `#{...}`.
- Reader dispatch macros: `'`, `` ` ``, `~`, `~@`, `^`, `@`, `#'`, `#(...)`,
  `#"..."`, `##Inf`, `##-Inf`, `##NaN`, tagged literals.
- The full set of special forms: `def`, `fn*`, `if`, `do`, `let*`, `loop*`,
  `recur`, `quote`, `var`, `set!`, `throw`, `try`/`catch`/`finally`, `letfn`,
  `binding`.
- Persistent collections (HAMT-backed maps and sets, RRB vectors, linked lists,
  queues) with Clojure-compatible equality and hashing.
- `clojure.core` — arithmetic, comparison, collection operations, lazy
  sequences, transducers, I/O, concurrency primitives.
- Standard library namespaces: `clojure.string`, `clojure.set`, `clojure.test`,
  `clojure.walk`, `clojure.edn`, `clojure.zip`, `clojure.data`.
- Protocols (`defprotocol`, `extend-type`, `extend-protocol`), multimethods
  (`defmulti`, `defmethod`), records (`defrecord`), and `reify`.
- Concurrency: `atom`, `future`, `promise`, `delay`, `volatile!`, `agent`.
- Dynamic variables (`binding`, `with-bindings`, `*ns*`, `*out*`, etc.).
- Metadata on vars and some values (`with-meta`, `meta`, `^:dynamic`, etc.).

## Extensions

- [Reader conditionals](reader-conditionals.md) — the `:rust` platform key.
- [Versioned symbols](versioned-symbols.md) — `my-fn@abc1234` syntax.
- A small set of [new built-in functions](builtins.md) with no Clojure
  equivalent.

## Known differences and missing features

See [Differences from Clojure](differences.md) for the full list.
