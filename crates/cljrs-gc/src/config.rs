//! GC configuration: memory limits and collection triggers.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// GC configuration with soft and hard memory limits.
///
/// The soft limit is a target that triggers collection when exceeded.
/// The hard limit is an absolute maximum - exceeding it forces immediate collection.
///
/// Default values:
/// - Hard limit: 1/4 of available RAM (or 256MB minimum)
/// - Soft limit: 75% of hard limit
#[derive(Debug, Clone)]
pub struct GcConfig {
    /// Hard memory limit in bytes. GC will be forced when exceeded.
    hard_limit: usize,
    /// Soft memory limit in bytes. GC is triggered when exceeded.
    soft_limit: usize,
}

impl GcConfig {
    /// Create a new GC config with default limits.
    pub fn new() -> Self {
        let hard_limit = default_hard_limit();
        let soft_limit = (hard_limit as f64 * 0.75) as usize;
        Self {
            hard_limit,
            soft_limit,
        }
    }

    /// Create a new GC config with a custom hard limit.
    pub fn with_hard_limit(hard_limit: usize) -> Self {
        let soft_limit = (hard_limit as f64 * 0.75) as usize;
        Self {
            hard_limit,
            soft_limit,
        }
    }

    /// Create a new GC config with custom limits.
    pub fn with_limits(soft_limit: usize, hard_limit: usize) -> Self {
        Self {
            soft_limit,
            hard_limit,
        }
    }

    /// Get the hard memory limit in bytes.
    pub fn hard_limit(&self) -> usize {
        self.hard_limit
    }

    /// Get the soft memory limit in bytes.
    pub fn soft_limit(&self) -> usize {
        self.soft_limit
    }

    /// Check if memory usage has exceeded the soft limit.
    pub fn soft_limit_exceeded(&self, used: usize) -> bool {
        used > self.soft_limit
    }

    /// Check if memory usage has exceeded the hard limit.
    pub fn hard_limit_exceeded(&self, used: usize) -> bool {
        used > self.hard_limit
    }
}

