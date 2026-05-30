//! Native async file-I/O builtins for `clojure.rust.io.async`.
//!
//! Two delivery shapes, picked per operation (see the crate README for the
//! rationale):
//!
//! - **Streaming reads** — `chunk-chan`, `byte-chan`, `char-chan`, `line-chan`
//!   return a `core.async` channel *immediately* and spawn a producer task that
//!   reads the file and puts a sequence of values onto the channel, closing it
//!   at EOF. The channel's small buffer provides backpressure, so the producer
//!   only reads as far ahead as the consumer takes — large files never need to
//!   fit in memory.
//!
//! - **Discrete ops** — `slurp`, `slurp-bytes`, `read-bytes`, `spit` return a
//!   *promise channel* (capacity 1): the producer delivers exactly one result
//!   and closes the channel, so a single `(<! ...)` yields the value.
//!
//! Failures are delivered in-band as a `Value::Error` value (constructed by
//! [`io_error`]) on the same channel, then the channel is closed. Consumers can
//! test results with the `error?` helper in the namespace's Clojure source.
//!
//! All channels are ordinary `cljrs-async` [`CljChannel`]s, so the full
//! `clojure.core.async` API (`<!`, `<!!`, `alts!`, `go`, …) operates on them.

use std::sync::{Arc, Mutex};

use tokio::io::AsyncReadExt;

use cljrs_async::channel::{chan_deliver as deliver, chan_put as put, chan_ref, make_chan};
use cljrs_async::spawn_future;
use cljrs_env::env::GlobalEnv;
use cljrs_gc::GcPtr;
use cljrs_value::{
    Arity, ExceptionInfo, NativeFn, Value, ValueError, ValueResult,
};

use crate::charset::{CharDecoder, resolve_charset};

/// Default read buffer for `chunk-chan` / `byte-chan` / decoded streams.
const DEFAULT_BUF: usize = 8192;
/// Default backpressure buffer for streaming channels: the producer may read at
/// most this many values ahead of the consumer.
const DEFAULT_STREAM_CAP: usize = 8;

/// Signature shared by every native builtin in this crate.
type Builtin = fn(&[Value]) -> ValueResult<Value>;

/// Register the async I/O native functions into the given namespace.
pub fn register(globals: &Arc<GlobalEnv>, ns: &str) {
    let fns: Vec<(&str, Arity, Builtin)> = vec![
        // Streaming reads (raw channels).
        ("chunk-chan", Arity::Variadic { min: 1 }, builtin_chunk_chan),
        ("byte-chan", Arity::Variadic { min: 1 }, builtin_byte_chan),
        ("char-chan", Arity::Variadic { min: 1 }, builtin_char_chan),
        ("line-chan", Arity::Variadic { min: 1 }, builtin_line_chan),
        // Discrete ops (promise channels).
        ("slurp", Arity::Variadic { min: 1 }, builtin_slurp),
        ("slurp-bytes", Arity::Fixed(1), builtin_slurp_bytes),
        ("read-bytes", Arity::Fixed(2), builtin_read_bytes),
        ("spit", Arity::Variadic { min: 2 }, builtin_spit),
    ];
    for (name, arity, func) in fns {
        let nf = NativeFn::new(name, arity, func);
        globals.intern(ns, Arc::from(name), Value::NativeFunction(GcPtr::new(nf)));
    }
}

// ── Value helpers ─────────────────────────────────────────────────────────────

/// Wrap raw bytes as a Clojure `byte-array` (`Value::ByteArray`, signed `i8`s).
fn bytes_value(bytes: &[u8]) -> Value {
    let signed: Vec<i8> = bytes.iter().map(|&b| b as i8).collect();
    Value::ByteArray(GcPtr::new(Mutex::new(signed)))
}

/// Build an in-band error value to put on a channel when an I/O step fails.
fn io_error(msg: impl Into<String>) -> Value {
    let msg = msg.into();
    Value::Error(GcPtr::new(ExceptionInfo::new(
        ValueError::Other(msg.clone()),
        msg,
        None,
        None,
    )))
}

