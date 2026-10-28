//! Per-mutator state: the thread-locals that belong to *a running
//! computation* rather than to the isolate as a whole.
//!
//! Virtual threads (`docs/virtual-threads-plan.md`) run many fibers on one
//! isolate thread.  Root shadow stacks, region stacks, dynamic binding
//! frames, eval contexts, gas meters, … are LIFO structures owned by one
//! computation; letting two fibers interleave pushes onto them would corrupt
//! both.  Each such thread-local is therefore registered here as a
//! [`MutatorSlot`], and a [`MutatorState`] holds one value per slot.  A
//! scheduler swaps a fiber's `MutatorState` into the thread-locals before
//! resuming it and back out after it parks.
//!
//! Slots are collected at link time with [`inventory`], so every crate in the
//! binary contributes its slots before `main` runs and the set never changes
//! afterwards — a saved state can never miss a slot registered later.
//!
//! A swap is `std::mem::swap` on each slot's value: for the `Vec`-backed
//! stacks that is a pointer exchange, so a context switch is O(number of
//! slots), never O(stack depth).
//!
//! # Declaring a slot
//!
//! ```ignore
//! thread_local! {
//!     static MY_STACK: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
//! }
//! cljrs_gc::mutator_slot!("my-crate/MY_STACK", refcell MY_STACK: Vec<Frame>);
//! ```
//!
//! `refcell` swaps the contents of a `RefCell<T>` (panicking if the slot is
//! borrowed — a borrow held across a context switch is a bug); `cell` swaps
//! the contents of a `Cell<T>`.  The value type must implement `Default`; the
//! default is the state a fresh fiber starts with. Add `, trace trace_saved`
//! for slots that contain GC references. The callback takes `&T` and a
//! `&mut MarkVisitor` and traces the detached value without installing it.
//!
//! Every saved state registers weakly with its isolate. Collection traces
//! those states as well as the current thread-locals. A scheduler must keep
//! stack addresses valid until it restores the state and unwinds the stack.
//!
//! # Lock and critical-section discipline
//!
//! A fiber that parks while holding a runtime lock deadlocks the next fiber
//! on the same thread.  [`TrackedMutex`] counts held guards per thread in
//! debug builds and [`assert_parkable`] checks the count (and the governor's
//! critical-section depth) is zero at a park point.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::rc::{Rc, Weak};
use std::sync::{LockResult, OnceLock, PoisonError, TryLockError, TryLockResult};

#[doc(hidden)]
pub use inventory;

/// One per-mutator thread-local, registered with [`mutator_slot!`].
pub struct MutatorSlot {
    /// Stable, unique identifier: `"<crate>/<STATIC_NAME>"`.
    pub name: &'static str,
    /// The value a fresh mutator starts with.
    pub init: fn() -> Box<dyn Any>,
    /// Exchange the thread-local's current value with `*value`.
    pub swap: fn(&mut dyn Any),
    /// Trace GC references held by a detached slot value.
    pub trace: fn(&dyn Any, &mut crate::MarkVisitor),
    /// Check borrow discipline before any slot is exchanged.
    pub check: fn(),
}

inventory::collect!(MutatorSlot);

/// All registered slots, sorted by name.  Panics on a duplicate name.
pub fn slots() -> &'static [&'static MutatorSlot] {
    static SLOTS: OnceLock<Vec<&'static MutatorSlot>> = OnceLock::new();
    SLOTS.get_or_init(|| {
        let mut slots: Vec<&'static MutatorSlot> = inventory::iter::<MutatorSlot>().collect();
        slots.sort_by_key(|s| s.name);
        for pair in slots.windows(2) {
            assert!(
                pair[0].name != pair[1].name,
                "duplicate mutator slot `{}`",
                pair[0].name
            );
        }
        slots
    })
}

/// Names of every registered slot, sorted.
pub fn slot_names() -> Vec<&'static str> {
    slots().iter().map(|s| s.name).collect()
}

