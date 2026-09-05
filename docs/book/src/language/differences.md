# Differences from Clojure

This page documents intentional differences, missing features, and behavioural
variations between clojurust and Clojure (JVM). The goal is to be a useful
reference for porting `.cljc` code and for understanding what to expect when
running Clojure code under clojurust.

## No JVM, no Java interop

clojurust runs on Rust, not the JVM. There is no Java class hierarchy, no
`java.lang.*`, and no Java method calls. The `.` (dot) special form and `new`
have limited implementations:

- `(new Exception "msg")` and `(Exception. "msg")` produce clojurust
  exception values, not Java objects.
- `(.method obj args...)` is not yet implemented. Use protocols or the
  built-in equivalents instead.

Code that uses `(System/nanoTime)`, `(Thread/sleep n)`, or any other Java
static method must use [reader conditionals](reader-conditionals.md) to supply
a `:rust` alternative, or use the clojurust built-ins `nanotime` and `sleep`
respectively.

## Platform key

The reader-conditional platform key is `:rust`, not `:clj`. See
[Reader conditionals](reader-conditionals.md).

## Missing concurrency features

| Feature | Status |
|---|---|
| `ref` / STM (`dosync`, `alter`, `ref-set`, `commute`, `ensure`) | Not implemented |
| `locking` macro | Not implemented |
| `monitor-enter` / `monitor-exit` | Not implemented |

`atom`, `agent`, `future`, `promise`, `delay`, and `volatile!` are fully
implemented.

## `deftype`

`deftype` is implemented: a positional `->Name` constructor, inline
protocol/interface method bodies with the fields in scope, `.-field` access,
and mutable fields (`^:unsynchronized-mutable` / `^:volatile-mutable`) written
with `set!` — both `(set! field v)` inside a method and
`(set! (.-field inst) v)` from outside.

Instances are single-threaded here, so `^:volatile-mutable` and
`^:unsynchronized-mutable` behave identically. Unlike `defrecord`, `deftype`
generates no `map->Name` constructor and no map behaviour on the instance —
that matches Clojure. Host-class method calls (`(.someMethod inst)`) on an
instance are not supported; protocol methods are called as
`(proto-fn inst)`.

## Metadata on collections

Metadata is supported on vars and some values, but not yet propagated through
collection operations such as `assoc`. The `with-meta` and `meta` functions
work on any value that carries metadata.

## Sorted collections

`sorted-map` and `sorted-set` are implemented. `sorted-map-by` and
`sorted-set-by` (custom comparators) are not yet implemented.

## Hierarchies

`make-hierarchy`, `ancestors`, `descendants`, and `parents` have stub
implementations. `derive`, `underive`, and a full `isa?` hierarchy are not yet
implemented. `defmulti` / `defmethod` dispatch works but does not consult a
custom hierarchy.

## `amap` and `areduce`

These macros are registered as stubs but not fully implemented.

## `clojure.pprint`

Not implemented.

## `clojure.zip`

Stub — the namespace exists but most functions are not yet implemented.

## Numeric tower

The numeric tower (Long → BigInt → Ratio → BigDecimal → Double) follows
Clojure conventions. A few differences to be aware of:

- Integer overflow in `+`, `-`, `*` automatically promotes to `BigInt` (same
  as Clojure's `checked` arithmetic). The promoting variants `+'`, `-'`, `*'`
  are also available.
- `(/ 1 3)` returns a `Ratio` (`1/3`), not a `Double`, same as Clojure.
- BigDecimal precision is controlled by `with-precision` (the Clojure macro) or
  the lower-level `push-precision!` / `pop-precision!` built-ins.

## `*clojure-version*` / `*cljrs-version*`

These vars are not yet defined. `clojure-version` as a function returns a
map describing the current runtime.

## `with-open` and `close`

clojurust provides RAII-style resource management via `with-open` (a macro)
and `close` (a built-in function). These follow the same protocol as Clojure's
`with-open`; any value that implements the `Resource` protocol can be used.

## Source file namespace mapping

The namespace→file mapping converts `.` to `/` and `-` to `_`, the same as
Clojure:

```
myapp.core    →  myapp/core.cljrs  (or .cljc)
my-app.utils  →  my_app/utils.cljrs
```

## Standard library

The following `clojure.*` namespaces are available:
`clojure.string`, `clojure.set`, `clojure.test`, `clojure.walk`,
`clojure.edn`, `clojure.data`, `clojure.spec.alpha`,
`clojure.spec.test.alpha`, and `clojure.spec.gen.alpha`. `clojure.core.async`
is also available (as the `cljrs-async` crate — see the
[Async & I/O](../async-io/index.md) chapter).

`clojure.zip` and `clojure.pprint` exist as stubs. `clojure.spec.alpha` has
no generators: `clojure.spec.gen.alpha` and every generative-testing entry
point (`s/gen`, `s/exercise`, `s/exercise-fn`, `stest/check`) throw a clear
`ex-info` instead of generating values. `clojure.core.match` is not
available.
