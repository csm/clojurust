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
///
/// This is a "soft" cancellation check - operations should periodically
/// call this to yield control if needed.
pub fn check_cancellation() -> Result<(), GcParked> {
    CONFIG_CANCELLATION.check()
}

/// A helper that wraps a function with cancellation checking.
///
/// The wrapped function is called periodically, and if a GC is in progress,
/// it returns early with an error.
pub fn with_cancellation_check<T, E: std::fmt::Debug>(
    f: impl FnOnce() -> Result<T, E>,
) -> Result<T, GcParked> {
    check_cancellation()?;
    f().map_err(|_| GcParked)
}

/// Mark the current thread as parked during GC.
///
/// This should be called when a thread needs to wait for GC to complete.
/// Threads should park at safe points when they detect a GC is in progress.
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
