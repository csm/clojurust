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
    pub safepoint: fn() -> Result<(), SafepointError>,
    pub enter_native: fn(&Arc<str>),
    pub exit_native: fn(),
}

static HOOKS: OnceLock<Hooks> = OnceLock::new();
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static POOLED: AtomicUsize = AtomicUsize::new(0);

/// Install the native driver once. Without a driver safepoints are no-ops.
pub fn install(hooks: Hooks) {
    let _ = HOOKS.set(hooks);
}

/// Cooperate with the scheduler and check the running fiber's stack margin.
pub fn safepoint() -> Result<(), SafepointError> {
    HOOKS.get().map_or(Ok(()), |h| (h.safepoint)())
}

/// Name the innermost native call for stall diagnostics. May span parks.
pub fn native_frame(name: &Arc<str>) -> NativeFrameGuard {
    if let Some(hooks) = HOOKS.get() {
        (hooks.enter_native)(name);
    }
    NativeFrameGuard(std::marker::PhantomData)
}

pub struct NativeFrameGuard(std::marker::PhantomData<std::rc::Rc<()>>);
impl Drop for NativeFrameGuard {
    fn drop(&mut self) {
        if let Some(hooks) = HOOKS.get() {
            (hooks.exit_native)();
        }
    }
}

/// Governor charge for reserved stack bytes, retained while the stack is pooled.
pub struct StackReservation {
    _charge: MemoryCharge,
    pooled: bool,
    overhead: Option<MemoryCharge>,
}

impl StackReservation {
    pub fn new(bytes: usize) -> Result<Self, MemoryLimitExceeded> {
        let charge = governor().reserve_shared(MemoryClass::FiberStack, bytes)?;
        let live = LIVE.fetch_add(1, Ordering::Relaxed) + 1;
        PEAK.fetch_max(live, Ordering::Relaxed);
        Ok(Self {
            _charge: charge,
            pooled: false,
            overhead: None,
        })
    }

    /// Charge page rounding and guard pages after the OS reports the bounds.
    pub fn account_actual_bytes(&mut self, bytes: usize) -> Result<(), MemoryLimitExceeded> {
        let charged = self._charge.bytes() + self.overhead.as_ref().map_or(0, MemoryCharge::bytes);
        if bytes <= charged {
            return Ok(());
        }
        let extra = bytes - self._charge.bytes();
        if extra > 0 {
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
        if self.pooled {
            POOLED.fetch_sub(1, Ordering::Relaxed);
        } else {
            LIVE.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

pub(crate) fn counts() -> (usize, usize, usize) {
    (
        LIVE.load(Ordering::Relaxed),
        PEAK.load(Ordering::Relaxed),
        POOLED.load(Ordering::Relaxed),
    )
}
