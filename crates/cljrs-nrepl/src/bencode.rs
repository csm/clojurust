//! Minimal bencode codec for the nREPL wire protocol.
//!
//! nREPL messages are bencode dictionaries; the protocol only ever uses the
//! four core bencode types (integers, byte strings, lists, dictionaries), so
//! this hand-rolled codec stays deliberately small instead of pulling in a
//! BitTorrent-oriented dependency.
//!
//! Decoding is incremental: a message that is still in flight on the TCP
//! stream decodes to `Ok(None)` ("need more bytes"), which is what the
//! per-connection read loop uses for framing.

use std::collections::BTreeMap;

/// A bencode value. Dictionary keys are byte strings sorted lexicographically
/// (`BTreeMap` gives us canonical encoding order for free).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bencode {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<Bencode>),
    Dict(BTreeMap<Vec<u8>, Bencode>),
}

impl Bencode {
    /// Byte string from UTF-8 text (the common case for nREPL).
    pub fn str(s: impl AsRef<str>) -> Self {
        Bencode::Bytes(s.as_ref().as_bytes().to_vec())
    }

    /// View a byte string as UTF-8 text.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Bencode::Bytes(b) => std::str::from_utf8(b).ok(),
            _ => None,
        }
    }

    pub fn as_dict(&self) -> Option<&BTreeMap<Vec<u8>, Bencode>> {
        match self {
            Bencode::Dict(d) => Some(d),
            _ => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BencodeError {
    #[error("invalid bencode: {0}")]
    Invalid(&'static str),
}

// ── Encoding ──────────────────────────────────────────────────────────────────

pub fn encode(v: &Bencode, out: &mut Vec<u8>) {
    match v {
        Bencode::Int(i) => {
            out.push(b'i');
            out.extend_from_slice(i.to_string().as_bytes());
            out.push(b'e');
        }
        Bencode::Bytes(b) => {
            out.extend_from_slice(b.len().to_string().as_bytes());
            out.push(b':');
            out.extend_from_slice(b);
        }
        Bencode::List(items) => {
            out.push(b'l');
            for item in items {
                encode(item, out);
            }
            out.push(b'e');
        }
        Bencode::Dict(entries) => {
            out.push(b'd');
            for (k, val) in entries {
                encode(&Bencode::Bytes(k.clone()), out);
                encode(val, out);
            }
            out.push(b'e');
        }
    }
}

pub fn encode_to_vec(v: &Bencode) -> Vec<u8> {
    let mut out = Vec::new();
    encode(v, &mut out);
    out
}

// ── Decoding ──────────────────────────────────────────────────────────────────

/// Internal parse error: distinguishes "buffer ends mid-value" (recoverable —
/// wait for more bytes) from genuinely malformed input.
enum ParseError {
    Incomplete,
    Invalid(&'static str),
}

/// Decode one bencode value from the front of `buf`.
///
/// Returns `Ok(Some((value, consumed)))` on success, `Ok(None)` when `buf`
/// holds only a prefix of a value (read more bytes and retry), and `Err` when
/// the input cannot be valid bencode no matter what bytes follow.
pub fn decode(buf: &[u8]) -> Result<Option<(Bencode, usize)>, BencodeError> {
    let mut pos = 0;
    match parse_value(buf, &mut pos) {
        Ok(v) => Ok(Some((v, pos))),
        Err(ParseError::Incomplete) => Ok(None),
        Err(ParseError::Invalid(msg)) => Err(BencodeError::Invalid(msg)),
    }
}

fn parse_value(buf: &[u8], pos: &mut usize) -> Result<Bencode, ParseError> {
    match buf.get(*pos) {
        None => Err(ParseError::Incomplete),
        Some(b'i') => parse_int(buf, pos),
        Some(b'l') => {
            *pos += 1;
            let mut items = Vec::new();
            loop {
                if buf.get(*pos) == Some(&b'e') {
                    *pos += 1;
                    return Ok(Bencode::List(items));
                }
                items.push(parse_value(buf, pos)?);
            }
        }
        Some(b'd') => {
            *pos += 1;
            let mut entries = BTreeMap::new();
            loop {
                if buf.get(*pos) == Some(&b'e') {
                    *pos += 1;
                    return Ok(Bencode::Dict(entries));
                }
                let key = match parse_value(buf, pos)? {
                    Bencode::Bytes(k) => k,
                    _ => return Err(ParseError::Invalid("dict key must be a byte string")),
                };
                let val = parse_value(buf, pos)?;
                entries.insert(key, val);
            }
        }
        Some(b'0'..=b'9') => parse_bytes(buf, pos),
        Some(_) => Err(ParseError::Invalid("unexpected type marker")),
    }
}

fn parse_int(buf: &[u8], pos: &mut usize) -> Result<Bencode, ParseError> {
    let start = *pos + 1; // skip 'i'
    let mut end = start;
    loop {
        match buf.get(end) {
            None => return Err(ParseError::Incomplete),
            Some(b'e') => break,
            Some(b'-') if end == start => end += 1,
            Some(b'0'..=b'9') => end += 1,
            Some(_) => return Err(ParseError::Invalid("non-digit in integer")),
        }
    }
    let text = std::str::from_utf8(&buf[start..end])
        .map_err(|_| ParseError::Invalid("non-UTF-8 integer"))?;
    let n: i64 = text
        .parse()
        .map_err(|_| ParseError::Invalid("integer out of range or empty"))?;
    *pos = end + 1; // consume 'e'
    Ok(Bencode::Int(n))
}

fn parse_bytes(buf: &[u8], pos: &mut usize) -> Result<Bencode, ParseError> {
    let start = *pos;
    let mut colon = start;
    loop {
        match buf.get(colon) {
            None => return Err(ParseError::Incomplete),
            Some(b':') => break,
            Some(b'0'..=b'9') => colon += 1,
            Some(_) => return Err(ParseError::Invalid("non-digit in string length")),
        }
    }
    let len: usize = std::str::from_utf8(&buf[start..colon])
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or(ParseError::Invalid("bad string length"))?;
    let data_start = colon + 1;
    let data_end = data_start
        .checked_add(len)
        .ok_or(ParseError::Invalid("string length overflow"))?;
    if buf.len() < data_end {
        return Err(ParseError::Incomplete);
    }
    *pos = data_end;
    Ok(Bencode::Bytes(buf[data_start..data_end].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(v: &Bencode) {
        let bytes = encode_to_vec(v);
        let (decoded, consumed) = decode(&bytes).unwrap().unwrap();
        assert_eq!(&decoded, v);
        assert_eq!(consumed, bytes.len());
    }

    #[test]
    fn roundtrip_scalars() {
        roundtrip(&Bencode::Int(0));
        roundtrip(&Bencode::Int(-42));
        roundtrip(&Bencode::Int(i64::MAX));
        roundtrip(&Bencode::str(""));
        roundtrip(&Bencode::str("hello"));
        roundtrip(&Bencode::Bytes(vec![0, 255, 128]));
    }

    #[test]
    fn roundtrip_nested() {
        let mut dict = BTreeMap::new();
        dict.insert(b"op".to_vec(), Bencode::str("eval"));
        dict.insert(b"code".to_vec(), Bencode::str("(+ 1 2)"));
        dict.insert(
            b"status".to_vec(),
            Bencode::List(vec![Bencode::str("done"), Bencode::Int(7)]),
        );
        dict.insert(b"nested".to_vec(), Bencode::Dict(dict.clone()));
        roundtrip(&Bencode::Dict(dict));
    }

    #[test]
    fn encoded_form_matches_spec() {
        assert_eq!(encode_to_vec(&Bencode::Int(42)), b"i42e");
        assert_eq!(encode_to_vec(&Bencode::str("spam")), b"4:spam");
        let mut dict = BTreeMap::new();
        dict.insert(b"a".to_vec(), Bencode::Int(1));
        assert_eq!(encode_to_vec(&Bencode::Dict(dict)), b"d1:ai1ee");
    }

    #[test]
    fn incomplete_input_returns_none() {
        for full in [
            encode_to_vec(&Bencode::Int(12345)),
            encode_to_vec(&Bencode::str("hello world")),
            {
                let mut d = BTreeMap::new();
                d.insert(b"key".to_vec(), Bencode::List(vec![Bencode::str("v")]));
                encode_to_vec(&Bencode::Dict(d))
            },
        ] {
            for cut in 0..full.len() {
                assert!(
                    decode(&full[..cut]).unwrap().is_none(),
                    "prefix of length {cut} should be incomplete"
                );
            }
        }
    }

    #[test]
    fn decode_leaves_trailing_bytes() {
        let mut bytes = encode_to_vec(&Bencode::Int(1));
        bytes.extend_from_slice(b"i2e");
        let (v, consumed) = decode(&bytes).unwrap().unwrap();
        assert_eq!(v, Bencode::Int(1));
        assert_eq!(consumed, 3);
        let (v2, _) = decode(&bytes[consumed..]).unwrap().unwrap();
        assert_eq!(v2, Bencode::Int(2));
    }

    #[test]
    fn invalid_input_errors() {
        assert!(decode(b"x").is_err());
        assert!(decode(b"i4x2e").is_err());
        assert!(decode(b"d3:key").unwrap().is_none()); // incomplete, not invalid
        assert!(decode(b"di1ei2ee").is_err()); // non-string dict key
    }
}
