use cljrs_value::Value;
use std::cell::RefCell;

struct TapState {
    fns: Vec<Value>,
}

thread_local! {
    static TAP: RefCell<TapState> = const { RefCell::new(TapState { fns: Vec::new() }) };
}

pub fn add_tap(f: Value) {
    TAP.with(|tap| {
        let mut state = tap.borrow_mut();
        if !state.fns.iter().any(|existing| existing == &f) {
            state.fns.push(f);
        }
    });
}

pub fn remove_tap(f: &Value) {
    TAP.with(|tap| {
        tap.borrow_mut().fns.retain(|existing| existing != f);
    });
}

pub fn send(val: Value) -> bool {
    let fns: Vec<Value> = TAP.with(|tap| tap.borrow().fns.clone());
    if fns.is_empty() {
        return false;
    }
    for f in &fns {
        let _ = crate::callback::invoke(f, vec![val.clone()]);
    }
    true
}

/// Trace all GcPtr values in the tap system as GC roots.
pub fn trace_roots(visitor: &mut cljrs_gc::MarkVisitor) {
    use cljrs_gc::Trace;
    TAP.with(|tap| {
        for val in &tap.borrow().fns {
            val.trace(visitor);
        }
    });
}
