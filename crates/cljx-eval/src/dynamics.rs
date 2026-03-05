//! Thread-local dynamic variable binding stack.
//!
//! `binding` forms push a frame onto `BINDING_STACK` for the duration of their
//! body; the RAII `BindingGuard` pops it on drop (handles both normal return
//! and panics).

use std::cell::RefCell;
use std::collections::HashMap;

use cljx_gc::GcPtr;
use cljx_gc::Trace as _;
use cljx_value::{Value, Var};

/// Opaque key for a Var in the binding stack (pointer identity).
/// Stable because the GC is non-moving.
pub type VarKey = usize;

pub fn var_key_of(var: &GcPtr<Var>) -> VarKey {
    var.get() as *const Var as usize
}

thread_local! {
    static BINDING_STACK: RefCell<Vec<HashMap<VarKey, Value>>> =
        const { RefCell::new(Vec::new()) };
}

// ── RAII guard ────────────────────────────────────────────────────────────────

/// Pops the innermost binding frame when dropped.
pub struct BindingGuard;

impl Drop for BindingGuard {
    fn drop(&mut self) {
        pop_frame();
    }
}

// ── Stack manipulation ────────────────────────────────────────────────────────

/// Push a new dynamic binding frame; return a guard that pops it on drop.
pub fn push_frame(bindings: HashMap<VarKey, Value>) -> BindingGuard {
    BINDING_STACK.with(|s| s.borrow_mut().push(bindings));
    BindingGuard
}

fn pop_frame() {
    BINDING_STACK.with(|s| {
        s.borrow_mut().pop();
    });
}

// ── Lookup ────────────────────────────────────────────────────────────────────

/// Check the thread-local stack first (innermost frame wins); fall back to the
/// root binding stored in the `Var` itself.
pub fn deref_var(var: &GcPtr<Var>) -> Option<Value> {
    let key = var_key_of(var);
    let tl = BINDING_STACK.with(|s| {
        s.borrow()
            .iter()
            .rev()
            .find_map(|frame| frame.get(&key).cloned())
    });
    tl.or_else(|| var.get().deref())
}

/// True if `var` has any thread-local binding on this thread.
pub fn is_thread_bound(var: &GcPtr<Var>) -> bool {
    let key = var_key_of(var);
    BINDING_STACK.with(|s| s.borrow().iter().any(|frame| frame.contains_key(&key)))
}

/// Set the innermost thread-local binding for `var`.
/// Returns `false` if no thread-local binding exists (caller should fall back
/// to setting the root).
pub fn set_thread_local(var: &GcPtr<Var>, val: Value) -> bool {
    let key = var_key_of(var);
    BINDING_STACK.with(|s| {
        for frame in s.borrow_mut().iter_mut().rev() {
            if let std::collections::hash_map::Entry::Occupied(mut e) = frame.entry(key) {
                e.insert(val);
                return true;
            }
        }
        false
    })
}

// ── Binding conveyance ────────────────────────────────────────────────────────

/// Snapshot the current thread's entire binding stack (for conveyance into a
/// child thread, e.g. `future`).
pub fn capture_current() -> Vec<HashMap<VarKey, Value>> {
    BINDING_STACK.with(|s| s.borrow().clone())
}

/// Install a previously captured snapshot on the current (new) thread.
pub fn install_frames(frames: Vec<HashMap<VarKey, Value>>) {
    BINDING_STACK.with(|s| *s.borrow_mut() = frames);
}

// ── GC root tracing ───────────────────────────────────────────────────────────

/// Trace all values in the current thread's binding stack as GC roots.
/// Call this during the GC root enumeration phase.
pub fn trace_current(visitor: &mut cljx_gc::MarkVisitor) {
    BINDING_STACK.with(|s| {
        for frame in s.borrow().iter() {
            for val in frame.values() {
                val.trace(visitor);
            }
        }
    });
}
