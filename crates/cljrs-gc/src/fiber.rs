//! Executor-independent fiber safepoints, diagnostics, and stack accounting.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use crate::{MemoryCharge, MemoryClass, MemoryLimitExceeded, governor};

#[derive(Debug)]
pub enum SafepointError {
    StackOverflow(String),
    /// Cancellation caught at an automatic yield, before it can cross a C ABI frame.
    Cancelled,
}

pub struct Hooks {
    pub safepoint: fn() -> Result<bool, SafepointError>,
    pub yield_now: fn() -> Result<(), SafepointError>,
    pub reclaim_stacks: fn(),
    pub enter_native: fn(&Arc<str>) -> usize,
    pub exit_native: fn(usize),
}

static HOOKS: OnceLock<Hooks> = OnceLock::new();
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static POOLED: AtomicUsize = AtomicUsize::new(0);
static RESERVED: AtomicUsize = AtomicUsize::new(0);

/// Install the native driver once. Without a driver safepoints are no-ops.
pub fn install(hooks: Hooks) {
    let _ = HOOKS.set(hooks);
}

/// Cooperate with the scheduler and check the running fiber's stack margin.
pub fn safepoint() -> Result<(), SafepointError> {
    safepoint_with(|yield_now| yield_now())
}

/// Install temporary roots only if the scheduler is about to yield.
pub fn safepoint_with(
    around_yield: impl FnOnce(fn() -> Result<(), SafepointError>) -> Result<(), SafepointError>,
) -> Result<(), SafepointError> {
    if let Some(hooks) = HOOKS.get()
        && (hooks.safepoint)()?
    {
        around_yield(hooks.yield_now)
    } else {
        Ok(())
    }
}

pub(crate) fn reclaim_idle_stacks() {
    if let Some(hooks) = HOOKS.get() {
        (hooks.reclaim_stacks)();
    }
}

/// Name the innermost native call for stall diagnostics. May span parks.
pub fn native_frame(name: &Arc<str>) -> NativeFrameGuard {
    let previous = HOOKS.get().map_or(0, |hooks| (hooks.enter_native)(name));
    NativeFrameGuard {
        previous,
        _thread: std::marker::PhantomData,
    }
}

pub struct NativeFrameGuard {
    previous: usize,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Drop for NativeFrameGuard {
    fn drop(&mut self) {
        if let Some(hooks) = HOOKS.get() {
            (hooks.exit_native)(self.previous);
        }
    }
}

/// Bytes of each stack charged to the governor as committed memory. Stacks
/// are reserved address space committed lazily by the OS, so charging the
/// whole reservation would refuse work far below real resident memory.
pub const STACK_COMMIT_ESTIMATE: usize = 64 * 1024;

/// Governor charge for a fiber stack, retained while the stack is pooled.
/// The charge covers the commit estimate plus guard pages and page rounding;
/// the full reservation is reported separately.
pub struct StackReservation {
    _charge: MemoryCharge,
    pooled: bool,
    overhead: Option<MemoryCharge>,
    reserved: usize,
}

impl StackReservation {
    pub fn new(bytes: usize) -> Result<Self, MemoryLimitExceeded> {
        let charge =
            governor().reserve_shared(MemoryClass::FiberStack, bytes.min(STACK_COMMIT_ESTIMATE))?;
        let live = LIVE.fetch_add(1, Ordering::Relaxed) + 1;
        PEAK.fetch_max(live, Ordering::Relaxed);
        RESERVED.fetch_add(bytes, Ordering::Relaxed);
        Ok(Self {
            _charge: charge,
            pooled: false,
            overhead: None,
            reserved: bytes,
        })
    }

    /// Charge guard pages and page rounding beyond the requested size, after
    /// the OS reports the bounds of the `requested`-byte stack.
    pub fn account_actual_bytes(
        &mut self,
        requested: usize,
        actual: usize,
    ) -> Result<(), MemoryLimitExceeded> {
        if actual > self.reserved {
            RESERVED.fetch_add(actual - self.reserved, Ordering::Relaxed);
            self.reserved = actual;
        }
        let extra = actual.saturating_sub(requested);
        if extra > self.overhead.as_ref().map_or(0, MemoryCharge::bytes) {
            self.overhead = Some(governor().reserve_shared(MemoryClass::FiberStack, extra)?);
        }
        Ok(())
    }

    pub fn set_pooled(&mut self, pooled: bool) {
        if self.pooled == pooled {
            return;
        }
        self.pooled = pooled;
        if pooled {
            LIVE.fetch_sub(1, Ordering::Relaxed);
            POOLED.fetch_add(1, Ordering::Relaxed);
        } else {
            POOLED.fetch_sub(1, Ordering::Relaxed);
            let live = LIVE.fetch_add(1, Ordering::Relaxed) + 1;
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
    }
}

impl Drop for StackReservation {
    fn drop(&mut self) {
        RESERVED.fetch_sub(self.reserved, Ordering::Relaxed);
        if self.pooled {
            POOLED.fetch_sub(1, Ordering::Relaxed);
        } else {
            LIVE.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

/// Reserved address space of active and pooled fiber stacks.
pub fn reserved_stack_bytes() -> usize {
    RESERVED.load(Ordering::Relaxed)
}

pub(crate) fn counts() -> (usize, usize, usize) {
    (
        LIVE.load(Ordering::Relaxed),
        PEAK.load(Ordering::Relaxed),
        POOLED.load(Ordering::Relaxed),
    )
}
