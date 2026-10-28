//! Cooperative execution-credit metering shared by every evaluation tier.
//!
//! Meter installation is thread-local; compiled async state machines explicitly
//! capture and reinstall the active meter stack when they are spawned. Nested meters
//! are charged together so inner evaluations cannot escape an outer budget.
//! Native code reports exhaustion through per-scope sticky thread-local flags,
//! allowing the signal to survive callback/JIT bridge boundaries without
//! contaminating a healthy enclosing or subsequent scope.
//!
//! The same checkpoints also observe *interrupt flags*: an embedder (e.g. the
//! nREPL server) installs an `Arc<AtomicBool>` with [`InterruptGuard`] for the
//! extent of an evaluation, and another thread sets it to stop that
//! evaluation. A set flag makes every subsequent charge fail exactly like an
//! exhausted budget, so the evaluation unwinds through the same uncatchable
//! [`EvalError::GasExhausted`](crate::env::error::EvalError::GasExhausted)
//! path in every tier (tree-walker, IR interpreter, JIT). Use
//! [`interrupt_requested`] to tell an interrupt from real exhaustion. When no
//! interrupt is pending the cost is one relaxed atomic load per charge.
//!
//! An interrupt is scoped to the evaluation, not to the thread: an async task
//! is polled under the flags of the evaluation that spawned it
//! ([`active_interrupts`], [`InterruptScope`]), so interrupting whichever
//! evaluation is driving the executor leaves other evaluations' tasks alone.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// A shareable, monotonically-decreasing execution-credit budget.
#[derive(Debug)]
pub struct GasMeter {
    remaining: AtomicU64,
}

impl GasMeter {
    pub fn new(credits: u64) -> Arc<Self> {
        Arc::new(Self {
            remaining: AtomicU64::new(credits),
        })
    }

    pub fn remaining(&self) -> u64 {
        self.remaining.load(Ordering::Relaxed)
    }

    /// Consume `cost` credits, returning false without partially charging when
    /// the budget cannot cover the whole checkpoint.
    pub fn charge(&self, cost: u64) -> bool {
        self.remaining
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                remaining.checked_sub(cost)
            })
            .is_ok()
    }
}

thread_local! {
    static ACTIVE: RefCell<Vec<Arc<GasMeter>>> = const { RefCell::new(Vec::new()) };
    static EXHAUSTED: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
    static INTERRUPTS: RefCell<Vec<Arc<AtomicBool>>> = const { RefCell::new(Vec::new()) };
}
cljrs_gc::mutator_slot!("cljrs-runtime/GAS_ACTIVE", refcell ACTIVE: Vec<Arc<GasMeter>>);
cljrs_gc::mutator_slot!("cljrs-runtime/GAS_EXHAUSTED", refcell EXHAUSTED: Vec<bool>);
cljrs_gc::mutator_slot!("cljrs-runtime/GAS_INTERRUPTS", refcell INTERRUPTS: Vec<Arc<AtomicBool>>);

/// Installs an interrupt flag for the dynamic extent of an evaluation on
/// this thread. While installed, setting the flag (from any thread) makes the
/// next execution-credit checkpoint fail, unwinding the evaluation as
/// `GasExhausted`; [`interrupt_requested`] then reports true.
#[must_use = "dropping InterruptGuard immediately uninstalls the interrupt flag"]
pub struct InterruptGuard;

impl InterruptGuard {
    pub fn install(flag: Arc<AtomicBool>) -> Self {
        INTERRUPTS.with(|flags| flags.borrow_mut().push(flag));
        Self
    }
}

impl Drop for InterruptGuard {
    fn drop(&mut self) {
        INTERRUPTS.with(|flags| {
            flags.borrow_mut().pop();
        });
    }
}

/// True when an interrupt flag installed on this thread has been set.
pub fn interrupt_requested() -> bool {
    INTERRUPTS.with(|flags| {
        flags
            .borrow()
            .iter()
            .any(|flag| flag.load(Ordering::Relaxed))
    })
}

