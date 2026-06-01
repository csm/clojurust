# cljrs-charset

**Purpose**: Charset encoding and decoding with stream support for clojurust — exposes the `clojure.rust.charset` namespace, backed by [`encoding_rs`](https://github.com/nicowillis/encoding_rs) (the WHATWG encoding standard implementation).

**Status**: Phase 1 — implemented and tested.

## File layout

| File | Description |
|---|---|
| `src/lib.rs` | Crate entry point; `NS` constant; `init()` registers the namespace; unit tests |
| `src/codec.rs` | `CljDecoder` and `CljEncoder` — `NativeObject` wrappers around `encoding_rs::Decoder`/`Encoder` with `Mutex`-based interior mutability |
| `src/fns.rs` | Clojure-callable builtin function implementations and registration |

## Public API

### `init(globals: &Arc<GlobalEnv>)`

Register the `clojure.rust.charset` namespace.  Idempotent: subsequent calls are no-ops.

### `NS: &str`

The namespace name: `"clojure.rust.charset"`.

## Clojure namespace: `clojure.rust.charset`

| Function | Arity | Description |
|---|---|---|
| `decoder` | 0–1 | `(decoder)` or `(decoder :shift-jis)` — create a streaming decoder. Optional charset keyword or string; defaults to UTF-8. |
| `encoder` | 0–1 | `(encoder)` or `(encoder :windows-1252)` — create a streaming encoder. |
| `update!` | 2 | Feed a chunk to a codec. `(update! dec bytes)` → string; `(update! enc string)` → byte-blob. Returns accumulated output for this chunk. |
| `finish!` | 1 | Flush and close a codec. `(finish! dec)` → string; `(finish! enc)` → byte-blob. After `finish!`, further calls return an error. |
| `decode` | 1–2 | One-shot decode: `(decode bytes)` or `(decode bytes :iso-8859-1)` → string. |
| `encode` | 1–2 | One-shot encode: `(encode string)` or `(encode string :shift-jis)` → byte-blob. |

### Charset labels

Any label recognised by `encoding_rs` is accepted as a keyword or string:
`:utf-8`, `:utf-16le`, `:shift-jis`, `:windows-1252`, `:iso-8859-1`, etc.
`nil` or omitted defaults to UTF-8.

### Unmappable characters

When encoding to a non-Unicode charset, characters that cannot be represented
are replaced with HTML numeric character references (e.g. `&#128512;` for 😀).

### Example

```clojure
(require '[clojure.rust.charset :as charset])

;; Streaming decode (e.g. reading from a network stream in chunks)
(let [dec (charset/decoder :shift-jis)]
  (charset/update! dec chunk1)   ;; => "..."
  (charset/update! dec chunk2)   ;; => "..."
  (charset/finish! dec))         ;; => "tail"

;; One-shot
(charset/decode my-bytes :windows-1252)   ;; => "Hello World"
(charset/encode "こんにちは" :shift-jis) ;; => #bytes[...]
```
