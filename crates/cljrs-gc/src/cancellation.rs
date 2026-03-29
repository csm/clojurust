//! GC cancellation and parking infrastructure.
//!
//! This module provides mechanisms for coordinating threads during garbage
//! collection, allowing long-running operations to yield control when a GC
//! is in progress.

use crate::config::{GC_CANCELLATION as CONFIG_CANCELLATION, GcParked};

/// A guard that marks the current thread as cancellable.
///
/// When dropped, the thread is marked as no longer cancellable.
/// This allows the GC to know which threads are safe to interrupt.
#[derive(Default)]
pub struct CancellableGuard {
    cancelled: bool,
}

impl CancellableGuard {
    /// Create a new cancellable guard.
    pub fn new() -> Self {
        Self { cancelled: false }
    }

    /// Check if this guard has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// Mark this guard as cancelled.
    pub fn set_cancelled(&mut self) {
        self.cancelled = true;
    }
}

impl Drop for CancellableGuard {
    fn drop(&mut self) {
        // Guard is automatically removed from the cancellable set
    }
}

/// Check if the current operation should be cancelled due to GC.
///
/// This function checks the global GC cancellation flag. If a GC is in
/// progress, it returns an error indicating the thread should park.
pub fn check_cancellation() -> Result<(), GcParked> {
    CONFIG_CANCELLATION.check()
}

/// GC safepoint: if a collection is in progress, park this thread until
/// it completes.  This is a blocking call suitable for use at allocation
/// sites, function entry, and loop heads.
///
/// The implementation spin-yields to avoid busy-waiting.
pub fn safepoint() {
    if !CONFIG_CANCELLATION.in_progress() {
        return;
    }
    // GC is in progress — park and wait.
    CONFIG_CANCELLATION.park();
    while CONFIG_CANCELLATION.in_progress() {
        std::thread::yield_now();
    }
    CONFIG_CANCELLATION.unpark();
}

/// Register the current thread as a GC mutator thread.
/// Must be called before the thread begins executing Clojure code.
/// Returns a [`MutatorGuard`] that unregisters on drop.
pub fn register_mutator() -> MutatorGuard {
    CONFIG_CANCELLATION.register_thread();
    MutatorGuard { _private: () }
}

/// RAII guard that unregisters a mutator thread on drop.
pub struct MutatorGuard {
    _private: (),
}

impl Drop for MutatorGuard {
    fn drop(&mut self) {
        CONFIG_CANCELLATION.unregister_thread();
    }
}

/// Request a GC collection at the next interpreter safepoint.
pub fn request_gc() {
    CONFIG_CANCELLATION.request_gc();
}

/// Check if a GC has been requested (memory pressure).
pub fn gc_requested() -> bool {
    CONFIG_CANCELLATION.gc_requested()
}

/// Atomically check and clear the GC request flag.
pub fn take_gc_request() -> bool {
    CONFIG_CANCELLATION.take_gc_request()
}

/// Wait for all registered mutator threads (except this one, the collector)
/// to park at safepoints.  The caller must have already set `in_progress`.
///
/// Returns the number of threads that parked (for diagnostics).
pub fn wait_for_threads_to_park() -> usize {
    let expected = CONFIG_CANCELLATION.registered_threads().saturating_sub(1);
    if expected == 0 {
        return 0;
    }
    // Spin-yield until all other threads have parked.
    loop {
        let parked = CONFIG_CANCELLATION.parked_threads();
        if parked >= expected {
            return parked;
        }
        std::thread::yield_now();
    }
}

/// Begin a stop-the-world collection phase.
///
/// Sets the `in_progress` flag and waits for all other mutator threads
/// to park.  Returns a [`StwGuard`] that clears the flag on drop
/// (allowing parked threads to resume).
///
/// Only one thread may call this at a time.  Uses compare-and-swap to
/// ensure mutual exclusion; returns `None` if another thread is already
/// collecting.
pub fn begin_stw() -> Option<StwGuard> {
    // Try to become the collector (atomic CAS: false → true).
    if !CONFIG_CANCELLATION.try_begin_collection() {
        // Another thread is already collecting.
        return None;
    }
    // Wait for all other mutator threads to park.
    wait_for_threads_to_park();
    Some(StwGuard { _private: () })
}

/// RAII guard that ends the STW phase on drop.
pub struct StwGuard {
    _private: (),
}

impl Drop for StwGuard {
    fn drop(&mut self) {
        CONFIG_CANCELLATION.set_in_progress(false);
    }
}

/// A helper that wraps a function with cancellation checking.
pub fn with_cancellation_check<T, E: std::fmt::Debug>(
    f: impl FnOnce() -> Result<T, E>,
) -> Result<T, GcParked> {
    check_cancellation()?;
    f().map_err(|_| GcParked)
}

/// Mark the current thread as parked during GC.
pub fn park_thread() {
    CONFIG_CANCELLATION.park();
}

/// Mark the current thread as unparked after GC completes.
pub fn unpark_thread() {
    CONFIG_CANCELLATION.unpark();
}

/// Get the number of threads currently parked waiting for GC.
pub fn parked_threads() -> usize {
    CONFIG_CANCELLATION.parked_threads()
}

/// Get the number of registered mutator threads.
pub fn registered_threads() -> usize {
    CONFIG_CANCELLATION.registered_threads()
}

// Extension trait for GcCancellation
pub trait GcCancellationExt {
    /// Check if GC is in progress and the current thread should park.
    fn check(&self) -> Result<(), GcParked>;
}

impl GcCancellationExt for crate::config::GcCancellation {
    fn check(&self) -> Result<(), GcParked> {
        if self.in_progress() {
            Err(GcParked)
        } else {
            Ok(())
        }
    }
}
