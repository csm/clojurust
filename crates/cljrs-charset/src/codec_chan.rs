//! Async channel-to-channel charset transformers — `clojure.rust.charset.async`.
//!
//! `decode-chan` and `encode-chan` each accept an input channel and return a
//! new output channel immediately.  A producer task spawned via
//! `cljrs_async::spawn_future` drives values from input to output,
//! applying incremental charset decode/encode along the way.
//!
//! The same backpressure and error-passthrough conventions used by
//! `cljrs-io` apply here:
//! - A small output buffer (default 8) bounds memory; the producer yields
//!   whenever the consumer hasn't drained the buffer.
//! - `Value::Nil` from the input channel signals "input is closed".  The
//!   producer flushes any partial codec state, puts the tail (if non-empty),
//!   and closes the output.
//! - Non-ByteBlob / non-Str values (including `Value::Error`) are forwarded
//!   to the output channel unchanged.

use std::sync::{Arc, Mutex};

use cljrs_async::{
    channel::{chan_put as put, chan_ref, chan_take, make_chan},
    spawn_future,
};
use cljrs_env::env::GlobalEnv;
use cljrs_gc::GcPtr;
use cljrs_value::{Arity, NativeFn, NativeFnPtr, NativeObjectBox, Value, ValueError, ValueResult};

use crate::{
    codec::{decode_bytes, encode_str},
    fns::resolve_encoding,
};

/// Default output-channel buffer: the producer may stay this many items
/// ahead of the consumer before it blocks.
const DEFAULT_CAP: usize = 8;

// ── Registration ─────────────────────────────────────────────────────────────

pub fn register(globals: &Arc<GlobalEnv>, ns: &str) {
    let entries: &[(&str, Arity, NativeFnPtr)] = &[
        ("decode-chan", Arity::Variadic { min: 1 }, builtin_decode_chan),
        ("encode-chan", Arity::Variadic { min: 1 }, builtin_encode_chan),
    ];
    for (name, arity, func) in entries {
        let nf = NativeFn::new(*name, arity.clone(), *func);
        globals.intern(ns, Arc::from(*name), Value::NativeFunction(GcPtr::new(nf)));
    }
}

// ── Argument helpers ──────────────────────────────────────────────────────────

/// Extract a channel `GcPtr` from the argument list, validating the type.
fn channel_arg(args: &[Value], idx: usize) -> ValueResult<GcPtr<NativeObjectBox>> {
    match &args[idx] {
        Value::NativeObject(ch) if ch.get().type_tag() == "Channel" => Ok(ch.clone()),
        Value::NativeObject(ch) => Err(ValueError::WrongType {
            expected: "channel",
            got: ch.get().type_tag().to_string(),
        }),
        other => Err(ValueError::WrongType {
            expected: "channel",
            got: other.type_name().to_string(),
        }),
    }
}

fn cap_arg_or(args: &[Value], idx: usize, default: usize) -> ValueResult<usize> {
    match args.get(idx) {
        None | Some(Value::Nil) => Ok(default),
        Some(Value::Long(n)) if *n >= 0 => Ok(*n as usize),
        Some(Value::Long(n)) => Err(ValueError::Other(format!(
            "channel capacity must be non-negative, got {n}"
        ))),
        Some(other) => Err(ValueError::WrongType {
            expected: "integer capacity",
            got: other.type_name().to_string(),
        }),
    }
}

// ── decode-chan ───────────────────────────────────────────────────────────────

