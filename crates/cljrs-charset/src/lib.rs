//! Charset encoding and decoding with stream support — `clojure.rust.charset`.
//!
//! Provides the `clojure.rust.charset` namespace backed by [`encoding_rs`],
//! which supports the WHATWG encoding standard (UTF-8, UTF-16, Shift-JIS,
//! windows-1252, ISO-8859-*, and many more).
//!
//! # Clojure API
//!
//! ```clojure
//! ;; Streaming decode
//! (let [dec (charset/decoder :shift-jis)]
//!   (charset/update! dec some-bytes)   ;; => "partial string"
//!   (charset/finish! dec))             ;; => "tail string"
//!
//! ;; Streaming encode
//! (let [enc (charset/encoder :windows-1252)]
//!   (charset/update! enc "Hello")      ;; => #bytes[...]
//!   (charset/finish! enc))             ;; => #bytes[]
//!
//! ;; One-shot helpers
//! (charset/decode blob)                ;; UTF-8 decode
//! (charset/decode blob :iso-8859-1)   ;; with explicit charset
//! (charset/encode "こんにちは" :shift-jis)
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! let globals = cljrs_stdlib::standard_env();
//! cljrs_charset::init(&globals);
//! ```

use std::sync::Arc;

use cljrs_env::env::GlobalEnv;

mod codec;
mod fns;

/// The Clojure namespace populated by this crate.
pub const NS: &str = "clojure.rust.charset";

/// Register the `clojure.rust.charset` namespace into `globals`.
///
/// Idempotent: the namespace is built only on the first call.
pub fn init(globals: &Arc<GlobalEnv>) {
    if globals.is_loaded(NS) {
        return;
    }
    globals.get_or_create_ns(NS);
    globals.refer_all(NS, "clojure.core");
    fns::register(globals, NS);
    globals.mark_loaded(NS);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::codec::{CljDecoder, CljEncoder};

    #[test]
    fn utf8_roundtrip_streaming() {
        let input = "Hello, 世界! こんにちは";

        let enc = CljEncoder::new(encoding_rs::UTF_8);
        let mut encoded = enc.update(input).unwrap();
        encoded.extend_from_slice(&enc.finish().unwrap());

        let dec = CljDecoder::new(encoding_rs::UTF_8);
        let mut decoded = dec.update(&encoded).unwrap();
        decoded.push_str(&dec.finish().unwrap());

        assert_eq!(decoded, input);
    }

    #[test]
    fn utf8_chunked_decode() {
        let bytes = "abcdef".as_bytes();
        let dec = CljDecoder::new(encoding_rs::UTF_8);
        let mut out = dec.update(&bytes[..3]).unwrap();
        out.push_str(&dec.update(&bytes[3..]).unwrap());
        out.push_str(&dec.finish().unwrap());
        assert_eq!(out, "abcdef");
    }

    #[test]
    fn utf8_multibyte_split_across_chunks() {
        // 'あ' = U+3042 encodes as 3 bytes: E3 81 82.
        // Split every possible way across two update! calls and verify the
        // decoder buffers the incomplete sequence internally and completes it
        // on the next call — the defining behaviour of a streaming decoder.
        let input = "あいう"; // 9 bytes total: [E3 81 82] [E3 81 84] [E3 81 86]
        let bytes = input.as_bytes();
        assert_eq!(bytes.len(), 9);

        for split in 1..bytes.len() {
            let dec = CljDecoder::new(encoding_rs::UTF_8);
            // First chunk may end mid-character; decoder must buffer the partial bytes.
            let part1 = dec.update(&bytes[..split]).unwrap();
            // Second chunk supplies the remaining bytes; decoder completes the char.
            let part2 = dec.update(&bytes[split..]).unwrap();
            let tail = dec.finish().unwrap();
            let decoded = part1 + &part2 + &tail;
            assert_eq!(
                decoded, input,
                "split at byte {split} produced wrong output"
            );
        }
    }

    #[test]
    fn utf8_split_finish_replaces_incomplete_sequence() {
        // If the stream ends with an incomplete multi-byte sequence, finish!
        // should replace the dangling bytes with U+FFFD (replacement character).
        let dec = CljDecoder::new(encoding_rs::UTF_8);
        // First two bytes of 'あ' (E3 81), no third byte — stream ends here.
        let partial = dec.update(&[0xE3, 0x81]).unwrap();
        assert_eq!(partial, ""); // nothing output yet; bytes are buffered
        let tail = dec.finish().unwrap();
        assert_eq!(
            partial + &tail,
            "\u{FFFD}",
            "incomplete trailing sequence must produce U+FFFD"
        );
    }

    #[test]
    fn finish_after_finish_returns_error() {
        let dec = CljDecoder::new(encoding_rs::UTF_8);
        dec.finish().unwrap();
        assert!(dec.finish().is_err());
    }

    #[test]
    fn update_after_finish_returns_error() {
        let enc = CljEncoder::new(encoding_rs::UTF_8);
        enc.finish().unwrap();
        assert!(enc.update("more").is_err());
    }

    #[test]
    fn latin1_encode_unmappable_uses_ncr() {
        // "café" — 'é' (U+00E9) is mappable in latin-1; emoji is not.
        let enc = CljEncoder::new(encoding_rs::WINDOWS_1252);
        let bytes = enc.update("A\u{1F600}B").unwrap();
        enc.finish().unwrap();
        // Byte for 'A', then NCR bytes, then byte for 'B'.
        let s = String::from_utf8(bytes).unwrap();
        assert!(s.starts_with('A'));
        assert!(s.contains("&#128512;")); // U+1F600 = 128512
        assert!(s.ends_with('B'));
    }
}
