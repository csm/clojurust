//! Native function implementations for `clojure.rust.charset`.

use std::sync::Arc;

use cljrs_env::env::GlobalEnv;
use cljrs_gc::GcPtr;
use cljrs_value::{Arity, NativeFn, NativeFnPtr, NativeObjectBox, Value, ValueError, ValueResult};
use encoding_rs::Encoding;

use crate::codec::{CljDecoder, CljEncoder};

// ── Registration ─────────────────────────────────────────────────────────────

pub fn register(globals: &Arc<GlobalEnv>, ns: &str) {
    let entries: &[(&str, Arity, NativeFnPtr)] = &[
        ("decoder", Arity::Variadic { min: 0 }, builtin_decoder),
        ("encoder", Arity::Variadic { min: 0 }, builtin_encoder),
        ("update!", Arity::Fixed(2), builtin_update),
        ("finish!", Arity::Fixed(1), builtin_finish),
        ("decode", Arity::Variadic { min: 1 }, builtin_decode),
        ("encode", Arity::Variadic { min: 1 }, builtin_encode),
    ];
    for (name, arity, func) in entries {
        let nf = NativeFn::new(*name, arity.clone(), *func);
        globals.intern(ns, Arc::from(*name), Value::NativeFunction(GcPtr::new(nf)));
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Resolve an optional charset argument to an `encoding_rs::Encoding`.
///
/// Accepts a keyword (`:utf-8`) or string (`"utf-8"`); `None`/`nil` defaults
/// to UTF-8.  Unrecognised labels return an error.
fn resolve_encoding(arg: Option<&Value>) -> ValueResult<&'static Encoding> {
    let label: String = match arg {
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

fn bytes_from_value(v: &Value) -> ValueResult<&[u8]> {
    match v {
        Value::ByteBlob(b) => Ok(b.as_ref()),
        other => Err(ValueError::WrongType {
            expected: "byte-blob",
            got: other.type_name().to_string(),
        }),
    }
}

fn str_from_value(v: &Value) -> ValueResult<&str> {
    match v {
        Value::Str(s) => Ok(s.get().as_str()),
        other => Err(ValueError::WrongType {
            expected: "string",
            got: other.type_name().to_string(),
        }),
    }
}

fn as_native<'a>(v: &'a Value, expected: &'static str) -> ValueResult<&'a NativeObjectBox> {
    match v {
        Value::NativeObject(obj) => Ok(obj.get()),
        other => Err(ValueError::WrongType {
            expected,
            got: other.type_name().to_string(),
        }),
    }
}

fn bytes_to_value(bytes: Vec<u8>) -> Value {
    Value::ByteBlob(Arc::from(bytes.as_slice()))
}

// ── Streaming constructors ────────────────────────────────────────────────────

/// `(decoder)` or `(decoder :shift-jis)` — return a streaming decoder.
fn builtin_decoder(args: &[Value]) -> ValueResult<Value> {
    let enc = resolve_encoding(args.first())?;
    Ok(Value::NativeObject(cljrs_value::gc_native_object(
        CljDecoder::new(enc),
    )))
}

/// `(encoder)` or `(encoder :windows-1252)` — return a streaming encoder.
fn builtin_encoder(args: &[Value]) -> ValueResult<Value> {
    let enc = resolve_encoding(args.first())?;
    Ok(Value::NativeObject(cljrs_value::gc_native_object(
        CljEncoder::new(enc),
    )))
}

// ── Streaming operations ──────────────────────────────────────────────────────

/// `(update! decoder bytes)` → string
/// `(update! encoder string)` → byte-blob
///
/// Feed an incremental chunk to a codec.  The codec remains open for further
/// calls; use `finish!` to flush trailing state.
fn builtin_update(args: &[Value]) -> ValueResult<Value> {
    let obj = as_native(&args[0], "Decoder or Encoder")?;
    if let Some(dec) = obj.downcast_ref::<CljDecoder>() {
        let bytes = bytes_from_value(&args[1])?;
        let s = dec.update(bytes)?;
        Ok(Value::Str(GcPtr::new(s)))
    } else if let Some(enc) = obj.downcast_ref::<CljEncoder>() {
        let s = str_from_value(&args[1])?;
        let bytes = enc.update(s)?;
        Ok(bytes_to_value(bytes))
    } else {
        Err(ValueError::WrongType {
            expected: "Decoder or Encoder",
            got: obj.type_tag().to_string(),
        })
    }
}

/// `(finish! decoder)` → string
/// `(finish! encoder)` → byte-blob
///
/// Flush any buffered state and close the codec.  Further calls to `update!`
/// or `finish!` on the same object will return an error.
fn builtin_finish(args: &[Value]) -> ValueResult<Value> {
    let obj = as_native(&args[0], "Decoder or Encoder")?;
    if let Some(dec) = obj.downcast_ref::<CljDecoder>() {
        let s = dec.finish()?;
        Ok(Value::Str(GcPtr::new(s)))
    } else if let Some(enc) = obj.downcast_ref::<CljEncoder>() {
        let bytes = enc.finish()?;
        Ok(bytes_to_value(bytes))
    } else {
        Err(ValueError::WrongType {
            expected: "Decoder or Encoder",
            got: obj.type_tag().to_string(),
        })
    }
}

// ── One-shot helpers ──────────────────────────────────────────────────────────

/// `(decode bytes)` or `(decode bytes :shift-jis)` — decode bytes to a string.
///
/// Uses `encoding_rs`'s one-shot path which avoids creating a persistent
/// codec object.  Malformed sequences are replaced with U+FFFD.
fn builtin_decode(args: &[Value]) -> ValueResult<Value> {
    let bytes = bytes_from_value(&args[0])?;
    let enc = resolve_encoding(args.get(1))?;
    let (cow, _, _) = enc.decode(bytes);
    Ok(Value::Str(GcPtr::new(cow.into_owned())))
}

/// `(encode string)` or `(encode string :windows-1252)` — encode a string to bytes.
///
/// Uses `encoding_rs`'s one-shot path.  Unmappable characters are replaced
/// with HTML numeric character references.
fn builtin_encode(args: &[Value]) -> ValueResult<Value> {
    let s = str_from_value(&args[0])?;
    let enc = resolve_encoding(args.get(1))?;
    let (cow, _, _) = enc.encode(s);
    Ok(Value::ByteBlob(Arc::from(cow.as_ref())))
}