/// Clone the interrupt flags installed on this thread for async task
/// propagation, as [`active_meters`] does for gas meters.
pub fn active_interrupts() -> Vec<Arc<AtomicBool>> {
    INTERRUPTS.with(|flags| flags.borrow().clone())
}

/// Makes `flags` the only interrupt flags this thread observes, until dropped.
///
/// An interrupt belongs to one evaluation, but the flag stack is per thread,
/// and every task on a `LocalSet` is polled on the thread of whichever
/// evaluation happens to be driving it. A task polled inside an interrupted
/// evaluation's extent would otherwise fail its next charge, though nobody
/// interrupted the evaluation that spawned it. So a task is polled under the
/// flags captured where it was spawned, replacing (not stacking on) the
/// driver's, which are put back when the poll ends.
#[must_use = "dropping InterruptScope immediately restores the previous interrupt flags"]
pub struct InterruptScope {
    outer: Vec<Arc<AtomicBool>>,
}

impl InterruptScope {
    pub fn enter(flags: &[Arc<AtomicBool>]) -> Self {
        let outer = INTERRUPTS.with(|installed| installed.replace(flags.to_vec()));
        Self { outer }
    }
}

impl Drop for InterruptScope {
    fn drop(&mut self) {
        INTERRUPTS.with(|installed| {
            installed.replace(std::mem::take(&mut self.outer));
        });
    }
}

/// Installs a meter for the dynamic extent of an evaluation.
#[must_use = "dropping GasGuard immediately uninstalls the active gas meter"]
pub struct GasGuard;

impl GasGuard {
    pub fn install(meter: Arc<GasMeter>) -> Self {
        ACTIVE.with(|active| active.borrow_mut().push(meter));
        EXHAUSTED.with(|exhausted| exhausted.borrow_mut().push(false));
        Self
    }
}

impl Drop for GasGuard {
    fn drop(&mut self) {
        ACTIVE.with(|active| {
            let mut active = active.borrow_mut();
            active.pop();
        });
        EXHAUSTED.with(|exhausted| {
            exhausted.borrow_mut().pop();
        });
    }
}

/// Charge the active evaluation, or succeed at no cost when unmetered.
pub fn charge(cost: u64) -> bool {
    if interrupt_requested() || is_exhausted() {
        return false;
    }
    let charged = ACTIVE.with(|active| {
        let active = active.borrow();
        if active.is_empty() {
            return true;
        }
        if active.iter().any(|meter| meter.remaining() < cost) {
            return false;
        }
        active.iter().all(|meter| meter.charge(cost))
    });
    if !charged {
        ACTIVE.with(|active| {
            let active = active.borrow();
            EXHAUSTED.with(|exhausted| {
                for (index, meter) in active.iter().enumerate() {
                    if meter.remaining() < cost {
                        exhausted.borrow_mut()[index] = true;
                    }
                }
            });
        });
    }
    charged
}

/// Peek at the native-tier exhaustion signal set by a failed charge.
pub fn is_exhausted() -> bool {
    interrupt_requested()
        || EXHAUSTED.with(|exhausted| exhausted.borrow().iter().any(|value| *value))
}

/// Clone the complete active meter stack for async task propagation.
pub fn active_meters() -> Vec<Arc<GasMeter>> {
    ACTIVE.with(|active| active.borrow().clone())
}

/// Install a captured meter stack in outer-to-inner order.
pub fn install_meters(meters: &[Arc<GasMeter>]) -> Vec<GasGuard> {
    meters.iter().cloned().map(GasGuard::install).collect()
}

