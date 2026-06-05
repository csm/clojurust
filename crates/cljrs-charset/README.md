# cljrs-charset

**Purpose**: Charset encoding and decoding with stream support for clojurust — exposes the `clojure.rust.charset` and `clojure.rust.charset.async` namespaces, backed by [`encoding_rs`](https://github.com/nicowillis/encoding_rs) (the WHATWG encoding standard implementation).

**Status**: Phase 1 — implemented and tested.

## File layout

| File | Description |
|---|---|
| `src/lib.rs` | Crate entry point; `NS`/`NS_ASYNC` constants; `init()` and `init_async()`; unit tests |
| `src/codec.rs` | `CljDecoder` and `CljEncoder` — `NativeObject` wrappers around `encoding_rs::Decoder`/`Encoder` with `Mutex`-based interior mutability; shared decode/encode helpers |
| `src/fns.rs` | Clojure-callable builtin function implementations and registration for the sync namespace |
| `src/codec_chan.rs` | Async channel-to-channel transformers: `decode-chan` and `encode-chan` with their registration |

## Public API

### `init(globals: &Arc<GlobalEnv>)`

Register the `clojure.rust.charset` namespace.  Idempotent.

### `init_async(globals: &Arc<GlobalEnv>)`

Register the `clojure.rust.charset.async` namespace.  Idempotent.  Requires
`cljrs_async::init` and a running Tokio `LocalSet` before channel functions
are called.

### `NS: &str` / `NS_ASYNC: &str`

The namespace names: `"clojure.rust.charset"` and `"clojure.rust.charset.async"`.

## Clojure namespace: `clojure.rust.charset`

| Function | Arity | Description |
|---|---|---|
| `decoder` | 0–1 | `(decoder)` or `(decoder :shift-jis)` — create a streaming decoder. Optional charset keyword or string; defaults to UTF-8. |
| `encoder` | 0–1 | `(encoder)` or `(encoder :windows-1252)` — create a streaming encoder. |
| `update!` | 2 | Feed a chunk to a codec. `(update! dec bytes)` → string; `(update! enc string)` → byte-blob. |
| `finish!` | 1 | Flush and close a codec. `(finish! dec)` → string; `(finish! enc)` → byte-blob. After `finish!`, further calls return an error. |
| `decode` | 1–2 | One-shot decode: `(decode bytes)` or `(decode bytes :iso-8859-1)` → string. |
| `encode` | 1–2 | One-shot encode: `(encode string)` or `(encode string :shift-jis)` → byte-blob. |

## Clojure namespace: `clojure.rust.charset.async`

| Function | Arity | Description |
|---|---|---|
| `decode-chan` | 1–3 | `(decode-chan bytes-chan)` / `(decode-chan bytes-chan :shift-jis)` / `(decode-chan bytes-chan :shift-jis 16)` — reads `ByteBlob` values from the input channel, decodes them, and delivers strings onto a new output channel. Closed when the input closes. |
| `encode-chan` | 1–3 | `(encode-chan strings-chan)` / `(encode-chan strings-chan :windows-1252)` / `(encode-chan strings-chan :windows-1252 16)` — reads strings from the input channel, encodes them, and delivers `ByteBlob` values onto a new output channel. |

### Async conventions

- Both functions **return the output channel immediately**; a producer task drives the transform in the background.
- The default output buffer is 8 items; the producer yields whenever the consumer hasn't drained it (natural backpressure).
- `Value::Nil` on the input channel signals closure; the producer flushes any partial codec state (e.g. a split multi-byte sequence) and closes the output.
- Non-blob / non-string values — including `Value::Error` — are forwarded to the output channel unchanged so consumers can detect upstream errors.

### Charset labels

Any label recognised by `encoding_rs` is accepted as a keyword or string:
`:utf-8`, `:utf-16le`, `:shift-jis`, `:windows-1252`, `:iso-8859-1`, etc.
`nil` or omitted defaults to UTF-8.

### Unmappable characters

When encoding to a non-Unicode charset, unmappable characters are replaced with
HTML numeric character references (e.g. `&#128512;` for 😀).

### Example

```clojure
(require '[clojure.rust.charset :as charset])
(require '[clojure.rust.charset.async :as ca])
(require '[clojure.core.async :as async])

;; Synchronous streaming decode
(let [dec (charset/decoder :shift-jis)]
  (charset/update! dec chunk1)   ;; => "..."
  (charset/update! dec chunk2)   ;; => "..."
  (charset/finish! dec))         ;; => "tail"

;; One-shot
(charset/decode my-bytes :windows-1252)  ;; => "Hello World"
(charset/encode "こんにちは" :shift-jis) ;; => #bytes[...]

;; Async pipeline: byte-chan from network → decode → process strings
(let [bytes-ch  (network/read-chan conn)
      strings-ch (ca/decode-chan bytes-ch :utf-8)]
  (async/go-loop []
    (when-let [s (async/<! strings-ch)]
      (process! s)
      (recur))))

;; Async encode pipeline
(let [in-ch  (async/chan 8)
      out-ch (ca/encode-chan in-ch :shift-jis)]
  (async/onto-chan! in-ch ["Hello" " " "世界"])
  ;; out-ch delivers ByteBlob values
  )
```
