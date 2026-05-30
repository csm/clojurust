//! Framing layer for `clojure.rust.net.frame` — Phase C of the networking plan.
//!
//! Stateful framers reassemble TCP byte-array chunks into application messages.
//! The primary API is the `frame` function, which pipes an input channel through
//! a framer spec into a new output channel:
//!
//! ```clojure
//! (let [msgs (frame (:in conn) (lines))]
//!   (go-loop [] (when-let [m (<! msgs)] (handle m) (recur))))
//!
//! (let [msgs (frame (:in conn) (length-prefixed {:bytes 4 :endian :big}))]
//!   ...)
//! ```
//!
//! Framer specs are created by `(lines)`, `(by-delimiter b)`, and
//! `(length-prefixed opts)` and consumed by `frame`.
//!
//! Encode direction helpers produce framed byte-arrays for the write side:
//! - `(lines-encode str)` → `byte-array` (UTF-8 + `\n`)
//! - `(length-prefixed-encode byte-array opts)` → `byte-array` (N-byte header + data)

use std::any::Any;
use std::sync::{Arc, Mutex};

use cljrs_async::channel::{chan_put, chan_ref, make_chan};
use cljrs_env::env::GlobalEnv;
use cljrs_gc::{GcPtr, MarkVisitor, Trace};
use cljrs_value::{
    Arity, ExceptionInfo, Keyword, MapValue, NativeFn, NativeObject, NativeObjectBox, Value,
    ValueError, ValueResult, gc_native_object,
};

// ── Public entry point ─────────────────────────────────────────────────────────

type Builtin = fn(&[Value]) -> ValueResult<Value>;

pub fn register(globals: &Arc<GlobalEnv>, ns: &str) {
    let fns: Vec<(&str, Arity, Builtin)> = vec![
        // Decoder specs
        ("lines", Arity::Fixed(0), builtin_lines),
        ("by-delimiter", Arity::Fixed(1), builtin_by_delimiter),
        ("length-prefixed", Arity::Fixed(1), builtin_length_prefixed),
        // Pipe: in-chan + spec → out-chan of messages
        ("frame", Arity::Variadic { min: 2 }, builtin_frame),
        // Encode helpers (write side)
        ("lines-encode", Arity::Fixed(1), builtin_lines_encode),
        (
            "length-prefixed-encode",
            Arity::Fixed(2),
            builtin_length_prefixed_encode,
        ),
    ];
    for (name, arity, func) in fns {
        let nf = NativeFn::new(name, arity, func);
        globals.intern(ns, Arc::from(name), Value::NativeFunction(GcPtr::new(nf)));
    }
}

// ── FramerSpec NativeObject ────────────────────────────────────────────────────

const FRAMER_TAG: &str = "FramerSpec";

#[derive(Clone, Debug)]
pub enum FramerKind {
    Lines,
    Delimiter(u8),
    LengthPrefixed { prefix_len: usize, big_endian: bool },
}

/// Native object that describes a framing algorithm. Created by `(lines)`,
/// `(by-delimiter b)`, or `(length-prefixed opts)` and consumed by `frame`.
#[derive(Clone, Debug)]
pub struct FramerSpec {
    pub kind: FramerKind,
    /// Default output channel buffer depth (overridden by the optional 3rd arg to `frame`).
    pub out_buf: usize,
}

impl Trace for FramerSpec {
    fn trace(&self, _visitor: &mut MarkVisitor) {}
}

impl NativeObject for FramerSpec {
    fn type_tag(&self) -> &str {
        FRAMER_TAG
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ── Stateful framer trait + implementations ────────────────────────────────────

/// A stateful byte-stream framer. `feed` accepts a chunk and returns any
/// complete frames; `finish` flushes any partial frame at EOF (may return `None`
/// if the protocol requires a clean frame boundary).
trait Framer {
    fn feed(&mut self, bytes: &[u8]) -> Vec<Value>;
    fn finish(&mut self) -> Option<Value>;
}

// ── Lines framer ──────────────────────────────────────────────────────────────

/// Splits on `\n`, strips trailing `\r`, emits `Value::Str` per line.
struct LinesFramer {
    buf: Vec<u8>,
}

impl LinesFramer {
    fn new() -> Self {
        Self { buf: Vec::new() }
    }
}

impl Framer for LinesFramer {
    fn feed(&mut self, bytes: &[u8]) -> Vec<Value> {
        let mut frames = Vec::new();
        self.buf.extend_from_slice(bytes);
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..pos).collect();
            self.buf.remove(0); // consume the '\n'
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let s = String::from_utf8_lossy(&line).into_owned();
            frames.push(Value::string(s));
        }
        frames
    }

    fn finish(&mut self) -> Option<Value> {
        if self.buf.is_empty() {
            return None;
        }
        let mut line = std::mem::take(&mut self.buf);
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        let s = String::from_utf8_lossy(&line).into_owned();
        Some(Value::string(s))
    }
}

