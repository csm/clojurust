# Asynchronous I/O

The `clojure.rust.io.async` namespace exposes the host filesystem to Clojure as
a **non-blocking, channel-oriented** API. Every operation runs on the same Tokio
`LocalSet` executor that drives [core.async](async.md) and returns a
`clojure.core.async` channel, so file I/O composes with `go`, `take!`, `alts`,
and the rest of the async toolkit instead of blocking the interpreter thread.

It is provided by the `cljrs-io` crate and loaded automatically by the `cljrs`
CLI (with its default-on `async` feature).

```clojure
(require '[clojure.rust.io.async :as aio]
         '[clojure.core.async :refer [take!]])
```

The channels it returns are ordinary core.async channels — `cljrs-io` reuses
`cljrs-async`'s `CljChannel` rather than defining its own type.

## Two channel shapes

The API deliberately uses two channel shapes, chosen per operation:

- **Streaming reads return a raw channel.** The call returns *immediately* and a
  background producer task reads the file and puts a sequence of values onto the
  channel, closing it at EOF. A small buffer (`cap`, default 8) bounds how far
  the producer reads ahead of the consumer, so even a multi-gigabyte file is
  streamed with backpressure rather than slurped into memory.

- **Discrete request/response ops return a promise channel.** The call returns a
  capacity-1 channel onto which the producer delivers exactly one result before
  closing it. A single `take!` yields the value; a second yields `nil`. This
  signals "resolves exactly once" while staying uniformly takeable alongside
  everything else.

## Streaming reads

Each returns a channel immediately and closes it at EOF:

| Function | Yields |
|---|---|
| `(chunk-chan path [buf-size [cap]])` | `byte-array` chunks of up to `buf-size` bytes (default 8192) |
| `(byte-chan path [cap])` | individual bytes as signed `long`s (-128..127) |
| `(char-chan path [charset [cap]])` | characters decoded with `charset` (default `:utf-8`) |
| `(line-chan path [charset [cap]])` | lines, without the trailing `\n` / `\r\n` |

```clojure
(go (loop []
      (when-let [line (await (take! (line-chan "big.log")))]
        (println line)
        (recur))))
```

## Discrete operations

Each returns a one-shot promise channel carrying a single result:

| Function | Delivers |
|---|---|
| `(slurp path [charset])` | the whole file as a decoded string |
| `(slurp-bytes path)` | the whole file as a `byte-array` |
| `(read-bytes path n)` | a `byte-array` of up to the first `n` bytes |
| `(spit path data [charset])` | the number of bytes written (`data` is a string or `byte-array`) |

```clojure
(go (let [text (await (take! (slurp "config.edn")))]
      (println text)))
```

## Error handling

Failures are delivered **in band**: the producer puts an error value (an
exception) onto the channel and then closes it. Consumers distinguish results
from failures with the `error?` / `ok?` helpers:

```clojure
(require '[clojure.core.async :refer [<!!]]
         '[clojure.rust.io.async :as aio])

(let [result (<!! (aio/slurp "config.edn"))]
  (if (aio/error? result)
    (println "read failed:" (ex-message result))
    (println result)))
```

> **Top-level consumption.** From `cljrs repl`, `cljrs run`, or `cljrs eval`,
> consume results with `(await (take! ...))`. The blocking `<!!` / `>!!` ops
> deadlock the single-threaded executor at the top level — they are for use off
> the executor thread (separate test threads, embedders), not for top-level CLI
> forms.

## Charsets

The `charset` argument is a keyword or string label resolved by `encoding_rs`,
defaulting to UTF-8:

```clojure
(aio/slurp "data.txt" :utf-8)
(aio/char-chan "legacy.txt" :windows-1252)
(aio/line-chan "jp.txt" :shift_jis)
```

Supported labels include `:utf-8`, `:utf-16le`, `:iso-8859-1`, `:windows-1252`,
`:shift_jis`, and the rest of the `encoding_rs` set.

## Status and scope

The eight builtins above plus the `error?` / `ok?` helpers are implemented.
Candidate follow-ups not yet covered: a stateful `AsyncReader` handle with a
cursor (`open` / `read-chunk!` / `seek`), append/options maps for `spit`,
directory streaming, and transducer-equipped channels.

## Embedding from Rust

`cljrs-io::init` registers the namespace; it is idempotent and requires
`cljrs_async::init` and a running `LocalSet`:

```rust
rt.block_on(local.run_until(async {
    let globals = cljrs_stdlib::standard_env();
    cljrs_async::init(&globals);   // required first
    cljrs_io::init(&globals);
    // ... evaluate code ...
}));
```