/// Declare a thread-local as per-mutator state.  See the module docs.
#[macro_export]
macro_rules! mutator_slot {
    ($name:literal, refcell $tl:ident : $ty:ty $(, trace $tracer:path)?) => {
        const _: () = {
            fn check() {
                $tl.with(|cell| {
                    let _borrow = cell.try_borrow_mut().expect(concat!(
                        "mutator slot ", $name, " is borrowed across a context switch"
                    ));
                });
            }
            fn init() -> ::std::boxed::Box<dyn ::std::any::Any> {
                ::std::boxed::Box::new(<$ty as ::std::default::Default>::default())
            }
            fn swap(value: &mut dyn ::std::any::Any) {
                let value = value.downcast_mut::<$ty>().expect(concat!(
                    "mutator slot ",
                    $name,
                    ": type mismatch"
                ));
                $tl.with(|cell| {
                    let mut current = cell.try_borrow_mut().expect(concat!(
                        "mutator slot ",
                        $name,
                        " is borrowed across a context switch"
                    ));
                    ::std::mem::swap(&mut *current, value);
                });
            }
            fn trace(value: &dyn ::std::any::Any, visitor: &mut $crate::MarkVisitor) {
                let _ = (value, &visitor);
                $( $tracer(value.downcast_ref::<$ty>().expect("mutator trace type mismatch"), visitor); )?
            }
            $crate::mutator::inventory::submit! {
                $crate::mutator::MutatorSlot { name: $name, init, swap, trace, check }
            }
        };
    };
    ($name:literal, cell $tl:ident : $ty:ty $(, trace $tracer:path)?) => {
        const _: () = {
            fn check() {}
            fn init() -> ::std::boxed::Box<dyn ::std::any::Any> {
                ::std::boxed::Box::new(<$ty as ::std::default::Default>::default())
            }
            fn swap(value: &mut dyn ::std::any::Any) {
                let value = value.downcast_mut::<$ty>().expect(concat!(
                    "mutator slot ",
                    $name,
                    ": type mismatch"
                ));
                $tl.with(|cell| {
                    let incoming = ::std::mem::take(value);
                    *value = cell.replace(incoming);
                });
            }
            fn trace(value: &dyn ::std::any::Any, visitor: &mut $crate::MarkVisitor) {
                let _ = (value, &visitor);
                $( $tracer(value.downcast_ref::<$ty>().expect("mutator trace type mismatch"), visitor); )?
            }
            $crate::mutator::inventory::submit! {
                $crate::mutator::MutatorSlot { name: $name, init, swap, trace, check }
            }
        };
    };
}

/// The per-mutator state of one computation, detached from the thread.
///
/// Values are created lazily: an entry that has never been swapped in is the
/// slot's default, so [`MutatorState::fresh`] allocates nothing per slot.
pub struct MutatorState {
    values: Rc<RefCell<SlotValues>>,
    _not_send: PhantomData<*const ()>,
}

type SlotValues = Box<[Option<Box<dyn Any>>]>;

thread_local! {
    // Weak entries never keep a finished fiber or its stack roots alive.
    static DETACHED: RefCell<Vec<Weak<RefCell<SlotValues>>>> = const { RefCell::new(Vec::new()) };
}

/// Trace every detached state on this isolate thread, including a driver's
/// state while a fiber is running. Stack addresses stay valid until the fiber
/// is unwound with its state installed and the state is dropped.
#[cfg(not(feature = "no-gc"))]
pub(crate) fn trace_suspended(visitor: &mut crate::MarkVisitor) {
    DETACHED.with(|states| {
        states.borrow_mut().retain(|weak| {
            let Some(values) = weak.upgrade() else {
                return false;
            };
            for (slot, value) in slots().iter().zip(values.borrow().iter()) {
                if let Some(value) = value {
                    (slot.trace)(&**value, visitor);
                }
            }
            true
        });
    });
}