/// Take the native-tier exhaustion signal set by a failed charge.
///
/// Prefer [`is_exhausted`] at dispatch boundaries; this remains available for
/// tests and rare code that intentionally owns the current gas scope.
pub fn take_exhausted() -> bool {
    EXHAUSTED.with(|exhausted| {
        let mut exhausted = exhausted.borrow_mut();
        let was_exhausted = exhausted.iter().any(|value| *value);
        exhausted.fill(false);
        was_exhausted
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_meter_charges_without_partial_consumption() {
        let meter = GasMeter::new(3);
        let _guard = GasGuard::install(meter.clone());
        assert!(charge(2));
        assert!(!charge(2));
        assert_eq!(meter.remaining(), 1);
    }

    #[test]
    fn nested_meters_charge_outer_budget() {
        let outer = GasMeter::new(3);
        let _outer_guard = GasGuard::install(outer.clone());
        let inner = GasMeter::new(2);
        let _inner_guard = GasGuard::install(inner.clone());
        assert!(charge(2));
        assert_eq!(outer.remaining(), 1);
        assert_eq!(inner.remaining(), 0);
        assert!(!charge(1));
    }

    #[test]
    fn inner_exhaustion_does_not_poison_healthy_outer_scope() {
        let outer = GasMeter::new(10);
        let _outer_guard = GasGuard::install(outer.clone());
        {
            let inner = GasMeter::new(0);
            let _inner_guard = GasGuard::install(inner);
            assert!(!charge(1));
            assert!(is_exhausted());
        }
        assert!(!is_exhausted());
        assert!(charge(1));
        assert_eq!(outer.remaining(), 9);
    }

    #[test]
    fn interrupt_flag_fails_charges_until_uninstalled() {
        let flag = Arc::new(AtomicBool::new(false));
        {
            let _guard = InterruptGuard::install(flag.clone());
            assert!(charge(1));
            flag.store(true, Ordering::Relaxed);
            assert!(!charge(1));
            assert!(is_exhausted());
            assert!(interrupt_requested());
        }
        assert!(!interrupt_requested());
        assert!(!is_exhausted());
        assert!(charge(1));
    }

    #[test]
    fn interrupt_scope_replaces_the_installed_flags_and_restores_them() {
        let driver = Arc::new(AtomicBool::new(true));
        let _guard = InterruptGuard::install(driver.clone());
        assert!(!charge(1));
        {
            // A task spawned by an evaluation nobody interrupted.
            let _scope = InterruptScope::enter(&[]);
            assert!(!interrupt_requested());
            assert!(charge(1));
        }
        {
            // A task spawned by the interrupted evaluation itself.
            let _scope = InterruptScope::enter(&[driver]);
            assert!(!charge(1));
        }
        assert!(interrupt_requested());
        assert_eq!(active_interrupts().len(), 1);
    }

    proptest::proptest! {
        /// Inside a scope the thread observes the scope's flags and no
        /// others; once it is dropped, the flags installed before it,
        /// unchanged. Holds for any flags, nested scopes included.
        #[test]
        fn interrupt_scope_observes_exactly_its_own_flags(
            installed in proptest::collection::vec(proptest::bool::ANY, 0..4),
            scoped in proptest::collection::vec(proptest::bool::ANY, 0..4),
            nested in proptest::collection::vec(proptest::bool::ANY, 0..4),
        ) {
            use proptest::prop_assert_eq;
            let flags = |set: &[bool]| -> Vec<Arc<AtomicBool>> {
                set.iter().map(|b| Arc::new(AtomicBool::new(*b))).collect()
            };
            let any = |set: &[bool]| set.iter().any(|b| *b);

            let _guards: Vec<InterruptGuard> = flags(&installed)
                .into_iter()
                .map(InterruptGuard::install)
                .collect();
            prop_assert_eq!(interrupt_requested(), any(&installed));
            {
                let _scope = InterruptScope::enter(&flags(&scoped));
                prop_assert_eq!(interrupt_requested(), any(&scoped));
                prop_assert_eq!(charge(1), !any(&scoped));
                {
                    let _nested = InterruptScope::enter(&flags(&nested));
                    prop_assert_eq!(interrupt_requested(), any(&nested));
                    prop_assert_eq!(active_interrupts().len(), nested.len());
                }
                prop_assert_eq!(interrupt_requested(), any(&scoped));
                prop_assert_eq!(active_interrupts().len(), scoped.len());
            }
            prop_assert_eq!(interrupt_requested(), any(&installed));
            prop_assert_eq!(active_interrupts().len(), installed.len());
        }
    }
}