// ── Argument parsing ──────────────────────────────────────────────────────────

fn str_arg(args: &[Value], idx: usize, expected: &'static str) -> ValueResult<String> {
    match args.get(idx) {
        Some(Value::Str(s)) => Ok(s.get().clone()),
        other => Err(ValueError::WrongType {
            expected,
            got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
        }),
    }
}

/// Read an optional positive-`usize` argument (e.g. buffer/capacity), clamping
/// to at least 1 and falling back to `default` when absent/`nil`.
fn size_arg_or(args: &[Value], idx: usize, default: usize) -> ValueResult<usize> {
    match args.get(idx) {
        None | Some(Value::Nil) => Ok(default),
        Some(Value::Long(n)) if *n >= 0 => Ok((*n as usize).max(1)),
        Some(other) => Err(ValueError::WrongType {
            expected: "non-negative long",
            got: other.type_name().to_string(),
        }),
    }
}

// ── Streaming reads ───────────────────────────────────────────────────────────

/// `(chunk-chan path)` / `(chunk-chan path buf-size)` /
/// `(chunk-chan path buf-size cap)` — a channel of `byte-array` chunks of up to
/// `buf-size` bytes (default 8192), closed at EOF. `cap` sets the channel's
/// backpressure buffer (default 8).
fn builtin_chunk_chan(args: &[Value]) -> ValueResult<Value> {
    let path = str_arg(args, 0, "string (path)")?;
    let buf_size = size_arg_or(args, 1, DEFAULT_BUF)?;
    let cap = size_arg_or(args, 2, DEFAULT_STREAM_CAP)?;
    let ch = make_chan(cap);
    let ch_val = Value::NativeObject(ch.clone());
    spawn_future(async move {
        let mut file = match tokio::fs::File::open(&path).await {
            Ok(f) => f,
            Err(e) => {
                put(&ch, io_error(format!("cannot open {path}: {e}"))).await;
                chan_ref(ch.get()).close();
                return Ok(Value::Nil);
            }
        };
        let mut buf = vec![0u8; buf_size];
        loop {
            match file.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if !put(&ch, bytes_value(&buf[..n])).await {
                        break; // consumer closed the channel
                    }
                }
                Err(e) => {
                    put(&ch, io_error(format!("read error on {path}: {e}"))).await;
                    break;
                }
            }
        }
        chan_ref(ch.get()).close();
        Ok(Value::Nil)
    });
    Ok(ch_val)
}

/// `(byte-chan path)` / `(byte-chan path cap)` — a channel of individual bytes
/// as signed `long`s (-128..127, matching `byte-array`/`aget` semantics),
/// closed at EOF.
fn builtin_byte_chan(args: &[Value]) -> ValueResult<Value> {
    let path = str_arg(args, 0, "string (path)")?;
    let cap = size_arg_or(args, 1, DEFAULT_STREAM_CAP)?;
    let ch = make_chan(cap);
    let ch_val = Value::NativeObject(ch.clone());
    spawn_future(async move {
        let mut file = match tokio::fs::File::open(&path).await {
            Ok(f) => f,
            Err(e) => {
                put(&ch, io_error(format!("cannot open {path}: {e}"))).await;
                chan_ref(ch.get()).close();
                return Ok(Value::Nil);
            }
        };
        let mut buf = vec![0u8; DEFAULT_BUF];
        'outer: loop {
            match file.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    for &b in &buf[..n] {
                        if !put(&ch, Value::Long((b as i8) as i64)).await {
                            break 'outer;
                        }
                    }
                }
                Err(e) => {
                    put(&ch, io_error(format!("read error on {path}: {e}"))).await;
                    break;
                }
            }
        }
        chan_ref(ch.get()).close();
        Ok(Value::Nil)
    });
    Ok(ch_val)
}