impl MutatorState {
    /// The state of a computation that has not started yet.
    pub fn fresh() -> Self {
        let values = slots().iter().map(|_| None).collect();
        Self {
            values: {
                let values = Rc::new(RefCell::new(values));
                DETACHED.with(|states| {
                    let mut states = states.borrow_mut();
                    states.retain(|w| w.strong_count() > 0);
                    states.push(Rc::downgrade(&values));
                });
                values
            },
            _not_send: PhantomData,
        }
    }

    /// Detach the current thread's per-mutator state, leaving fresh defaults
    /// in the thread-locals.
    pub fn save() -> Self {
        let mut state = Self::fresh();
        state.swap_with_current();
        state
    }

    /// Install this state as the current thread's per-mutator state.  The
    /// state it replaces is dropped; callers normally restore onto the fresh
    /// state left by [`MutatorState::save`].
    pub fn restore(mut self) {
        self.swap_with_current();
    }

    /// Exchange this state with the current thread's per-mutator state.
    ///
    /// A scheduler calls this once before resuming a fiber and once after it
    /// parks, so `self` alternates between "the fiber's state" and "the state
    /// of whoever was running before".
    pub fn swap_with_current(&mut self) {
        check_slots();
        for (slot, value) in slots().iter().zip(self.values.borrow_mut().iter_mut()) {
            let value = value.get_or_insert_with(slot.init);
            (slot.swap)(&mut **value);
        }
    }
}

impl Default for MutatorState {
    fn default() -> Self {
        Self::fresh()
    }
}

// ── Park-point discipline ───────────────────────────────────────────────────

thread_local! {
    /// Number of [`TrackedMutex`] guards held by the running computation.
    /// Always zero at a park point, so it is per-thread, not per-mutator.
    static GUARDS_HELD: Cell<usize> = const { Cell::new(0) };
}

/// Number of [`TrackedMutex`] guards the current thread holds.  Always zero
/// in release builds, where guards are not counted.
pub fn guards_held() -> usize {
    GUARDS_HELD.with(Cell::get)
}

/// Assert that the running computation may park: it holds no runtime lock
/// and is not inside a governor critical section (debug builds). Borrowed
/// mutator slots are rejected in every build, before a switch changes state.
#[track_caller]
pub fn assert_parkable() {
    check_slots();
    if cfg!(debug_assertions) {
        let held = guards_held();
        assert!(
            held == 0,
            "fiber parked while holding {held} runtime lock guard(s)"
        );
        assert!(
            !crate::governor::in_critical_section(),
            "fiber parked inside a governor critical section"
        );
    }
}

fn check_slots() {
    for slot in slots() {
        (slot.check)();
    }
}

/// A `std::sync::Mutex` whose guards are counted per thread in debug builds,
/// so a park point can assert no runtime lock is held across it.
///
/// Mirrors the subset of the `std::sync::Mutex` API the runtime uses.
#[derive(Default)]
pub struct TrackedMutex<T: ?Sized> {
    inner: std::sync::Mutex<T>,
}

impl<T> TrackedMutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            inner: std::sync::Mutex::new(value),
        }
    }

    pub fn into_inner(self) -> LockResult<T> {
        self.inner.into_inner()
    }
}

impl<T: ?Sized> TrackedMutex<T> {
    pub fn lock(&self) -> LockResult<TrackedMutexGuard<'_, T>> {
        match self.inner.lock() {
            Ok(guard) => Ok(TrackedMutexGuard::new(guard)),
            Err(poison) => Err(PoisonError::new(TrackedMutexGuard::new(
                poison.into_inner(),
            ))),
        }
    }

    pub fn try_lock(&self) -> TryLockResult<TrackedMutexGuard<'_, T>> {
        match self.inner.try_lock() {
            Ok(guard) => Ok(TrackedMutexGuard::new(guard)),
            Err(TryLockError::Poisoned(poison)) => Err(TryLockError::Poisoned(PoisonError::new(
                TrackedMutexGuard::new(poison.into_inner()),
            ))),
            Err(TryLockError::WouldBlock) => Err(TryLockError::WouldBlock),
        }
    }

    pub fn get_mut(&mut self) -> LockResult<&mut T> {
        self.inner.get_mut()
    }

    pub fn is_poisoned(&self) -> bool {
        self.inner.is_poisoned()
    }
}

