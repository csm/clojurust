//! Streaming codec wrappers around `encoding_rs::Decoder` and `encoding_rs::Encoder`.
//!
//! Both types implement [`NativeObject`] so they can be stored as
//! `Value::NativeObject` and passed to Clojure-level functions.
//! Interior mutability is provided by `Mutex` so that the codec can be updated
//! through a shared `&GcPtr<NativeObjectBox>` reference.

use std::{any::Any, fmt, sync::Mutex};

use cljrs_gc::{MarkVisitor, Trace};
use cljrs_value::{NativeObject, ValueError, ValueResult};
use encoding_rs::Encoding;

// ── Decoder ───────────────────────────────────────────────────────────────────

/// A streaming byte-to-string decoder backed by `encoding_rs`.
///
/// Feed byte chunks via [`CljDecoder::update`]; flush trailing state via
/// [`CljDecoder::finish`], which consumes the inner decoder so the object
/// cannot be used afterwards.
pub struct CljDecoder {
    // Option is None after finish!; held inside Mutex for interior mutability.
    inner: Mutex<Option<encoding_rs::Decoder>>,
    encoding_name: &'static str,
}

// SAFETY: `encoding_rs::Decoder` contains only `&'static Encoding` (a static
// lookup table) plus plain value-type state.  Moving it across threads is safe;
// the Mutex ensures exclusive access to the mutable state.
unsafe impl Send for CljDecoder {}
unsafe impl Sync for CljDecoder {}

impl CljDecoder {
    pub fn new(encoding: &'static Encoding) -> Self {
        Self {
            inner: Mutex::new(Some(encoding.new_decoder())),
            encoding_name: encoding.name(),
        }
    }

    /// Decode a non-final byte chunk, returning the decoded text produced.
    ///
    /// Returns an error if the decoder was already finished.
    pub fn update(&self, src: &[u8]) -> ValueResult<String> {
        let mut guard = self.inner.lock().unwrap();
        let dec = guard
            .as_mut()
            .ok_or_else(|| ValueError::Other("decoder is already finished".into()))?;
        Ok(decode_bytes(dec, src, false))
    }

    /// Flush any buffered trailing bytes and mark the decoder as finished.
    ///
    /// Returns the remaining decoded text (may be empty).  Calling again
    /// after finish returns an error.
    pub fn finish(&self) -> ValueResult<String> {
        let mut guard = self.inner.lock().unwrap();
        let mut dec = guard
            .take()
            .ok_or_else(|| ValueError::Other("decoder is already finished".into()))?;
        Ok(decode_bytes(&mut dec, &[], true))
    }
}

/// Decode `src` into a new `String`.
///
/// `max_utf8_buffer_length` pre-sizes the buffer so that `decode_to_string`
/// completes in a single call without reallocation.
pub(crate) fn decode_bytes(dec: &mut encoding_rs::Decoder, src: &[u8], last: bool) -> String {
    let cap = dec
        .max_utf8_buffer_length(src.len())
        .unwrap_or_else(|| src.len().saturating_mul(4).max(4));
    let mut out = String::with_capacity(cap);
    let _ = dec.decode_to_string(src, &mut out, last);
    out
}

impl fmt::Debug for CljDecoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Decoder({})", self.encoding_name)
    }
}

impl Trace for CljDecoder {
    fn trace(&self, _: &mut MarkVisitor) {}
}

impl NativeObject for CljDecoder {
    fn type_tag(&self) -> &str {
        "Decoder"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ── Encoder ───────────────────────────────────────────────────────────────────

/// A streaming string-to-bytes encoder backed by `encoding_rs`.
///
/// Feed string chunks via [`CljEncoder::update`]; flush via
/// [`CljEncoder::finish`].  Unmappable characters are replaced with HTML
/// numeric character references (`&#NNNN;`).
pub struct CljEncoder {
    inner: Mutex<Option<encoding_rs::Encoder>>,
    encoding_name: &'static str,
}

// SAFETY: Same reasoning as CljDecoder — the Encoder contains only a static
// pointer plus value-type state; the Mutex guards mutable access.
unsafe impl Send for CljEncoder {}
unsafe impl Sync for CljEncoder {}

impl CljEncoder {
    pub fn new(encoding: &'static Encoding) -> Self {
        Self {
            inner: Mutex::new(Some(encoding.new_encoder())),
            encoding_name: encoding.name(),
        }
    }

    /// Encode a non-final string chunk, returning the encoded bytes produced.
    ///
    /// Returns an error if the encoder was already finished.
    pub fn update(&self, src: &str) -> ValueResult<Vec<u8>> {
        let mut guard = self.inner.lock().unwrap();
        let enc = guard
            .as_mut()
            .ok_or_else(|| ValueError::Other("encoder is already finished".into()))?;
        Ok(encode_str(enc, src, false))
    }

    /// Flush any pending state and mark the encoder as finished.
    ///
    /// Returns any trailing encoded bytes (may be empty).
    pub fn finish(&self) -> ValueResult<Vec<u8>> {
        let mut guard = self.inner.lock().unwrap();
        let mut enc = guard
            .take()
            .ok_or_else(|| ValueError::Other("encoder is already finished".into()))?;
        Ok(encode_str(&mut enc, "", true))
    }
}

/// Encode `src` into a `Vec<u8>`.
///
/// Unmappable characters are substituted with HTML numeric character
/// references (e.g. `&#12354;` for あ when encoding as Latin-1).  The loop
/// handles `OutputFull` by extending the output buffer.
pub(crate) fn encode_str(enc: &mut encoding_rs::Encoder, src: &str, last: bool) -> Vec<u8> {
    use encoding_rs::EncoderResult;

    let mut out = Vec::new();
    let mut remaining = src;

    loop {
        let space = enc
            .max_buffer_length_from_utf8_without_replacement(remaining.len().max(1))
            .unwrap_or_else(|| remaining.len().saturating_mul(4).max(8));
        let old_len = out.len();
        out.resize(old_len + space, 0u8);

        let is_last = last && remaining.is_empty();
        let (result, read, written) =
            enc.encode_from_utf8_without_replacement(remaining, &mut out[old_len..], is_last);
        out.truncate(old_len + written);
        remaining = &remaining[read..];

        match result {
            EncoderResult::InputEmpty => {
                if last && !is_last {
                    // All input consumed; now do the final flush call.
                    continue;
                }
                break;
            }
            EncoderResult::OutputFull => {} // loop with more buffer
            EncoderResult::Unmappable(ch) => {
                // encoding_rs already counted the unmappable char's bytes in
                // `read`, so `remaining` is already past it.  Just emit the
                // HTML numeric character reference substitution.
                let ncr = format!("&#{};", ch as u32);
                out.extend_from_slice(ncr.as_bytes());
            }
        }
    }

    out
}

impl fmt::Debug for CljEncoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Encoder({})", self.encoding_name)
    }
}

impl Trace for CljEncoder {
    fn trace(&self, _: &mut MarkVisitor) {}
}

impl NativeObject for CljEncoder {
    fn type_tag(&self) -> &str {
        "Encoder"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
