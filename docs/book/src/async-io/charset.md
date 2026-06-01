# Charset encoding and decoding

The `clojure.rust.charset` namespace provides streaming charset encoding and
decoding backed by [`encoding_rs`](https://github.com/nicowillis/encoding_rs),
which implements the WHATWG Encoding Standard. The companion namespace
`clojure.rust.charset.async` wraps the same codecs as channel-to-channel
transformers that compose naturally with the rest of the async I/O stack.

Both are provided by the `cljrs-charset` crate and loaded automatically by the
`cljrs` CLI.

```clojure
(require '[clojure.rust.charset :as charset])
(require '[clojure.rust.charset.async :as ca])
```

## Charset labels

Any label recognized by `encoding_rs` is accepted as a keyword or string:

```clojure
:utf-8   :utf-16le   :shift-jis   :windows-1252   :iso-8859-1   ...
```

`nil` or an omitted argument defaults to UTF-8. The full list of accepted
labels is defined by the WHATWG Encoding Standard.

## Streaming codecs

Streaming codecs handle input that arrives in pieces — TCP segments, buffered
reads — without buffering the whole input first.

### Creating a codec

```clojure
(charset/decoder)             ; => CljDecoder — UTF-8
(charset/decoder :shift-jis)  ; => CljDecoder — Shift-JIS

(charset/encoder)              ; => CljEncoder — UTF-8
(charset/encoder :windows-1252) ; => CljEncoder — Windows-1252
```

### Feeding chunks

```clojure
(charset/update! dec chunk)  ; => string  — decoded output so far
(charset/update! enc string) ; => byte-blob — encoded bytes so far
```

`update!` may return an empty string or empty byte-blob if the chunk ended on a
multibyte boundary; the partial state is held inside the codec.

### Flushing

```clojure
(charset/finish! dec)  ; => string  — any remaining decoded characters
(charset/finish! enc)  ; => byte-blob — any remaining encoded bytes
```

After `finish!`, the codec is closed and further calls return an error.

### Full example

```clojure
;; Decode a Shift-JIS file streamed in two chunks
(let [dec (charset/decoder :shift-jis)]
  (println (charset/update! dec chunk1))
  (println (charset/update! dec chunk2))
  (println (charset/finish! dec)))
```

## One-shot helpers

When the entire input is available at once, the one-shot helpers are more
convenient:

| Function | Signature | Returns |
|---|---|---|
| `decode` | `(decode bytes)` / `(decode bytes charset)` | decoded string |
| `encode` | `(encode string)` / `(encode string charset)` | byte-blob |

```clojure
(charset/decode my-bytes :windows-1252)  ; => "Hello World"
(charset/encode "こんにちは" :shift-jis) ; => #bytes[...]
```

## Unmappable characters

When encoding to a non-Unicode charset, characters that have no representation
are replaced with HTML numeric character references:

```
😀  →  &#128512;
```

## Async channel transformers

`clojure.rust.charset.async` wraps the streaming codecs as channel-to-channel
transformers. Each function returns an output channel immediately; a background
producer task drives the conversion.

| Function | Signature | Input | Output |
|---|---|---|---|
| `decode-chan` | `(decode-chan bytes-chan [charset [buf]])` | `ByteBlob` values | `string` values |
| `encode-chan` | `(encode-chan strings-chan [charset [buf]])` | `string` values | `ByteBlob` values |

The third argument sets the output channel buffer depth (default 8). The
producer yields whenever the consumer has not drained the buffer, applying
natural backpressure back to the upstream source.

### Closure and errors

- Closing the input channel (or putting `nil`) signals the end of the stream.
  The producer flushes any partial codec state — for example a split multibyte
  sequence — and then closes the output channel.
- Non-blob / non-string values, including `Value::Error`, are forwarded to the
  output channel unchanged so consumers can detect upstream failures.

### Example: network decode pipeline

```clojure
(require '[clojure.rust.net :as net])
(require '[clojure.rust.charset.async :as ca])
(require '[clojure.core.async :refer [go <! close!]])

;; byte-chan from a TCP connection → decode UTF-8 → process strings
(let [conn      (await (take! (net/connect {:host "example.com" :port 80})))
      strings-ch (ca/decode-chan (:in conn) :utf-8)]
  (go (loop []
        (when-let [s (<! strings-ch)]
          (process! s)
          (recur)))
      (close! (:out conn))))
```

### Example: encode and send

```clojure
;; Encode strings as Shift-JIS and write to a connection
(let [in-ch  (async/chan 8)
      out-ch (ca/encode-chan in-ch :shift-jis)]
  (async/onto-chan! in-ch ["Hello" " " "世界"])
  ;; out-ch delivers ByteBlob values ready to put on (:out conn)
  )
```

## Embedding from Rust

`cljrs_charset::init` registers the sync namespace; `init_async` registers the
channel-based namespace. Both are idempotent. `init_async` requires
`cljrs_async::init` and a running Tokio `LocalSet`.

```rust
cljrs_async::init(&globals);        // required before init_async
cljrs_charset::init(&globals);
cljrs_charset::init_async(&globals);
```

The namespace name constants are also exported for use in attribute maps or
`require` calls from Rust:

```rust
cljrs_charset::NS        // "clojure.rust.charset"
cljrs_charset::NS_ASYNC  // "clojure.rust.charset.async"
```