/// `(char-chan path)` / `(char-chan path charset)` /
/// `(char-chan path charset cap)` — a channel of characters decoded from the
/// file with `charset` (default `:utf-8`), closed at EOF.
fn builtin_char_chan(args: &[Value]) -> ValueResult<Value> {
    let path = str_arg(args, 0, "string (path)")?;
    let encoding = resolve_charset(args.get(1))?;
    let cap = size_arg_or(args, 2, DEFAULT_STREAM_CAP)?;
    let ch = make_chan(cap);
    let ch_val = Value::NativeObject(ch.clone());
    spawn_future(async move {
        let mut file = match tokio::fs::File::open(&path).await {
            Ok(f) => f,
            Err(e) => {
                put(&ch, io_error(format!("cannot open {path}: {e}"))).await;
                chan_ref(ch.get()).close();
                return Ok(Value::Nil);
            }
        };
        let mut decoder = CharDecoder::new(encoding);
        let mut buf = vec![0u8; DEFAULT_BUF];
        'outer: loop {
            match file.read(&mut buf).await {
                Ok(0) => {
                    for c in decoder.finish().chars() {
                        let _ = put(&ch, Value::Char(c)).await;
                    }
                    break;
                }
                Ok(n) => {
                    for c in decoder.push(&buf[..n]).chars() {
                        if !put(&ch, Value::Char(c)).await {
                            break 'outer;
                        }
                    }
                }
                Err(e) => {
                    put(&ch, io_error(format!("read error on {path}: {e}"))).await;
                    break;
                }
            }
        }
        chan_ref(ch.get()).close();
        Ok(Value::Nil)
    });
    Ok(ch_val)
}

/// `(line-chan path)` / `(line-chan path charset)` /
/// `(line-chan path charset cap)` — a channel of lines (without their trailing
/// `\n`/`\r\n`) decoded from the file with `charset` (default `:utf-8`), closed
/// at EOF.
fn builtin_line_chan(args: &[Value]) -> ValueResult<Value> {
    let path = str_arg(args, 0, "string (path)")?;
    let encoding = resolve_charset(args.get(1))?;
    let cap = size_arg_or(args, 2, DEFAULT_STREAM_CAP)?;
    let ch = make_chan(cap);
    let ch_val = Value::NativeObject(ch.clone());
    spawn_future(async move {
        let mut file = match tokio::fs::File::open(&path).await {
            Ok(f) => f,
            Err(e) => {
                put(&ch, io_error(format!("cannot open {path}: {e}"))).await;
                chan_ref(ch.get()).close();
                return Ok(Value::Nil);
            }
        };
        let mut decoder = CharDecoder::new(encoding);
        let mut pending = String::new();
        let mut buf = vec![0u8; DEFAULT_BUF];
        'outer: loop {
            let (text, eof) = match file.read(&mut buf).await {
                Ok(0) => (decoder.finish(), true),
                Ok(n) => (decoder.push(&buf[..n]), false),
                Err(e) => {
                    put(&ch, io_error(format!("read error on {path}: {e}"))).await;
                    break;
                }
            };
            pending.push_str(&text);
            while let Some(idx) = pending.find('\n') {
                let mut line = pending[..idx].to_string();
                if line.ends_with('\r') {
                    line.pop();
                }
                pending.replace_range(..=idx, "");
                if !put(&ch, Value::string(line)).await {
                    break 'outer;
                }
            }
            if eof {
                if !pending.is_empty() {
                    let _ = put(&ch, Value::string(std::mem::take(&mut pending))).await;
                }
                break;
            }
        }
        chan_ref(ch.get()).close();
        Ok(Value::Nil)
    });
    Ok(ch_val)
}

// ── Discrete ops (promise channels) ───────────────────────────────────────────

