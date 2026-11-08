//! Executor-independent fiber safepoints, diagnostics, and stack accounting.

use std::cell::Cell;
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
    pub enter_native: fn(&Arc<str>, &NativeDiagnosticId) -> usize,
    /// Like `enter_native`, but marks a garbage-collection pause rather than
    /// a named native call. Undone by `exit_native`.
    pub enter_gc_pause: fn() -> usize,
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

/// Read the machine stack, rather than an ASan fake-stack local's address.
#[inline(always)]
pub fn stack_pointer() -> usize {
    #[cfg(any(
        target_arch = "x86_64",
        target_arch = "x86",
        target_arch = "aarch64",
        target_arch = "arm",
        target_arch = "riscv32",
        target_arch = "riscv64",
        target_arch = "loongarch64",
        target_arch = "powerpc64",
    ))]
    {
        let pointer: usize;
        // SAFETY: each instruction only copies the current stack register. It
        // neither accesses memory nor changes the stack or condition flags.
        unsafe {
            #[cfg(target_arch = "x86_64")]
            std::arch::asm!("mov {}, rsp", out(reg) pointer, options(nomem, nostack, preserves_flags));
            #[cfg(target_arch = "x86")]
            std::arch::asm!("mov {}, esp", out(reg) pointer, options(nomem, nostack, preserves_flags));
            #[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
            std::arch::asm!("mov {}, sp", out(reg) pointer, options(nomem, nostack, preserves_flags));
            #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
            std::arch::asm!("mv {}, sp", out(reg) pointer, options(nomem, nostack, preserves_flags));
            #[cfg(target_arch = "loongarch64")]
            std::arch::asm!("move {}, $sp", out(reg) pointer, options(nomem, nostack, preserves_flags));
            #[cfg(target_arch = "powerpc64")]
            std::arch::asm!("mr {}, 1", out(reg) pointer, options(nomem, nostack, preserves_flags));
        }
        pointer
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "x86",
        target_arch = "aarch64",
        target_arch = "arm",
        target_arch = "riscv32",
        target_arch = "riscv64",
        target_arch = "loongarch64",
        target_arch = "powerpc64",
    )))]
    {
        // No fiber stacks here (e.g. wasm); approximate with a local.
        let marker = 0u8;
        std::hint::black_box(&marker) as *const u8 as usize
    }
}

/// Stack kept free below a native recursion probe: room to unwind to the
/// nearest evaluation boundary. Smaller than the interpreter safepoint's
/// margin, so Clojure code called back from native code still fails first.
pub const NATIVE_STACK_MARGIN: usize = 128 * 1024;

thread_local! {
    /// Lowest usable address of the running fiber's stack (guard included);
    /// zero when no fiber runs on this thread.
    static NATIVE_STACK_LIMIT: Cell<usize> = const { Cell::new(0) };
}

/// Publish the running fiber's stack limit for [`native_stack_low`]. Zero
/// clears it. Returns the previous limit, to restore when the resume ends.
pub fn set_native_stack_limit(limit: usize) -> usize {
    NATIVE_STACK_LIMIT.with(|cell| cell.replace(limit))
}

/// Probe for recursive native code (printing, equality, hashing, comparison)
/// that never reaches an interpreter safepoint. Returns the bytes left when
/// the running fiber is within [`NATIVE_STACK_MARGIN`] of its guard page.
/// Always `None` off a fiber.
#[inline]
pub fn native_stack_low() -> Option<usize> {
    let limit = NATIVE_STACK_LIMIT.with(Cell::get);
    if limit == 0 {
        return None;
    }
    let remaining = stack_pointer().saturating_sub(limit);
    (remaining < NATIVE_STACK_MARGIN).then_some(remaining)
}

/// A native function's lazily assigned diagnostic name id, stored with the
/// function so warm calls need no name lookup. Zero means unassigned.
#[derive(Debug, Default)]
pub struct NativeDiagnosticId(AtomicUsize);

impl NativeDiagnosticId {
    pub fn get(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
    pub fn set(&self, id: usize) {
        self.0.store(id, Ordering::Relaxed);
    }
}

impl Clone for NativeDiagnosticId {
    fn clone(&self) -> Self {
        Self(AtomicUsize::new(self.get()))
    }
}

/// Name the innermost native call for stall diagnostics. May span parks.
/// `id` caches the name's diagnostic id across calls.
pub fn native_frame(name: &Arc<str>, id: &NativeDiagnosticId) -> NativeFrameGuard {
    let previous = HOOKS
        .get()
        .map_or(0, |hooks| (hooks.enter_native)(name, id));
    NativeFrameGuard {
        previous,
        _thread: std::marker::PhantomData,
    }
}

/// Mark the running fiber as paused for garbage collection (collecting, or
/// waiting for a stop-the-world phase), so a stall is not blamed on native
/// code. Without a running fiber this does nothing.
pub fn gc_pause() -> NativeFrameGuard {
    let previous = HOOKS.get().map_or(0, |hooks| (hooks.enter_gc_pause)());
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
