//! Charset resolution and streaming character decoding.
//!
//! The async character reader ([`crate::fs::builtin_char_chan`]) decodes a byte
//! stream into a sequence of `char`s using a caller-supplied charset. Charset
//! labels are resolved through [`encoding_rs`] — the WHATWG-standard, pure-Rust
//! decoder — so any label it recognises (`utf-8`, `utf-16le`, `iso-8859-1`,
//! `windows-1252`, `shift_jis`, …) is accepted, given as either a Clojure
//! keyword (`:utf-8`) or string (`"utf-8"`). A missing/`nil` charset is UTF-8.

use encoding_rs::{Decoder, Encoding};

use cljrs_value::{Value, ValueError, ValueResult};

/// Resolve an optional charset argument to an [`Encoding`].
///
/// Accepts a keyword or string label; `None`/`nil` defaults to UTF-8. Returns a
/// `WrongType`/`Other` error for non-label values or labels `encoding_rs` does
/// not recognise.
pub fn resolve_charset(arg: Option<&Value>) -> ValueResult<&'static Encoding> {
    let label = match arg {
        None | Some(Value::Nil) => return Ok(encoding_rs::UTF_8),
        Some(Value::Keyword(k)) => k.get().name.as_ref().to_string(),
        Some(Value::Str(s)) => s.get().clone(),
        Some(other) => {
            return Err(ValueError::WrongType {
                expected: "charset keyword or string",
                got: other.type_name().to_string(),
            });
        }
    };
    Encoding::for_label(label.as_bytes())
        .ok_or_else(|| ValueError::Other(format!("unknown charset: {label}")))
}

/// An incremental byte-to-`char` decoder wrapping an [`encoding_rs::Decoder`].
///
/// Bytes are fed one chunk at a time via [`Self::push`]; the trailing partial
/// code unit (if any) is buffered inside the decoder and completed by the next
/// chunk or flushed by [`Self::finish`]. Malformed input is replaced with the
/// Unicode replacement character (U+FFFD), matching `encoding_rs`'s
/// non-fatal decode contract.
pub struct CharDecoder {
    decoder: Decoder,
}

impl CharDecoder {
    pub fn new(encoding: &'static Encoding) -> Self {
        Self {
            decoder: encoding.new_decoder(),
        }
    }

    /// Decode `src` (a non-final chunk), returning the text produced so far.
    pub fn push(&mut self, src: &[u8]) -> String {
        self.decode(src, false)
    }

    /// Flush any buffered trailing bytes at end-of-stream.
    pub fn finish(&mut self) -> String {
        self.decode(&[], true)
    }

    fn decode(&mut self, src: &[u8], last: bool) -> String {
        // `max_utf8_buffer_length` sizes `out` so a single `decode_to_string`
        // call consumes all of `src`, so no re-loop is required.
        let cap = self
            .decoder
            .max_utf8_buffer_length(src.len())
            .unwrap_or(src.len().saturating_mul(4));
        let mut out = String::with_capacity(cap);
        let _ = self.decoder.decode_to_string(src, &mut out, last);
        out
    }
}
