use std::any::Any;
use std::sync::{Arc, Mutex};
use tokio::runtime;
use cljrs_value::{Arity, NativeObject, NativeObjectBox, Value, ValueError, ValueResult};
use lazy_static::lazy_static;
use cljrs_gc::{GcPtr, MarkVisitor, Trace};
use crate::register_fns;

/*
 * Sketch of the overall design:
 *
 * Lazily initialized tokio Runtime (multi-threaded) that's global for the app.
 *
 * go macro pushes a Runtime::enter to a global state (thread local?) and async
 * operations <! >! are performed via Runtime::spawn.
 *
 * Use native tokio channels as channels.
 */

#[derive(Clone, Debug)]
enum Buffer {
    Fixed(u16),
    Dropping(u16),
    Sliding(u16),
}

#[derive(Clone, Debug)]
struct BufferWrapper {
    buffer: Buffer,
}

impl Trace for BufferWrapper {
    fn trace(&self, _visitor: &mut MarkVisitor) {}
}

impl NativeObject for BufferWrapper {
    fn type_tag(&self) -> &str {
        "Buffer"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug, Clone)]
struct ManyToOneChannel {
    sender: Arc<Mutex<tokio::sync::mpsc::Sender<Value>>>,
    receiver: Arc<Mutex<tokio::sync::mpsc::Receiver<Value>>>,
    buffer: Buffer,
    closed: Arc<Mutex<bool>>,
}

#[derive(Debug)]
enum PromiseState {
    Open,
    Complete,
    Closed
}

#[derive(Debug, Clone)]
struct PromiseChannel {
    sender: Arc<Mutex<tokio::sync::oneshot::Sender<Value>>>,
    receiver: Arc<Mutex<tokio::sync::oneshot::Receiver<Value>>>,
    state: Arc<Mutex<PromiseState>>,
}

#[derive(Debug, Clone)]
enum Channel {
    ManyToOne(ManyToOneChannel),
    Promise(PromiseChannel),
}

impl Trace for Channel {
    fn trace(&self, _visitor: &mut MarkVisitor) {}
}

impl NativeObject for Channel {
    fn type_tag(&self) -> &str {
        match self {
            Channel::ManyToOne(_) => "ManyToOneChannel",
            Channel::Promise(_) => "PromiseChannel",
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn register(globals: &Arc<cljrs_eval::GlobalEnv>, ns: &str) {
    register_fns!(
        globals,
        ns,
        [
            ("enter-async", Arity::Fixed(0), enter_async),
            ("exit-async", Arity::Fixed(0), exit_async),
            ("chan", Arity::Variadic { min: 0 }, chan),
            ("promise-chan", Arity::Fixed(0), promise_chan),
            ("buffer", Arity::Fixed(1), buffer),
            ("dropping-buffer", Arity::Fixed(1), dropping_buffer),
            ("sliding-buffer", Arity::Fixed(1), sliding_buffer),
        ]
    );
}

lazy_static!{
    static ref RUNTIME: runtime::Runtime = runtime::Runtime::new().unwrap();
}

pub fn enter_async(_args: &[Value]) -> ValueResult<Value> {
    todo!()
}

pub fn exit_async(_args: &[Value]) -> ValueResult<Value> {
    todo!()
}

pub fn chan(args: &[Value]) -> ValueResult<Value> {
    let (buffer, size) = match args.len() {
        1 => (Buffer::Fixed(1024), 1024),
        2 | 3 => match &args[1] {
            Value::Long(size) =>
                if *size > 0 && *size < 0xFFFF {
                    (Buffer::Fixed(*size as u16), *size as u16)
                } else {
                    return Err(ValueError::OutOfRange)
                }
            Value::NativeObject(obj) if obj.get().type_tag() == "Buffer" => {
                let buffer: &BufferWrapper = obj.get().downcast_ref::<BufferWrapper>().ok_or_else(||
                    ValueError::Other("not a buffer".to_string())
                )?;
                match buffer.buffer {
                    Buffer::Fixed(s) => (buffer.buffer.clone(), s),
                    Buffer::Dropping(s) => (buffer.buffer.clone(), s),
                    Buffer::Sliding(s) => (buffer.buffer.clone(), s)
                }
            }
            v => return Err(ValueError::WrongType {
                expected: "long or buffer",
                got: v.type_name().to_string()
            })
        }
        _ => return Err(ValueError::Other("this statement shouldn't be reached".to_string()))
    };
    let (sender, receiver) = tokio::sync::mpsc::channel(1024);
    Ok(
        Value::NativeObject(
            GcPtr::new(
                NativeObjectBox::new(
                    Channel::ManyToOne(ManyToOneChannel {
                        sender: Arc::new(Mutex::new(sender)),
                        receiver: Arc::new(Mutex::new(receiver)),
                        buffer,
                        closed: Arc::new(Mutex::new(false)),
                    })
                )
            )
        )
    )
}

pub fn promise_chan(_args: &[Value]) -> ValueResult<Value> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    Ok(
        Value::NativeObject(
            GcPtr::new(
                NativeObjectBox::new(
                    Channel::Promise(PromiseChannel {
                        sender: Arc::new(Mutex::new(sender)),
                        receiver: Arc::new(Mutex::new(receiver)),
                        state: Arc::new(Mutex::new(PromiseState::Open))
                    })
                )
            )
        )
    )
}

pub fn buffer(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Long(size ) =>
            if *size >= 0 && *size < 0xffff {
                Ok(Value::NativeObject(GcPtr::new(NativeObjectBox::new(BufferWrapper { buffer: Buffer::Fixed(*size as u16)}))))
            } else {
                Err(ValueError::OutOfRange)
            }
        v => Err(ValueError::WrongType {
            expected: "number",
            got: v.type_name().to_string(),
        })
    }
}

pub fn sliding_buffer(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Long(size ) =>
            if *size >= 0 && *size < 0xffff {
                Ok(Value::NativeObject(GcPtr::new(NativeObjectBox::new(BufferWrapper { buffer: Buffer::Sliding(*size as u16)}))))
            } else {
                Err(ValueError::OutOfRange)
            }
        v => Err(ValueError::WrongType {
            expected: "number",
            got: v.type_name().to_string(),
        })
    }
}