impl Default for GcConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Get default hard limit: 1/4 of available RAM or 256MB minimum.
fn default_hard_limit() -> usize {
    // Try to get total RAM from system info
    #[cfg(target_os = "linux")]
    fn get_total_ram() -> Option<usize> {
        // Read /proc/meminfo
        std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|content| {
                for line in content.lines() {
                    if line.starts_with("MemTotal:") {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 2 {
                            // Value is in kB
                            return parts[1].parse::<usize>().ok().map(|kb| kb * 1024);
                        }
                    }
                }
                None
            })
    }

    #[cfg(target_os = "macos")]
    fn get_total_ram() -> Option<usize> {
        // Use sysctl
        use std::ffi::CStr;

        let total: u64 = 0;
        let mut size = std::mem::size_of::<u64>();

        let name_cstr = {
            let bytes = b"hw.memsize\0";
            CStr::from_bytes_with_nul(bytes).ok()?
        };

        let ret = unsafe {
            let name = name_cstr.as_ptr();
            let addr = &total as *const u64 as *mut std::ffi::c_void;
            let oldlenp = &mut size as *mut usize;
            sysctlbyname(name, addr, oldlenp, std::ptr::null_mut(), 0)
        };

        if ret == 0 { Some(total as usize) } else { None }
    }

    #[cfg(target_os = "windows")]
    fn get_total_ram() -> Option<usize> {
        // Use GlobalMemoryStatusEx
        use std::mem::size_of;
        use windows::Win32::System::SystemInformation::GlobalMemoryStatusEx;
        use windows::Win32::System::SystemInformation::MEMORYSTATUSEX;

        let mut mem_status = MEMORYSTATUSEX::default();
        mem_status.dwLength = size_of::<MEMORYSTATUSEX>() as u32;

        if unsafe { GlobalMemoryStatusEx(&mut mem_status) }.is_ok() {
            Some(mem_status.ullTotalPhys as usize)
        } else {
            None
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    fn get_total_ram() -> Option<usize> {
        None
    }

    // Fallback to 256MB if we can't determine system RAM
    let total_ram = get_total_ram().unwrap_or(256 * 1024 * 1024);
    std::cmp::max(total_ram / 4, 256 * 1024 * 1024)
}

// sysctlbyname for macOS
#[cfg(target_os = "macos")]
#[link(name = "System")]
unsafe extern "C" {
    fn sysctlbyname(
        name: *const std::os::raw::c_char,
        oldp: *mut std::ffi::c_void,
        oldlenp: *mut usize,
        newp: *mut std::ffi::c_void,
        newlen: usize,
    ) -> std::os::raw::c_int;
}

/// Per-isolate coordination state for stop-the-world GC.
///
/// Each OS thread (isolate) has its own instance in a thread-local.
/// Tracks how many mutator threads are registered, how many have parked
/// at safepoints, and whether a GC has been requested or is in progress.
pub(crate) struct IsolateCancellation {
    /// Whether a GC collection is currently in progress (STW phase).
    in_progress: AtomicBool,
    /// Number of threads currently parked at a safepoint.
    parked_threads: AtomicUsize,
    /// Number of mutator threads registered with the GC.
    registered_threads: AtomicUsize,
    /// Flag set by the allocator when memory pressure is high.
    /// The next thread to hit an interpreter safepoint will initiate collection.
    gc_requested: AtomicBool,
}

impl IsolateCancellation {
    const fn new() -> Self {
        Self {
            in_progress: AtomicBool::new(false),
            parked_threads: AtomicUsize::new(0),
            registered_threads: AtomicUsize::new(0),
            gc_requested: AtomicBool::new(false),
        }
    }

    fn in_progress(&self) -> bool {
        self.in_progress.load(Ordering::SeqCst)
    }

    fn park(&self) {
        self.parked_threads.fetch_add(1, Ordering::SeqCst);
    }

    fn unpark(&self) {
        self.parked_threads.fetch_sub(1, Ordering::SeqCst);
    }

    fn parked_threads(&self) -> usize {
        self.parked_threads.load(Ordering::SeqCst)
    }

    fn set_in_progress(&self, value: bool) {
        self.in_progress.store(value, Ordering::SeqCst);
    }

    fn try_begin_collection(&self) -> bool {
        self.in_progress
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    fn register_thread(&self) {
        self.registered_threads.fetch_add(1, Ordering::SeqCst);
    }

    fn unregister_thread(&self) {
        self.registered_threads.fetch_sub(1, Ordering::SeqCst);
    }

    fn registered_threads(&self) -> usize {
        self.registered_threads.load(Ordering::SeqCst)
    }

    fn request_gc(&self) {
        self.gc_requested.store(true, Ordering::SeqCst);
    }

    fn take_gc_request(&self) -> bool {
        self.gc_requested.swap(false, Ordering::SeqCst)
    }

    fn gc_requested(&self) -> bool {
        self.gc_requested.load(Ordering::SeqCst)
    }
}

thread_local! {
    static ISOLATE_CANCELLATION: IsolateCancellation = IsolateCancellation::new();
}

/// Invoke `f` with a reference to this thread's `IsolateCancellation`.
pub(crate) fn with_cancellation<T>(f: impl FnOnce(&IsolateCancellation) -> T) -> T {
    ISOLATE_CANCELLATION.with(f)
}

/// Zero-sized proxy that dispatches every GC-coordination method to the
/// calling thread's [`IsolateCancellation`] via `with_cancellation`.
///
/// Keeping the public API as a `pub static GC_CANCELLATION: GcCancellation`
/// preserves all existing call sites while making the underlying state
/// per-isolate (thread-local).
pub struct GcCancellation;

impl GcCancellation {
    /// Create the zero-sized proxy.  The actual state lives in a thread-local.
    pub const fn new() -> Self {
        Self
    }

    /// Check if a GC is currently in progress on this thread's isolate.
    pub fn in_progress(&self) -> bool {
        with_cancellation(|c| c.in_progress())
    }

    /// Increment the parked thread count for this isolate.
    pub fn park(&self) {
        with_cancellation(|c| c.park());
    }

    /// Decrement the parked thread count for this isolate.
    pub fn unpark(&self) {
        with_cancellation(|c| c.unpark());
    }

    /// Get the number of parked threads for this isolate.
    pub fn parked_threads(&self) -> usize {
        with_cancellation(|c| c.parked_threads())
    }

    /// Set whether GC is in progress on this isolate.
    pub fn set_in_progress(&self, value: bool) {
        with_cancellation(|c| c.set_in_progress(value));
    }

    /// Atomically try to set `in_progress` from `false` to `true` on this isolate.
    /// Returns `true` if this thread won the race, `false` if another
    /// thread is already collecting.
    pub fn try_begin_collection(&self) -> bool {
        with_cancellation(|c| c.try_begin_collection())
    }

    /// Register a mutator thread on this isolate. Must be called before the
    /// thread begins executing Clojure code (interpreter or AOT).
    pub fn register_thread(&self) {
        with_cancellation(|c| c.register_thread());
    }

    /// Unregister a mutator thread on this isolate. Must be called when the
    /// thread is done executing Clojure code.
    pub fn unregister_thread(&self) {
        with_cancellation(|c| c.unregister_thread());
    }

    /// Get the number of registered mutator threads on this isolate.
    pub fn registered_threads(&self) -> usize {
        with_cancellation(|c| c.registered_threads())
    }

    /// Request a GC collection on this isolate. The next interpreter safepoint
    /// will initiate it.
    pub fn request_gc(&self) {
        with_cancellation(|c| c.request_gc());
    }

    /// Check and clear the GC request flag for this isolate. Returns true if a
    /// GC was requested.
    pub fn take_gc_request(&self) -> bool {
        with_cancellation(|c| c.take_gc_request())
    }

    /// Check if a GC has been requested but not yet started on this isolate.
    pub fn gc_requested(&self) -> bool {
        with_cancellation(|c| c.gc_requested())
    }
}

// SAFETY: GcCancellation is zero-sized and carries no data; all state is in
// a thread-local. The Send + Sync impls are needed so the static can be
// referenced from any thread.
unsafe impl Sync for GcCancellation {}
unsafe impl Send for GcCancellation {}

impl Default for GcCancellation {
    fn default() -> Self {
        Self::new()
    }
}

/// Global GC cancellation coordinator (dispatches to per-isolate thread-local).
pub static GC_CANCELLATION: GcCancellation = GcCancellation::new();

/// Check if a GC is in progress on this thread's isolate and return an error
/// if so.
///
/// Returns `Ok(())` if execution can continue, `Err(GcParked)` if the
/// thread should park until GC completes.
pub fn check_cancellation() -> Result<(), GcParked> {
    if GC_CANCELLATION.in_progress() {
        Err(GcParked)
    } else {
        Ok(())
    }
}

/// Error type returned when a thread should park during GC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcParked;

impl std::fmt::Display for GcParked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GC in progress, thread should park")
    }
}

impl std::error::Error for GcParked {}
