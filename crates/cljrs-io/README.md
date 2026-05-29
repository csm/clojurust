# cljrs-io

Asynchronous file I/O for clojurust — tokio-backed reads and writes whose
results are delivered over `clojure.core.async` channels.

**Phase:** runtime I/O (Phase 7+) — initial implementation.

## Purpose

Expose the host filesystem to Clojure code as a non-blocking, channel-oriented
API. Every operation runs on the shared Tokio `LocalSet` executor (the same one
`cljrs-async` drives) and returns a `clojure.core.async` channel, so file I/O
composes with `go`, `<!`, `alts!`, and the rest of the async toolkit instead of
blocking the interpreter thread.

The crate reuses `cljrs-async`'s `CljChannel` rather than defining its own
channel type, so the channels it returns are ordinary core.async channels.

## Channel shape: raw vs. promise

The API deliberately uses **two** channel shapes, chosen per operation — this is
the recommendation that came out of the design discussion:

- **Streaming reads → a raw channel.** `chunk-chan`, `byte-chan`, `char-chan`,
  and `line-chan` return a channel *immediately* and spawn a producer task that
  reads the file and puts a sequence of values onto it, closing it at EOF. The
  channel's small buffer (`cap`, default 8) bounds how far the producer reads
  ahead of the consumer, so a multi-gigabyte file is streamed, never slurped.
  This is where channels earn their keep: a sequence + backpressure.

- **Discrete request/response ops → a promise channel.** `slurp`, `slurp-bytes`,
  `read-bytes`, and `spit` return a capacity-1 channel onto which the producer
  delivers exactly one result before closing it. A single `(<! ...)` yields the
  value; a second take yields `nil`. Returning a one-shot channel (rather than a
  streaming one) signals "this resolves once" while staying uniformly
  `<!`-able with everything else.

A pure-`Future`/promise-only API was rejected because it forces re-calling for
streamed data and provides no backpressure; a raw-channel-only API was rejected
because it muddies the "resolves exactly once" contract of discrete ops.

### Errors

Failures are delivered **in band**: the producer puts a `Value::Error` (an
exception value) on the channel and then closes it. Consumers distinguish
results from failures with the `error?` helper:

```clojure
(require '[clojure.core.async :refer [<!!]]
         '[clojure.rust.io.async :as aio])

(let [result (<!! (aio/slurp "config.edn"))]
  (if (aio/error? result)
    (println "read failed:" (ex-message result))
    (println result)))
```

## Status

Implemented (initial): the eight builtins below plus the `error?`/`ok?` Clojure
helpers. Not yet covered (candidate follow-ups): a stateful `AsyncReader` handle
with a cursor (`open` / `read-chunk!` / `seek`), append/options maps for `spit`,
directory streaming, and transducer-equipped channels.

## File layout

| File | Description |
|---|---|
| `src/lib.rs` | `init(globals)` entry point; registers builtins and loads the namespace; re-exports `charset` and `fs` modules. |
| `src/fs.rs` | The native builtins (streaming + discrete) and their channel/value/argument helpers. |
| `src/charset.rs` | Charset-label resolution via `encoding_rs` and the incremental `CharDecoder` used by `char-chan`/`line-chan`. |
| `src/clojure_rust_io_async.cljrs` | Clojure-level helpers (`error?`, `ok?`) loaded at init. |

## Public API (Rust)

- `cljrs_io::init(globals: &Arc<GlobalEnv>)` — register and load the
  `clojure.rust.io.async` namespace. Idempotent. Requires `cljrs_async::init`
  and a running `LocalSet`.
- `cljrs_io::NS: &str` — the namespace name, `"clojure.rust.io.async"`.
- `fs::register(globals, ns)` — register only the native functions into `ns`.
- `charset::resolve_charset(arg: Option<&Value>) -> ValueResult<&'static Encoding>`
  — resolve a keyword/string charset label (default UTF-8).
- `charset::CharDecoder` — incremental byte→`char` decoder (`new`, `push`,
  `finish`).

## Public API (Clojure — `clojure.rust.io.async`)

Streaming reads (raw channel, closed at EOF):

| Function | Yields |
|---|---|
| `(chunk-chan path [buf-size [cap]])` | `byte-array` chunks of up to `buf-size` bytes (default 8192). |
| `(byte-chan path [cap])` | individual bytes as signed `long`s (-128..127). |
| `(char-chan path [charset [cap]])` | characters decoded with `charset` (default `:utf-8`). |
| `(line-chan path [charset [cap]])` | lines (without trailing `\n`/`\r\n`). |

Discrete ops (promise channel — one value, then closed):

| Function | Delivers |
|---|---|
| `(slurp path [charset])` | the whole file as a decoded string. |
| `(slurp-bytes path)` | the whole file as a `byte-array`. |
| `(read-bytes path n)` | a `byte-array` of up to the first `n` bytes. |
| `(spit path data [charset])` | the number of bytes written (`data` is a string or `byte-array`). |

Helpers: `(error? x)`, `(ok? x)`.

`charset` is a keyword or string label resolved by `encoding_rs` (`:utf-8`,
`:utf-16le`, `:iso-8859-1`, `:windows-1252`, `:shift_jis`, …).

## Dependencies

`cljrs-types`, `cljrs-gc`, `cljrs-value`, `cljrs-env`, `cljrs-reader`,
`cljrs-interp`, `cljrs-async` (channels + `spawn_future`), `tokio` (with the
`fs` and `io-util` features), and `encoding_rs`.