/// `(decode-chan bytes-chan)` /
/// `(decode-chan bytes-chan :shift-jis)` /
/// `(decode-chan bytes-chan :shift-jis 16)`
///
/// Reads `ByteBlob` values from `bytes-chan`, decodes them with the given
/// charset (default UTF-8), and delivers decoded strings onto a new output
/// channel.  The output channel is closed when the input closes.
///
/// Non-ByteBlob values (e.g. `Value::Error`) are forwarded to the output
/// channel without modification so callers can detect upstream errors with
/// the standard `error?` check.
///
/// Requires the `cljrs-async` runtime (`cljrs_async::init`) and a running
/// Tokio `LocalSet`.
fn builtin_decode_chan(args: &[Value]) -> ValueResult<Value> {
    let in_ch = channel_arg(args, 0)?;
    let encoding = resolve_encoding(args.get(1))?;
    let cap = cap_arg_or(args, 2, DEFAULT_CAP)?;

    let out_ch = make_chan(cap);
    let out_val = Value::NativeObject(out_ch.clone());

    spawn_future(async move {
        let mut dec = encoding.new_decoder();

        'pump: loop {
            match chan_take(&in_ch).await {
                Value::Nil => {
                    // Input closed — flush any bytes buffered inside the decoder
                    // (e.g. a trailing partial multi-byte sequence → U+FFFD).
                    let tail = decode_bytes(&mut dec, &[], true);
                    if !tail.is_empty() {
                        put(&out_ch, str_val(tail)).await;
                    }
                    break;
                }
                Value::ByteArray(a) => {
                    let raw: Vec<u8> = a.get().lock().unwrap().iter().map(|&b| b as u8).collect();
                    let s = decode_bytes(&mut dec, &raw, false);
                    if !s.is_empty() && !put(&out_ch, str_val(s)).await {
                        break 'pump;
                    }
                }
                Value::ByteBlob(blob) => {
                    let s = decode_bytes(&mut dec, blob.as_ref(), false);
                    // Skip empty strings: they happen when the decoder is
                    // buffering an incomplete multi-byte sequence.
                    if !s.is_empty() && !put(&out_ch, str_val(s)).await {
                        break 'pump; // output closed by consumer
                    }
                }
                other => {
                    // Forward errors and any other values unchanged.
                    if !put(&out_ch, other).await {
                        break 'pump;
                    }
                }
            }
        }

        chan_ref(out_ch.get()).close();
        Ok(Value::Nil)
    });

    Ok(out_val)
}

// ── encode-chan ───────────────────────────────────────────────────────────────

/// `(encode-chan strings-chan)` /
/// `(encode-chan strings-chan :windows-1252)` /
/// `(encode-chan strings-chan :windows-1252 16)`
///
/// Reads `String` values from `strings-chan`, encodes them with the given
/// charset (default UTF-8), and delivers `ByteBlob` values onto a new output
/// channel.  Unmappable characters are replaced with HTML numeric character
/// references.  The output channel is closed when the input closes.
///
/// Requires the `cljrs-async` runtime (`cljrs_async::init`) and a running
/// Tokio `LocalSet`.
fn builtin_encode_chan(args: &[Value]) -> ValueResult<Value> {
    let in_ch = channel_arg(args, 0)?;
    let encoding = resolve_encoding(args.get(1))?;
    let cap = cap_arg_or(args, 2, DEFAULT_CAP)?;

    let out_ch = make_chan(cap);
    let out_val = Value::NativeObject(out_ch.clone());

    spawn_future(async move {
        let mut enc = encoding.new_encoder();

        'pump: loop {
            match chan_take(&in_ch).await {
                Value::Nil => {
                    // Input closed — flush any pending encoder state.
                    let tail = encode_str(&mut enc, "", true);
                    if !tail.is_empty() {
                        put(&out_ch, blob_val(tail)).await;
                    }
                    break;
                }
                Value::Str(s) => {
                    let bytes = encode_str(&mut enc, s.get().as_str(), false);
                    if !bytes.is_empty() && !put(&out_ch, blob_val(bytes)).await {
                        break 'pump; // output closed by consumer
                    }
                }
                other => {
                    if !put(&out_ch, other).await {
                        break 'pump;
                    }
                }
            }
        }

        chan_ref(out_ch.get()).close();
        Ok(Value::Nil)
    });

    Ok(out_val)
}

// ── Value constructors ────────────────────────────────────────────────────────

fn str_val(s: String) -> Value {
    Value::Str(GcPtr::new(s))
}

fn blob_val(bytes: Vec<u8>) -> Value {
    let signed: Vec<i8> = bytes.iter().map(|&b| b as i8).collect();
    Value::ByteArray(GcPtr::new(Mutex::new(signed)))
}