// ── Delimiter framer ──────────────────────────────────────────────────────────

/// Splits on a single delimiter byte, emits `byte-array` frames (delimiter excluded).
struct DelimiterFramer {
    delim: u8,
    buf: Vec<u8>,
}

impl DelimiterFramer {
    fn new(delim: u8) -> Self {
        Self {
            delim,
            buf: Vec::new(),
        }
    }
}

impl Framer for DelimiterFramer {
    fn feed(&mut self, bytes: &[u8]) -> Vec<Value> {
        let mut frames = Vec::new();
        self.buf.extend_from_slice(bytes);
        while let Some(pos) = self.buf.iter().position(|&b| b == self.delim) {
            let frame: Vec<u8> = self.buf.drain(..pos).collect();
            self.buf.remove(0); // consume the delimiter
            frames.push(bytes_value(&frame));
        }
        frames
    }

    fn finish(&mut self) -> Option<Value> {
        if self.buf.is_empty() {
            return None;
        }
        let frame = std::mem::take(&mut self.buf);
        Some(bytes_value(&frame))
    }
}

// ── Length-prefixed framer ────────────────────────────────────────────────────

#[derive(PartialEq)]
enum LpState {
    Header,
    Body(usize),
}

/// Reads an N-byte big- or little-endian length header, then reads that many body
/// bytes and emits a `byte-array` frame. Handles chunks that span multiple frames
/// or that split a header/body boundary.
struct LengthPrefixedFramer {
    prefix_len: usize,
    big_endian: bool,
    buf: Vec<u8>,
    state: LpState,
}

impl LengthPrefixedFramer {
    fn new(prefix_len: usize, big_endian: bool) -> Self {
        Self {
            prefix_len,
            big_endian,
            buf: Vec::new(),
            state: LpState::Header,
        }
    }

    fn parse_length(&self, header: &[u8]) -> usize {
        let mut n: u64 = 0;
        if self.big_endian {
            for &b in header {
                n = (n << 8) | (b as u64);
            }
        } else {
            for &b in header.iter().rev() {
                n = (n << 8) | (b as u64);
            }
        }
        n as usize
    }
}

impl Framer for LengthPrefixedFramer {
    fn feed(&mut self, bytes: &[u8]) -> Vec<Value> {
        let mut frames = Vec::new();
        self.buf.extend_from_slice(bytes);
        loop {
            match self.state {
                LpState::Header => {
                    if self.buf.len() < self.prefix_len {
                        break;
                    }
                    let header: Vec<u8> = self.buf.drain(..self.prefix_len).collect();
                    let body_len = self.parse_length(&header);
                    self.state = LpState::Body(body_len);
                }
                LpState::Body(body_len) => {
                    if self.buf.len() < body_len {
                        break;
                    }
                    let body: Vec<u8> = self.buf.drain(..body_len).collect();
                    frames.push(bytes_value(&body));
                    self.state = LpState::Header;
                }
            }
        }
        frames
    }

    fn finish(&mut self) -> Option<Value> {
        // A partial frame at EOF is a protocol error; discard silently.
        None
    }
}

// ── Value helpers ──────────────────────────────────────────────────────────────

fn bytes_value(bytes: &[u8]) -> Value {
    let signed: Vec<i8> = bytes.iter().map(|&b| b as i8).collect();
    Value::ByteArray(GcPtr::new(Mutex::new(signed)))
}

fn frame_error(msg: impl Into<String>) -> Value {
    let msg = msg.into();
    Value::Error(GcPtr::new(ExceptionInfo::new(
        ValueError::Other(msg.clone()),
        msg,
        None,
        None,
    )))
}

fn kw(name: &str) -> Value {
    Value::keyword(Keyword::simple(name))
}

fn get_bytes(v: &Value) -> Option<Vec<u8>> {
    match v {
        Value::ByteArray(arr) => Some(arr.get().lock().unwrap().iter().map(|&b| b as u8).collect()),
        _ => None,
    }
}

fn parse_prefix_opts(opts: &MapValue) -> (usize, bool) {
    let prefix_len = match opts.get(&kw("bytes")) {
        Some(Value::Long(n)) if n > 0 && n <= 8 => n as usize,
        _ => 4,
    };
    let big_endian = opts
        .get(&kw("endian"))
        .map(|v| v != kw("little"))
        .unwrap_or(true);
    (prefix_len, big_endian)
}

// ── Framer async pipe task ─────────────────────────────────────────────────────

