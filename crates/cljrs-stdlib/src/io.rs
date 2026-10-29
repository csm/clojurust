//! Native implementations for `clojure.rust.io`.

use std::any::Any;
use std::io::{BufRead, BufReader, BufWriter, Cursor, Read, Write};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use cljrs_value::resource::Resource;
use cljrs_value::{Arity, ResourceHandle, Value, ValueError, ValueResult};

use crate::register_fns;

// ── Concrete resource types ──────────────────────────────────────────────────

// Only owned host data enters a worker. Managed errors are built after the wait.
fn file_work<T: Send + 'static>(
    job: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> ValueResult<T> {
    cljrs_value::parking::blocking(job).map_err(ValueError::Other)
}

/// A buffered file reader with parking-aware operations.
#[derive(Debug)]
pub struct IoReader {
    inner: Arc<Mutex<Option<BufReader<std::fs::File>>>>,
    closed: Arc<AtomicBool>,
}

impl IoReader {
    pub fn open(path: &str) -> ValueResult<Self> {
        let path = path.to_owned();
        let file = file_work(move || {
            std::fs::File::open(&path).map_err(|e| format!("cannot open {path}: {e}"))
        })?;
        Ok(Self {
            inner: Arc::new(Mutex::new(Some(BufReader::new(file)))),
            closed: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn read_line(&self) -> ValueResult<Option<String>> {
        let inner = self.inner.clone();
        file_work(move || {
            let mut guard = inner.lock().unwrap();
            let reader = guard.as_mut().ok_or("reader is closed")?;
            let mut line = String::new();
            let n = reader
                .read_line(&mut line)
                .map_err(|e| format!("read error: {e}"))?;
            Ok((n != 0).then_some(line))
        })
    }

    pub fn read_all(&self) -> ValueResult<String> {
        let inner = self.inner.clone();
        file_work(move || {
            let mut guard = inner.lock().unwrap();
            let reader = guard.as_mut().ok_or("reader is closed")?;
            let mut buf = String::new();
            reader
                .read_to_string(&mut buf)
                .map_err(|e| format!("read error: {e}"))?;
            Ok(buf)
        })
    }
}

impl Resource for IoReader {
    fn close(&self) -> ValueResult<()> {
        let inner = self.inner.clone();
        let closed = self.closed.clone();
        file_work(move || {
            *inner.lock().unwrap() = None;
            closed.store(true, Ordering::Release);
            Ok(())
        })
    }
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
    fn resource_type(&self) -> &'static str {
        "reader"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A buffered file writer with parking-aware operations.
#[derive(Debug)]
pub struct IoWriter {
    inner: Arc<Mutex<Option<BufWriter<std::fs::File>>>>,
    closed: Arc<AtomicBool>,
}

impl IoWriter {
    pub fn open(path: &str, append: bool) -> ValueResult<Self> {
        let path = path.to_owned();
        let file = file_work(move || {
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(!append)
                .append(append)
                .open(&path)
                .map_err(|e| format!("cannot open {path}: {e}"))
        })?;
        Ok(Self {
            inner: Arc::new(Mutex::new(Some(BufWriter::new(file)))),
            closed: Arc::new(AtomicBool::new(false)),
        })
    }
    pub fn write_str(&self, s: &str) -> ValueResult<()> {
        let inner = self.inner.clone();
        let text = s.to_owned();
        file_work(move || {
            let mut guard = inner.lock().unwrap();
            let writer = guard.as_mut().ok_or("writer is closed")?;
            writer
                .write_all(text.as_bytes())
                .map_err(|e| format!("write error: {e}"))
        })
    }
    pub fn flush(&self) -> ValueResult<()> {
        let inner = self.inner.clone();
        file_work(move || {
            if let Some(writer) = inner.lock().unwrap().as_mut() {
                writer.flush().map_err(|e| format!("flush error: {e}"))?;
            }
            Ok(())
        })
    }
}
impl Resource for IoWriter {
    fn close(&self) -> ValueResult<()> {
        let inner = self.inner.clone();
        let closed = self.closed.clone();
        file_work(move || {
            let mut guard = inner.lock().unwrap();
            let mut writer = guard.take();
            if let Some(writer) = writer.as_mut() {
                let _ = writer.flush();
            }
            drop(writer);
            closed.store(true, Ordering::Release);
            Ok(())
        })
    }
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
    fn resource_type(&self) -> &'static str {
        "writer"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A reader backed by a String (for EDN read-string, etc.).
#[derive(Debug)]
pub struct StringReader {
    inner: Mutex<Option<Cursor<String>>>,
}

impl StringReader {
    pub fn new(s: String) -> Self {
        Self {
            inner: Mutex::new(Some(Cursor::new(s))),
        }
    }

    pub fn read_line(&self) -> ValueResult<Option<String>> {
        let mut guard = self.inner.lock().unwrap();
        let cursor = guard
            .as_mut()
            .ok_or_else(|| ValueError::Other("reader is closed".into()))?;
        let mut line = String::new();
        let n = BufRead::read_line(cursor, &mut line)
            .map_err(|e| ValueError::Other(format!("read error: {e}")))?;
        if n == 0 { Ok(None) } else { Ok(Some(line)) }
    }

    pub fn read_all(&self) -> ValueResult<String> {
        let mut guard = self.inner.lock().unwrap();
        let cursor = guard
            .as_mut()
            .ok_or_else(|| ValueError::Other("reader is closed".into()))?;
        let mut buf = String::new();
        cursor
            .read_to_string(&mut buf)
            .map_err(|e| ValueError::Other(format!("read error: {e}")))?;
        Ok(buf)
    }
}

impl Resource for StringReader {
    fn close(&self) -> ValueResult<()> {
        *self.inner.lock().unwrap() = None;
        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.inner.lock().unwrap().is_none()
    }

    fn resource_type(&self) -> &'static str {
        "string-reader"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ── Native builtins ──────────────────────────────────────────────────────────

pub fn register(globals: &Arc<cljrs_runtime::env::env::GlobalEnv>, ns: &str) {
    register_fns!(
        globals,
        ns,
        [
            ("reader", Arity::Fixed(1), builtin_reader),
            ("writer", Arity::Variadic { min: 1 }, builtin_writer),
            ("string-reader", Arity::Fixed(1), builtin_string_reader),
            ("close", Arity::Fixed(1), builtin_close),
            ("read-line", Arity::Fixed(1), builtin_read_line),
            ("write", Arity::Fixed(2), builtin_write),
            ("flush", Arity::Fixed(1), builtin_flush),
            ("reader?", Arity::Fixed(1), builtin_reader_q),
            ("writer?", Arity::Fixed(1), builtin_writer_q),
            ("file", Arity::Fixed(1), builtin_file),
            (
                "delete-file",
                Arity::Variadic { min: 1 },
                builtin_delete_file
            ),
            ("make-parents", Arity::Fixed(1), builtin_make_parents),
        ]
    );
}

fn builtin_reader(args: &[Value]) -> ValueResult<Value> {
    let path = match &args[0] {
        Value::Str(s) => s.get().clone(),
        v => {
            return Err(ValueError::WrongType {
                expected: "string",
                got: v.type_name().to_string(),
            });
        }
    };
    let reader = IoReader::open(&path)?;
    Ok(Value::Resource(ResourceHandle::new(reader)))
}

fn builtin_writer(args: &[Value]) -> ValueResult<Value> {
    let path = match &args[0] {
        Value::Str(s) => s.get().clone(),
        v => {
            return Err(ValueError::WrongType {
                expected: "string",
                got: v.type_name().to_string(),
            });
        }
    };
    // Check for :append option
    let append = if args.len() >= 3 {
        matches!(
            (&args[1], &args[2]),
            (Value::Keyword(k), Value::Bool(true)) if k.get().name.as_ref() == "append"
        )
    } else {
        false
    };
    let writer = IoWriter::open(&path, append)?;
    Ok(Value::Resource(ResourceHandle::new(writer)))
}

fn builtin_string_reader(args: &[Value]) -> ValueResult<Value> {
    let s = match &args[0] {
        Value::Str(s) => s.get().clone(),
        v => {
            return Err(ValueError::WrongType {
                expected: "string",
                got: v.type_name().to_string(),
            });
        }
    };
    Ok(Value::Resource(ResourceHandle::new(StringReader::new(s))))
}

fn builtin_close(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Resource(r) => {
            r.close()?;
            Ok(Value::Nil)
        }
        v => Err(ValueError::WrongType {
            expected: "resource",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_read_line(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Resource(r) => {
            if let Some(reader) = r.downcast::<IoReader>() {
                match reader.read_line()? {
                    Some(line) => Ok(Value::string(line)),
                    None => Ok(Value::Nil),
                }
            } else if let Some(reader) = r.downcast::<StringReader>() {
                match reader.read_line()? {
                    Some(line) => Ok(Value::string(line)),
                    None => Ok(Value::Nil),
                }
            } else {
                Err(ValueError::Other("not a readable resource".into()))
            }
        }
        v => Err(ValueError::WrongType {
            expected: "reader",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_write(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Resource(r) => {
            let s = match &args[1] {
                Value::Str(s) => s.get().clone(),
                v => format!("{v}"),
            };
            if let Some(writer) = r.downcast::<IoWriter>() {
                writer.write_str(&s)?;
                Ok(Value::Nil)
            } else {
                Err(ValueError::Other("not a writable resource".into()))
            }
        }
        v => Err(ValueError::WrongType {
            expected: "writer",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_flush(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Resource(r) => {
            if let Some(writer) = r.downcast::<IoWriter>() {
                writer.flush()?;
                Ok(Value::Nil)
            } else {
                Err(ValueError::Other("not a writable resource".into()))
            }
        }
        v => Err(ValueError::WrongType {
            expected: "writer",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_reader_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(&args[0], Value::Resource(r) if
        r.downcast::<IoReader>().is_some() || r.downcast::<StringReader>().is_some()
    )))
}

fn builtin_writer_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(
        &args[0],
        Value::Resource(r) if r.downcast::<IoWriter>().is_some()
    )))
}

fn builtin_file(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Str(s) => Ok(Value::string(s.get().clone())),
        v => Err(ValueError::WrongType {
            expected: "string",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_delete_file(args: &[Value]) -> ValueResult<Value> {
    let path = match &args[0] {
        Value::Str(s) => s.get().clone(),
        v => {
            return Err(ValueError::WrongType {
                expected: "string",
                got: v.type_name().to_string(),
            });
        }
    };
    let silently = args.len() >= 2 && args[1] != Value::Nil && args[1] != Value::Bool(false);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(Value::Bool(true)),
        Err(_) if silently => Ok(Value::Bool(false)),
        Err(e) => Err(ValueError::Other(format!("cannot delete {path}: {e}"))),
    }
}

fn builtin_make_parents(args: &[Value]) -> ValueResult<Value> {
    let path = match &args[0] {
        Value::Str(s) => s.get().clone(),
        v => {
            return Err(ValueError::WrongType {
                expected: "string",
                got: v.type_name().to_string(),
            });
        }
    };
    if let Some(parent) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ValueError::Other(format!("cannot create dirs: {e}")))?;
    }
    Ok(Value::Bool(true))
}

#[cfg(all(test, unix))]
mod close_tests {
    use super::*;

    #[test]
    fn close_ignores_a_final_flush_error_and_is_idempotent() {
        use std::os::fd::OwnedFd;
        let (stream, peer) = std::os::unix::net::UnixStream::pair().unwrap();
        drop(peer);
        let file = std::fs::File::from(OwnedFd::from(stream));
        let writer = IoWriter {
            inner: Arc::new(Mutex::new(Some(BufWriter::new(file)))),
            closed: Arc::new(AtomicBool::new(false)),
        };
        writer.write_str("buffered body").unwrap();
        assert!(writer.flush().is_err());
        assert!(!writer.is_closed());
        writer.close().unwrap();
        assert!(writer.is_closed());
        writer.close().unwrap();
    }
}