pub fn dropping_buffer(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Long(size ) =>
            if *size >= 0 && *size < 0xffff {
                Ok(Value::NativeObject(GcPtr::new(NativeObjectBox::new(BufferWrapper { buffer: Buffer::Dropping(*size as u16)}))))
            } else {
                Err(ValueError::OutOfRange)
            }
        v => Err(ValueError::WrongType {
            expected: "number",
            got: v.type_name().to_string(),
        })
    }
}

pub fn put_bang(args: &[Value]) -> ValueResult<Value> {
    let arg = match &args[1] {
        Value::Nil => return Err(ValueError::Other("attempt to put nil on a channel".to_string())),
        v => v.clone()
    };
    let callback = if args.len() >= 3 {
        Some(args[2].clone())
    } else {
        None
    };
    match &args[0] {
        Value::NativeObject(obj) if obj.get().type_tag() == "ManyToOneChannel" || obj.get().type_tag() == "PromiseChannel" => {
            let channel: &Channel = obj.get().downcast_ref::<Channel>().ok_or_else(|| ValueError::Other("not a channel".to_string()))?;
            let open = match channel {
                Channel::ManyToOne(ch) =>
                    !ch.closed.lock().unwrap().clone(),
                Channel::Promise(ch) =>
                    matches!(*ch.state.lock().unwrap(), PromiseState::Open),
            };
            if open {
                let channel = channel.clone();
                tokio::spawn(async move {
                    match channel {
                        Channel::ManyToOne(ch) => {
                            let sender = ch.sender.lock().unwrap();
                            sender.capacity();
                            let _ = sender.send(arg).await;
                        }
                        Channel::Promise(ch) => {
                            let state = ch.state.lock().unwrap();
                            if matches!(*state, PromiseState::Open) {
                                let sender = ch.sender.lock().unwrap();
                                let _ = sender.send(arg);
                            };
                        },
                    };
                    if let Some(callback) = callback {
                        let _ = cljrs_eval::callback::invoke(&callback, vec![]);
                    }
                });
                Ok(Value::Bool(true))
            } else {
                Ok(Value::Bool(false))
            }
        }
        v => Err(ValueError::WrongType {
            expected: "channel",
            got: v.type_name().to_string(),
        })
    }
}