/// Reads from `in_chan`, feeds bytes through `framer`, and puts complete frames
/// on `out_chan`. Propagates `Value::Error` in-band and closes `out_chan` at EOF
/// or on error, exactly matching the `cljrs-io` streaming channel contract.
async fn framer_task(
    in_chan: GcPtr<NativeObjectBox>,
    out_chan: GcPtr<NativeObjectBox>,
    mut framer: Box<dyn Framer>,
) {
    'outer: loop {
        let val = chan_ref(in_chan.get()).take().await;
        match val {
            Value::Nil => {
                // EOF: flush any trailing partial frame, then close.
                if let Some(frame) = framer.finish() {
                    chan_put(&out_chan, frame).await;
                }
                break;
            }
            Value::Error(_) => {
                // Propagate error in-band then close.
                chan_put(&out_chan, val).await;
                break;
            }
            Value::ByteArray(_) => {
                if let Some(bytes) = get_bytes(&val) {
                    for frame in framer.feed(&bytes) {
                        if !chan_put(&out_chan, frame).await {
                            break 'outer;
                        }
                    }
                }
            }
            other => {
                chan_put(
                    &out_chan,
                    frame_error(format!(
                        "frame: unexpected value on :in (expected byte-array, got {})",
                        other.type_name()
                    )),
                )
                .await;
                break;
            }
        }
    }
    chan_ref(out_chan.get()).close();
}

// ── Builtin functions ──────────────────────────────────────────────────────────

/// `(lines)` — return a framer spec that splits byte-array chunks on `\n` and
/// emits a `string` per line (strips trailing `\r`). Pass to `frame`.
fn builtin_lines(_args: &[Value]) -> ValueResult<Value> {
    let spec = FramerSpec {
        kind: FramerKind::Lines,
        out_buf: 8,
    };
    Ok(Value::NativeObject(gc_native_object(spec)))
}

