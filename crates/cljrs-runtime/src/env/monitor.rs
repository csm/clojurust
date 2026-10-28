//! Reentrant object monitors. A waiter holds no OS lock while parked.
use cljrs_value::parking::{self, Event};
use cljrs_value::{MapValue, SetValue, Value, ValueError, ValueResult};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Owner(std::thread::ThreadId, Option<u64>);
impl Owner {
    fn current() -> Self {
        Self(std::thread::current().id(), parking::current())
    }
}
#[derive(Default)]
struct Monitor {
    owner: Mutex<Option<(Owner, usize)>>,
    changed: Event,
}
static MONITORS: OnceLock<Mutex<HashMap<usize, Weak<Monitor>>>> = OnceLock::new();

// The caller roots the object throughout ownership, so addresses cannot be
// reused while a live monitor has an owner or a waiter.
fn key(value: &Value) -> ValueResult<usize> {
    macro_rules! ptr {
        ($p:expr) => {
            $p.get() as *const _ as usize
        };
    }
    Ok(match value {
        Value::BoundFn(p) => ptr!(p),
        Value::ObjectArray(p) => ptr!(p),
        Value::Error(p) => ptr!(p),
        Value::Uuid(p) => ptr!(p),
        Value::BigInt(p) => ptr!(p),
        Value::BigDecimal(p) => ptr!(p),
        Value::Ratio(p) => ptr!(p),
        Value::Str(p) => ptr!(p),
        Value::Symbol(p) => ptr!(p),
        Value::Keyword(p) => ptr!(p),
        Value::List(p) => ptr!(p),
        Value::Vector(p) => ptr!(p),
        Value::Queue(p) => ptr!(p),
        Value::NativeFunction(p) => ptr!(p),
        Value::Fn(p) => ptr!(p),
        Value::Macro(p) => ptr!(p),
        Value::Var(p) => ptr!(p),
        Value::Atom(p) => ptr!(p),
        Value::Namespace(p) => ptr!(p),
        Value::LazySeq(p) => ptr!(p),
        Value::Cons(p) => ptr!(p),
        Value::Protocol(p) => ptr!(p),
        Value::ProtocolFn(p) => ptr!(p),
        Value::MultiFn(p) => ptr!(p),
        Value::Volatile(p) => ptr!(p),
        Value::Delay(p) => ptr!(p),
        Value::Promise(p) => ptr!(p),
        Value::Future(p) => ptr!(p),
        Value::Agent(p) => ptr!(p),
        Value::TypeInstance(p) => ptr!(p),
        Value::NativeObject(p) => ptr!(p),
        Value::Map(MapValue::Array(p)) => ptr!(p),
        Value::Map(MapValue::Hash(p)) => ptr!(p),
        Value::Map(MapValue::Sorted(p)) => ptr!(p),
        Value::Set(SetValue::Hash(p)) => ptr!(p),
        Value::Set(SetValue::Sorted(p)) => ptr!(p),
        Value::Resource(p) => Arc::as_ptr(&p.0) as *const () as usize,
        Value::SharedAtom(p) => Arc::as_ptr(p) as usize,
        _ => return Err(ValueError::Other("locking requires an object".into())),
    })
}

pub struct MonitorGuard(Arc<Monitor>, std::marker::PhantomData<std::rc::Rc<()>>);
pub fn enter(value: &Value) -> ValueResult<MonitorGuard> {
    let key = key(value)?;
    let monitor = {
        let mut table = MONITORS.get_or_init(Default::default).lock().unwrap();
        table.retain(|_, monitor| monitor.strong_count() > 0);
        match table.get(&key).and_then(Weak::upgrade) {
            Some(monitor) => monitor,
            None => {
                let monitor = Arc::new(Monitor::default());
                table.insert(key, Arc::downgrade(&monitor));
                monitor
            }
        }
    };
    let owner = Owner::current();
    loop {
        let listener = monitor.changed.listen();
        {
            let mut state = monitor.owner.lock().unwrap();
            match &mut *state {
                None => *state = Some((owner, 1)),
                Some((active, depth)) if *active == owner => *depth += 1,
                _ => {
                    drop(state);
                    listener.wait(None);
                    continue;
                }
            }
        }
        drop(listener);
        return Ok(MonitorGuard(monitor, std::marker::PhantomData));
    }
}
impl Drop for MonitorGuard {
    fn drop(&mut self) {
        let mut state = self.0.owner.lock().unwrap();
        let (owner, depth) = state.as_mut().expect("unowned monitor");
        assert!(*owner == Owner::current());
        *depth -= 1;
        if *depth == 0 {
            *state = None;
        }
        drop(state);
        self.0.changed.notify_all();
    }
}
