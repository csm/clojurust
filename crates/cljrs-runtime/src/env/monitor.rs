//! Reentrant object monitors. A waiter holds no OS lock while parked.
use cljrs_value::parking::Event;
use cljrs_value::types::Realizer;
use cljrs_value::{MapValue, SetValue, Value, ValueError, ValueResult};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

#[derive(Default)]
struct Monitor {
    owner: Mutex<Option<(Realizer, usize)>>,
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

// Every owner/waiter keeps a registry reference. The final reference removes
// only its own entry under the table lock, without scanning unrelated objects.
struct MonitorRef {
    key: usize,
    monitor: Option<Arc<Monitor>>,
}
impl MonitorRef {
    fn monitor(&self) -> &Monitor {
        self.monitor.as_deref().unwrap()
    }
}
impl Drop for MonitorRef {
    fn drop(&mut self) {
        let monitor = self.monitor.take().unwrap();
        let mut table = MONITORS.get().unwrap().lock().unwrap();
        if Arc::strong_count(&monitor) == 1
            && table
                .get(&self.key)
                .is_some_and(|entry| entry.as_ptr() == Arc::as_ptr(&monitor))
        {
            table.remove(&self.key);
        }
        // Serialize the decrement with lookups AND other reference drops.
        // Otherwise concurrent final drops could both observe count > 1.
        drop(monitor);
    }
}

pub struct MonitorGuard(MonitorRef, std::marker::PhantomData<std::rc::Rc<()>>);
pub fn enter(value: &Value) -> ValueResult<MonitorGuard> {
    let key = key(value)?;
    let monitor = {
        let mut table = MONITORS.get_or_init(Default::default).lock().unwrap();
        match table.get(&key).and_then(Weak::upgrade) {
            Some(monitor) => monitor,
            None => {
                let monitor = Arc::new(Monitor::default());
                table.insert(key, Arc::downgrade(&monitor));
                monitor
            }
        }
    };
    let reference = MonitorRef {
        key,
        monitor: Some(monitor),
    };
    let monitor = reference.monitor();
    let owner = Realizer::current();
    loop {
        let listener = monitor.changed.listen();
        {
            let mut state = monitor.owner.lock().unwrap();
            match &mut *state {
                None => *state = Some((owner, 1)),
                Some((active, depth)) if *active == owner => *depth += 1,
                Some((active, _)) => {
                    active.ensure_can_wait().map_err(ValueError::Other)?;
                    drop(state);
                    listener.wait(None);
                    continue;
                }
            }
        }
        drop(listener);
        return Ok(MonitorGuard(reference, std::marker::PhantomData));
    }
}
impl Drop for MonitorGuard {
    fn drop(&mut self) {
        let mut state = self.0.monitor().owner.lock().unwrap();
        let (owner, depth) = state.as_mut().expect("unowned monitor");
        assert!(*owner == Realizer::current());
        *depth -= 1;
        if *depth == 0 {
            *state = None;
        }
        drop(state);
        self.0.monitor().changed.notify_all();
    }
}

struct HeldMonitor {
    guard: MonitorGuard,
    value: Value,
    #[cfg(feature = "no-gc")]
    _lease: cljrs_gc::region::RegionLease,
}
#[derive(Default)]
struct MonitorState {
    held: Vec<HeldMonitor>,
    legacy_async: bool,
}
thread_local! {
    static HELD_MONITORS: std::cell::RefCell<MonitorState> = const { std::cell::RefCell::new(MonitorState { held: Vec::new(), legacy_async: false }) };
}
cljrs_gc::mutator_slot!("cljrs-runtime/HELD_MONITORS", refcell HELD_MONITORS: MonitorState, trace trace_held);
fn trace_held(state: &MonitorState, visitor: &mut cljrs_gc::MarkVisitor) {
    use cljrs_gc::Trace;
    for entry in &state.held {
        entry.value.trace(visitor);
    }
}
#[cfg(not(feature = "no-gc"))]
pub(crate) fn trace_current(visitor: &mut cljrs_gc::MarkVisitor) {
    HELD_MONITORS.with(|held| trace_held(&held.borrow(), visitor));
}

// Legacy async tasks share the root mutator and cannot retain explicit
// ownership across a yield. Fibers have their own monitor state and can.
fn ensure_explicit_owner() -> ValueResult<()> {
    if cljrs_value::parking::current().is_none()
        && (HELD_MONITORS.with(|state| state.borrow().legacy_async)
            || crate::env::callback::current_is_async())
    {
        return Err(ValueError::Other(
            "explicit monitors require a synchronous context or a fiber".into(),
        ));
    }
    Ok(())
}

/// Mark one poll of a legacy async task. Explicit monitors are rejected even
/// when that task calls a synchronous helper. Drop restores the caller's state.
#[must_use]
pub struct AsyncMonitorScope {
    previous: bool,
    _not_send: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl AsyncMonitorScope {
    pub fn enter() -> Self {
        Self {
            previous: HELD_MONITORS
                .with(|state| std::mem::replace(&mut state.borrow_mut().legacy_async, true)),
            _not_send: std::marker::PhantomData,
        }
    }
}
impl Drop for AsyncMonitorScope {
    fn drop(&mut self) {
        HELD_MONITORS.with(|state| state.borrow_mut().legacy_async = self.previous);
    }
}

/// Acquire an explicit monitor for this computation until `monitor_exit`.
pub fn monitor_enter(value: &Value) -> ValueResult<()> {
    ensure_explicit_owner()?;
    #[cfg(not(feature = "no-gc"))]
    cljrs_gc::region::poison_active_regions();
    let guard = enter(value)?;
    let entry = HeldMonitor {
        guard,
        value: value.clone(),
        #[cfg(feature = "no-gc")]
        _lease: cljrs_gc::region::RegionLease::for_value(value),
    };
    HELD_MONITORS.with(|held| held.borrow_mut().held.push(entry));
    Ok(())
}

/// Release one explicit acquisition. Reject objects this computation does not own.
pub fn monitor_exit(value: &Value) -> ValueResult<()> {
    ensure_explicit_owner()?;
    let key = key(value)?;
    let entry = HELD_MONITORS.with(|held| {
        let mut state = held.borrow_mut();
        let held = &mut state.held;
        let index = held
            .iter()
            .rposition(|entry| entry.guard.0.key == key)
            .ok_or_else(|| {
                ValueError::Other("monitor-exit: this computation does not own the monitor".into())
            })?;
        Ok(held.remove(index))
    });
    let entry: HeldMonitor = entry?;
    drop(entry);
    Ok(())
}

/// Release unmatched explicit acquisitions on fiber completion or cancellation.
pub struct MonitorCleanup;
impl Drop for MonitorCleanup {
    fn drop(&mut self) {
        let held = HELD_MONITORS.with(|held| std::mem::take(&mut held.borrow_mut().held));
        drop(held);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_reference_removes_only_its_own_monitor_entry() {
        let locks: Vec<_> = (0..1000)
            .map(|_| Value::Atom(cljrs_gc::GcPtr::new(cljrs_value::Atom::new(Value::Nil))))
            .collect();
        let keys: Vec<_> = locks.iter().map(|value| key(value).unwrap()).collect();
        let guards: Vec<_> = locks.iter().map(|value| enter(value).unwrap()).collect();
        {
            let table = MONITORS.get().unwrap().lock().unwrap();
            assert!(keys.iter().all(|key| table.contains_key(key)));
        }
        // Reentrant release must leave the first owner's registration live.
        drop(enter(&locks[0]).unwrap());
        assert!(
            MONITORS
                .get()
                .unwrap()
                .lock()
                .unwrap()
                .contains_key(&keys[0])
        );
        drop(guards);
        let table = MONITORS.get().unwrap().lock().unwrap();
        assert!(keys.iter().all(|key| !table.contains_key(key)));
    }
}
