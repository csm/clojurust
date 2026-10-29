//! Thread-local dynamic variable binding stack.
//!
//! `binding` forms push a frame onto `BINDING_STACK` for the duration of their
//! body; the RAII `BindingGuard` pops it on drop (handles both normal return
//! and panics).

use std::cell::RefCell;
use std::collections::HashMap;

use cljrs_gc::GcPtr;
use cljrs_gc::Trace as _;
use cljrs_value::{Value, Var};

/// Opaque key for a Var in the binding stack (pointer identity).
/// Stable because the GC is non-moving.
pub type VarKey = usize;

pub fn var_key_of(var: &GcPtr<Var>) -> VarKey {
    var.get() as *const Var as usize
}

struct BindingFrame {
    values: HashMap<VarKey, Value>,
    #[cfg(feature = "no-gc")]
    owner: cljrs_gc::region::AllocationOwner,
}
impl BindingFrame {
    fn new(values: HashMap<VarKey, Value>) -> Self {
        Self {
            values,
            #[cfg(feature = "no-gc")]
            owner: cljrs_gc::region::AllocationOwner::current(),
        }
    }
}

thread_local! {
    static BINDING_STACK: RefCell<Vec<BindingFrame>> =
        const { RefCell::new(Vec::new()) };
}
cljrs_gc::mutator_slot!("cljrs-runtime/BINDING_STACK", refcell BINDING_STACK: Vec<BindingFrame>, trace trace_saved);

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
    BINDING_STACK.with(|s| s.borrow_mut().push(BindingFrame::new(bindings)));
    BindingGuard
}

/// Pop the frame `guard` owns and hand it back, so a caller that yields between
/// uses (the async evaluator) can keep a `binding` frame off the thread-local
/// stack while other tasks on the same thread run, and push it again later.
/// Must be called with the innermost frame's guard.
pub fn take_frame(guard: BindingGuard) -> HashMap<VarKey, Value> {
    std::mem::forget(guard);
    BINDING_STACK.with(|s| {
        s.borrow_mut()
            .pop()
            .map(|frame| frame.values)
            .unwrap_or_default()
    })
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
            .find_map(|frame| frame.values.get(&key).cloned())
    });
    tl.or_else(|| var.get().deref())
}

/// True if `var` has any thread-local binding on this thread.
pub fn is_thread_bound(var: &GcPtr<Var>) -> bool {
    let key = var_key_of(var);
    BINDING_STACK.with(|s| {
        s.borrow()
            .iter()
            .any(|frame| frame.values.contains_key(&key))
    })
}

/// Set the innermost thread-local binding for `var`.
/// Returns `false` if no thread-local binding exists (caller should fall back
/// to setting the root).
pub fn set_thread_local(var: &GcPtr<Var>, val: Value) -> bool {
    let key = var_key_of(var);
    #[cfg(feature = "no-gc")]
    {
        let target = BINDING_STACK.with(|stack| {
            stack
                .borrow()
                .iter()
                .enumerate()
                .rev()
                .find(|(_, frame)| frame.values.contains_key(&key))
                .map(|(index, frame)| (index, frame.owner.clone()))
        });
        if let Some((index, owner)) = target {
            // The binding frame can outlive this function's scratch scope.
            // Trace outside the stack borrow; values can contain bound functions.
            let val = if owner.is_permanent() {
                cljrs_value::publish::publish_value(val)
            } else {
                owner.protect(&val);
                val
            };
            BINDING_STACK.with(|stack| {
                stack.borrow_mut()[index].values.insert(key, val);
            });
            true
        } else {
            false
        }
    }
    #[cfg(not(feature = "no-gc"))]
    BINDING_STACK.with(|s| {
        for frame in s.borrow_mut().iter_mut().rev() {
            if let std::collections::hash_map::Entry::Occupied(mut e) = frame.values.entry(key) {
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
    BINDING_STACK.with(|s| {
        s.borrow()
            .iter()
            .map(|frame| frame.values.clone())
            .collect()
    })
}

/// Install a previously captured snapshot on the current (new) thread.
pub fn install_frames(frames: Vec<HashMap<VarKey, Value>>) {
    BINDING_STACK.with(|s| *s.borrow_mut() = frames.into_iter().map(BindingFrame::new).collect());
}

// ── GC root tracing ───────────────────────────────────────────────────────────

/// Trace all values in the current thread's binding stack as GC roots.
/// Call this during the GC root enumeration phase.
pub fn trace_current(visitor: &mut cljrs_gc::MarkVisitor) {
    BINDING_STACK.with(|s| {
        for frame in s.borrow().iter() {
            for val in frame.values.values() {
                val.trace(visitor);
            }
        }
    });
}

fn trace_saved(frames: &Vec<BindingFrame>, visitor: &mut cljrs_gc::MarkVisitor) {
    for frame in frames {
        for value in frame.values.values() {
            value.trace(visitor);
        }
    }
}