/// `(by-delimiter b)` — return a framer spec that splits on byte `b` (0-255)
/// and emits a `byte-array` per frame (delimiter excluded). Pass to `frame`.
fn builtin_by_delimiter(args: &[Value]) -> ValueResult<Value> {
    let delim = match args.first() {
        Some(Value::Long(n)) if *n >= 0 && *n <= 255 => *n as u8,
        other => {
            return Err(ValueError::WrongType {
                expected: "byte (long 0-255)",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };
    let spec = FramerSpec {
        kind: FramerKind::Delimiter(delim),
        out_buf: 8,
    };
    Ok(Value::NativeObject(gc_native_object(spec)))
}

/// `(length-prefixed {:bytes n :endian :big})` — return a framer spec that
/// reads an N-byte (default 4) big-endian length header, then emits a
/// `byte-array` of exactly that many bytes. Pass to `frame`.
///
/// Options:
///   `:bytes`  — prefix width in bytes: 1, 2, 4 (default), or 8
///   `:endian` — `:big` (default) or `:little`
fn builtin_length_prefixed(args: &[Value]) -> ValueResult<Value> {
    let opts = match args.first() {
        Some(Value::Map(m)) => m.clone(),
        other => {
            return Err(ValueError::WrongType {
                expected: "map {:bytes n}",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };
    let (prefix_len, big_endian) = parse_prefix_opts(&opts);
    let spec = FramerSpec {
        kind: FramerKind::LengthPrefixed {
            prefix_len,
            big_endian,
        },
        out_buf: 8,
    };
    Ok(Value::NativeObject(gc_native_object(spec)))
}

/// `(frame in-chan framer-spec)` / `(frame in-chan framer-spec out-buf)` —
/// pipe `in-chan` through the framer and return a new channel that emits
/// complete application messages (strings or byte-arrays depending on the spec).
///
/// Spawns a background task on the `LocalSet` that reads byte-array chunks from
/// `in-chan`, feeds them through the stateful framer, and puts complete frames
/// on the output channel. The output channel closes when `in-chan` closes (EOF)
/// or when an error is propagated.
///
/// The optional `out-buf` argument controls the output channel's buffer depth
/// (default 8).
fn builtin_frame(args: &[Value]) -> ValueResult<Value> {
    let in_chan = match args.first() {
        Some(Value::NativeObject(obj)) if obj.get().type_tag() == "Channel" => obj.clone(),
        Some(Value::NativeObject(obj)) => {
            return Err(ValueError::Other(format!(
                "frame: first argument must be a channel, got native object type '{}'",
                obj.get().type_tag()
            )));
        }
        other => {
            return Err(ValueError::WrongType {
                expected: "channel",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };

    let spec = match args.get(1) {
        Some(Value::NativeObject(obj)) => match obj.get().downcast_ref::<FramerSpec>() {
            Some(s) => s.clone(),
            None => {
                return Err(ValueError::Other(format!(
                    "frame: second argument must be a framer spec (lines/by-delimiter/length-prefixed), \
                     got native object type '{}'",
                    obj.get().type_tag()
                )));
            }
        },
        other => {
            return Err(ValueError::WrongType {
                expected: "framer spec (lines, by-delimiter, or length-prefixed)",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };

    let out_buf = match args.get(2) {
        Some(Value::Long(n)) if *n > 0 => *n as usize,
        None | Some(Value::Nil) => spec.out_buf,
        Some(other) => {
            return Err(ValueError::WrongType {
                expected: "positive long (out-buf)",
                got: other.type_name().to_string(),
            });
        }
    };

    let out_chan = make_chan(out_buf);
    let out_val = Value::NativeObject(out_chan.clone());

    let framer: Box<dyn Framer> = match spec.kind {
        FramerKind::Lines => Box::new(LinesFramer::new()),
        FramerKind::Delimiter(d) => Box::new(DelimiterFramer::new(d)),
        FramerKind::LengthPrefixed {
            prefix_len,
            big_endian,
        } => Box::new(LengthPrefixedFramer::new(prefix_len, big_endian)),
    };

    tokio::task::spawn_local(framer_task(in_chan, out_chan, framer));

    Ok(out_val)
}

/// `(lines-encode str)` — encode a string for a line protocol by appending `\n`.
/// Returns a `byte-array` of the UTF-8 bytes followed by `\n`.
fn builtin_lines_encode(args: &[Value]) -> ValueResult<Value> {
    let s = match args.first() {
        Some(Value::Str(s)) => s.get().clone(),
        other => {
            return Err(ValueError::WrongType {
                expected: "string",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };
    let mut bytes: Vec<u8> = s.into_bytes();
    bytes.push(b'\n');
    Ok(bytes_value(&bytes))
}

/// `(length-prefixed-encode byte-array opts)` — prepend an N-byte length header
/// to `byte-array`. Same options as `length-prefixed`: `:bytes` (default 4),
/// `:endian` (default `:big`).
///
/// Returns a new `byte-array` of `[header || data]`.
fn builtin_length_prefixed_encode(args: &[Value]) -> ValueResult<Value> {
    let data: Vec<u8> = match args.first() {
        Some(Value::ByteArray(arr)) => arr.get().lock().unwrap().iter().map(|&b| b as u8).collect(),
        other => {
            return Err(ValueError::WrongType {
                expected: "byte-array",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };

    let (prefix_len, big_endian) = match args.get(1) {
        Some(Value::Map(m)) => parse_prefix_opts(m),
        None | Some(Value::Nil) => (4, true),
        Some(other) => {
            return Err(ValueError::WrongType {
                expected: "map {:bytes n :endian :big/:little}",
                got: other.type_name().to_string(),
            });
        }
    };

    let len = data.len() as u64;
    let header: Vec<u8> = if big_endian {
        (0..prefix_len)
            .rev()
            .map(|i| ((len >> (i * 8)) & 0xFF) as u8)
            .collect()
    } else {
        (0..prefix_len)
            .map(|i| ((len >> (i * 8)) & 0xFF) as u8)
            .collect()
    };

    let mut result = header;
    result.extend_from_slice(&data);
    Ok(bytes_value(&result))
}

// ── Public Rust API for tests ─────────────────────────────────────────────────

/// Pipe `in_chan` through the given `FramerSpec`, returning the output channel.
/// Convenience wrapper for tests and other Rust consumers.
pub fn frame_channel(
    in_chan: GcPtr<NativeObjectBox>,
    spec: FramerSpec,
    out_buf: usize,
) -> GcPtr<NativeObjectBox> {
    let out_chan = make_chan(out_buf);
    let framer: Box<dyn Framer> = match spec.kind {
        FramerKind::Lines => Box::new(LinesFramer::new()),
        FramerKind::Delimiter(d) => Box::new(DelimiterFramer::new(d)),
        FramerKind::LengthPrefixed {
            prefix_len,
            big_endian,
        } => Box::new(LengthPrefixedFramer::new(prefix_len, big_endian)),
    };
    tokio::task::spawn_local(framer_task(in_chan, out_chan.clone(), framer));
    out_chan
}

/// Encode a string for the line protocol (appends `\n`).
pub fn encode_line(s: &str) -> Value {
    let mut bytes: Vec<u8> = s.as_bytes().to_vec();
    bytes.push(b'\n');
    bytes_value(&bytes)
}

/// Encode a byte slice for the length-prefixed protocol (prepends N-byte header).
pub fn encode_length_prefixed(data: &[u8], prefix_len: usize, big_endian: bool) -> Value {
    let len = data.len() as u64;
    let header: Vec<u8> = if big_endian {
        (0..prefix_len)
            .rev()
            .map(|i| ((len >> (i * 8)) & 0xFF) as u8)
            .collect()
    } else {
        (0..prefix_len)
            .map(|i| ((len >> (i * 8)) & 0xFF) as u8)
            .collect()
    };
    let mut result = header;
    result.extend_from_slice(data);
    bytes_value(&result)
}