impl<T: ?Sized + std::fmt::Debug> std::fmt::Debug for TrackedMutex<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(f)
    }
}

/// Guard returned by [`TrackedMutex::lock`].
pub struct TrackedMutexGuard<'a, T: ?Sized> {
    guard: std::sync::MutexGuard<'a, T>,
}

impl<'a, T: ?Sized> TrackedMutexGuard<'a, T> {
    fn new(guard: std::sync::MutexGuard<'a, T>) -> Self {
        if cfg!(debug_assertions) {
            GUARDS_HELD.with(|n| n.set(n.get() + 1));
        }
        Self { guard }
    }
}

impl<T: ?Sized> Drop for TrackedMutexGuard<'_, T> {
    fn drop(&mut self) {
        if cfg!(debug_assertions) {
            // `try_with`: a guard dropped during thread teardown must not
            // panic on the already-destroyed counter.
            let _ = GUARDS_HELD.try_with(|n| n.set(n.get().saturating_sub(1)));
        }
    }
}

impl<T: ?Sized> Deref for TrackedMutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T: ?Sized> DerefMut for TrackedMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T: ?Sized + std::fmt::Debug> std::fmt::Debug for TrackedMutexGuard<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.guard.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static TEST_STACK: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
        static TEST_COUNTER: Cell<u64> = const { Cell::new(0) };
    }
    crate::mutator_slot!("cljrs-gc/TEST_STACK", refcell TEST_STACK: Vec<u32>);
    crate::mutator_slot!("cljrs-gc/TEST_COUNTER", cell TEST_COUNTER: u64);

    fn stack() -> Vec<u32> {
        TEST_STACK.with(|s| s.borrow().clone())
    }

    #[test]
    fn slots_are_registered_and_sorted() {
        let names = slot_names();
        assert!(names.contains(&"cljrs-gc/TEST_STACK"));
        assert!(names.contains(&"cljrs-gc/TEST_COUNTER"));
        assert!(names.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn save_restore_round_trips() {
        TEST_STACK.with(|s| s.borrow_mut().extend([1, 2, 3]));
        TEST_COUNTER.with(|c| c.set(7));

        let saved = MutatorState::save();
        assert!(stack().is_empty());
        assert_eq!(TEST_COUNTER.with(Cell::get), 0);

        TEST_STACK.with(|s| s.borrow_mut().push(99));
        let mut other = MutatorState::save();

        saved.restore();
        assert_eq!(stack(), vec![1, 2, 3]);
        assert_eq!(TEST_COUNTER.with(Cell::get), 7);

        // Swapping alternates between the two computations.
        other.swap_with_current();
        assert_eq!(stack(), vec![99]);
        other.swap_with_current();
        assert_eq!(stack(), vec![1, 2, 3]);

        TEST_STACK.with(|s| s.borrow_mut().clear());
        TEST_COUNTER.with(|c| c.set(0));
    }

    #[test]
    #[should_panic(expected = "borrowed across a context switch")]
    fn swapping_a_borrowed_slot_panics() {
        TEST_STACK.with(|s| {
            let _held = s.borrow();
            let mut state = MutatorState::fresh();
            state.swap_with_current();
        });
    }

    #[test]
    fn tracked_mutex_counts_guards() {
        let m = TrackedMutex::new(5);
        let before = guards_held();
        {
            let g = m.lock().unwrap();
            assert_eq!(*g, 5);
            if cfg!(debug_assertions) {
                assert_eq!(guards_held(), before + 1);
            }
        }
        assert_eq!(guards_held(), before);
        assert_parkable();
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "runtime lock guard")]
    fn parking_with_a_guard_held_panics() {
        let m = TrackedMutex::new(());
        let _g = m.lock().unwrap();
        assert_parkable();
    }
}