/// `(slurp path)` / `(slurp path charset)` — a promise channel delivering the
/// whole file decoded to a string with `charset` (default `:utf-8`).
fn builtin_slurp(args: &[Value]) -> ValueResult<Value> {
    let path = str_arg(args, 0, "string (path)")?;
    let encoding = resolve_charset(args.get(1))?;
    let ch = make_chan(1);
    let ch_val = Value::NativeObject(ch.clone());
    spawn_future(async move {
        let result = match tokio::fs::read(&path).await {
            Ok(bytes) => {
                let (text, _, _) = encoding.decode(&bytes);
                Value::string(text.into_owned())
            }
            Err(e) => io_error(format!("cannot read {path}: {e}")),
        };
        deliver(&ch, result).await;
        Ok(Value::Nil)
    });
    Ok(ch_val)
}

/// `(slurp-bytes path)` — a promise channel delivering the whole file as a
/// `byte-array`.
fn builtin_slurp_bytes(args: &[Value]) -> ValueResult<Value> {
    let path = str_arg(args, 0, "string (path)")?;
    let ch = make_chan(1);
    let ch_val = Value::NativeObject(ch.clone());
    spawn_future(async move {
        let result = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes_value(&bytes),
            Err(e) => io_error(format!("cannot read {path}: {e}")),
        };
        deliver(&ch, result).await;
        Ok(Value::Nil)
    });
    Ok(ch_val)
}

/// `(read-bytes path n)` — a promise channel delivering a `byte-array` of up to
/// the first `n` bytes of the file (fewer if the file is shorter).
fn builtin_read_bytes(args: &[Value]) -> ValueResult<Value> {
    let path = str_arg(args, 0, "string (path)")?;
    let n = match args.get(1) {
        Some(Value::Long(n)) if *n >= 0 => *n as usize,
        other => {
            return Err(ValueError::WrongType {
                expected: "non-negative long (byte count)",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };
    let ch = make_chan(1);
    let ch_val = Value::NativeObject(ch.clone());
    spawn_future(async move {
        let result = read_n_bytes(&path, n).await;
        deliver(&ch, result).await;
        Ok(Value::Nil)
    });
    Ok(ch_val)
}

/// Read up to `n` bytes from `path`, returning a `byte-array` value or an error
/// value.
async fn read_n_bytes(path: &str, n: usize) -> Value {
    let mut file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(e) => return io_error(format!("cannot open {path}: {e}")),
    };
    let mut out: Vec<u8> = Vec::with_capacity(n);
    let mut buf = vec![0u8; n.clamp(1, DEFAULT_BUF)];
    while out.len() < n {
        let want = buf.len().min(n - out.len());
        match file.read(&mut buf[..want]).await {
            Ok(0) => break,
            Ok(m) => out.extend_from_slice(&buf[..m]),
            Err(e) => return io_error(format!("read error on {path}: {e}")),
        }
    }
    bytes_value(&out)
}

/// `(spit path data)` / `(spit path data charset)` — write `data` (a string or
/// `byte-array`) to `path`, truncating any existing contents. Returns a promise
/// channel delivering the number of bytes written. Strings are encoded with
/// `charset` (default `:utf-8`).
fn builtin_spit(args: &[Value]) -> ValueResult<Value> {
    let path = str_arg(args, 0, "string (path)")?;
    let encoding = resolve_charset(args.get(2))?;
    // Materialise the bytes synchronously so no `Value`/`GcPtr` crosses into the
    // spawned task (the produced `Vec<u8>` is plain data).
    let bytes: Vec<u8> = match args.get(1) {
        Some(Value::Str(s)) => {
            let (encoded, _, _) = encoding.encode(s.get());
            encoded.into_owned()
        }
        Some(Value::ByteArray(a)) => a.get().lock().unwrap().iter().map(|&b| b as u8).collect(),
        other => {
            return Err(ValueError::WrongType {
                expected: "string or byte-array (data)",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };
    let ch = make_chan(1);
    let ch_val = Value::NativeObject(ch.clone());
    spawn_future(async move {
        let result = match tokio::fs::write(&path, &bytes).await {
            Ok(()) => Value::Long(bytes.len() as i64),
            Err(e) => io_error(format!("cannot write {path}: {e}")),
        };
        deliver(&ch, result).await;
        Ok(Value::Nil)
    });
    Ok(ch_val)
}
