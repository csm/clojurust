//! Process-global GC statistics counters.
//!
//! Tracks three classes of events:
//!
//! 1. **GC heap allocations** — every object allocated through `GcHeap::alloc`
//!    (default build) bumps `gc_allocations` and `gc_alloc_bytes`.
//! 2. **Region (bump) allocations** — every object allocated through
//!    [`crate::region::Region::alloc`] bumps `region_allocations` and
//!    `region_alloc_bytes`.  These represent cases where the bump allocator
//!    was used instead of the GC heap.
//! 3. **Stop-the-world collections** — every completed `GcHeap::collect`
//!    bumps `gc_collections` and accumulates pause time, freed object count,
//!    and freed bytes.
//!
//! Counters are process-global ([`GC_STATS`]) and thread-safe via atomics.
//! Reset is not supported — the counters live for the lifetime of the process.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Process-global GC statistics counters.
pub struct GcStats {
    gc_allocations: AtomicU64,
    gc_alloc_bytes: AtomicU64,
    region_allocations: AtomicU64,
    region_alloc_bytes: AtomicU64,
    gc_collections: AtomicU64,
    gc_pause_total_nanos: AtomicU64,
    gc_objects_freed: AtomicU64,
    gc_bytes_freed: AtomicU64,
}

impl GcStats {
    pub const fn new() -> Self {
        Self {
            gc_allocations: AtomicU64::new(0),
            gc_alloc_bytes: AtomicU64::new(0),
            region_allocations: AtomicU64::new(0),
            region_alloc_bytes: AtomicU64::new(0),
            gc_collections: AtomicU64::new(0),
            gc_pause_total_nanos: AtomicU64::new(0),
            gc_objects_freed: AtomicU64::new(0),
            gc_bytes_freed: AtomicU64::new(0),
        }
    }

    /// Record one allocation through the GC heap (`bytes` is the estimated
    /// or actual allocated size).
    #[inline]
    pub fn record_gc_alloc(&self, bytes: usize) {
        self.gc_allocations.fetch_add(1, Ordering::Relaxed);
        self.gc_alloc_bytes
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// Record one allocation through a bump region (instead of the GC heap).
    #[inline]
    pub fn record_region_alloc(&self, bytes: usize) {
        self.region_allocations.fetch_add(1, Ordering::Relaxed);
        self.region_alloc_bytes
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// Record a completed stop-the-world collection.
    #[inline]
    pub fn record_gc_pause(&self, pause: Duration, freed_objects: u64, freed_bytes: u64) {
        self.gc_collections.fetch_add(1, Ordering::Relaxed);
        self.gc_pause_total_nanos
            .fetch_add(pause.as_nanos() as u64, Ordering::Relaxed);
        self.gc_objects_freed
            .fetch_add(freed_objects, Ordering::Relaxed);
        self.gc_bytes_freed
            .fetch_add(freed_bytes, Ordering::Relaxed);
    }

    /// Take a point-in-time snapshot of all counters.
    pub fn snapshot(&self) -> GcStatsSnapshot {
        GcStatsSnapshot {
            gc_allocations: self.gc_allocations.load(Ordering::Relaxed),
            gc_alloc_bytes: self.gc_alloc_bytes.load(Ordering::Relaxed),
            region_allocations: self.region_allocations.load(Ordering::Relaxed),
            region_alloc_bytes: self.region_alloc_bytes.load(Ordering::Relaxed),
            gc_collections: self.gc_collections.load(Ordering::Relaxed),
            gc_pause_total_nanos: self.gc_pause_total_nanos.load(Ordering::Relaxed),
            gc_objects_freed: self.gc_objects_freed.load(Ordering::Relaxed),
            gc_bytes_freed: self.gc_bytes_freed.load(Ordering::Relaxed),
        }
    }
}

impl Default for GcStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Immutable point-in-time view of [`GcStats`].
#[derive(Debug, Clone, Copy, Default)]
pub struct GcStatsSnapshot {
    pub gc_allocations: u64,
    pub gc_alloc_bytes: u64,
    pub region_allocations: u64,
    pub region_alloc_bytes: u64,
    pub gc_collections: u64,
    pub gc_pause_total_nanos: u64,
    pub gc_objects_freed: u64,
    pub gc_bytes_freed: u64,
}

impl GcStatsSnapshot {
    /// Total cumulative GC pause time.
    pub fn total_pause(&self) -> Duration {
        Duration::from_nanos(self.gc_pause_total_nanos)
    }
}

impl fmt::Display for GcStatsSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "GC stats:")?;
        writeln!(
            f,
            "  GC allocations:        {} ({} bytes)",
            self.gc_allocations, self.gc_alloc_bytes
        )?;
        writeln!(
            f,
            "  Region (bump) allocs:  {} ({} bytes)",
            self.region_allocations, self.region_alloc_bytes
        )?;
        writeln!(f, "  GC collections:        {}", self.gc_collections)?;
        writeln!(f, "  Total GC pause time:   {:.3?}", self.total_pause())?;
        writeln!(f, "  Objects freed by GC:   {}", self.gc_objects_freed)?;
        write!(f, "  Bytes freed by GC:     {}", self.gc_bytes_freed)
    }
}

/// Process-global GC statistics counters.
pub static GC_STATS: GcStats = GcStats::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_zero_by_default() {
        let stats = GcStats::new();
        let snap = stats.snapshot();
        assert_eq!(snap.gc_allocations, 0);
        assert_eq!(snap.region_allocations, 0);
        assert_eq!(snap.gc_collections, 0);
        assert_eq!(snap.total_pause(), Duration::ZERO);
    }

    #[test]
    fn record_gc_alloc_updates_counters() {
        let stats = GcStats::new();
        stats.record_gc_alloc(48);
        stats.record_gc_alloc(48);
        let snap = stats.snapshot();
        assert_eq!(snap.gc_allocations, 2);
        assert_eq!(snap.gc_alloc_bytes, 96);
    }

    #[test]
    fn record_region_alloc_updates_counters() {
        let stats = GcStats::new();
        stats.record_region_alloc(16);
        stats.record_region_alloc(32);
        let snap = stats.snapshot();
        assert_eq!(snap.region_allocations, 2);
        assert_eq!(snap.region_alloc_bytes, 48);
    }

    #[test]
    fn record_gc_pause_updates_counters() {
        let stats = GcStats::new();
        stats.record_gc_pause(Duration::from_micros(500), 7, 336);
        stats.record_gc_pause(Duration::from_micros(250), 3, 144);
        let snap = stats.snapshot();
        assert_eq!(snap.gc_collections, 2);
        assert_eq!(snap.gc_objects_freed, 10);
        assert_eq!(snap.gc_bytes_freed, 480);
        assert_eq!(snap.total_pause(), Duration::from_micros(750));
    }

    #[test]
    fn display_renders_all_fields() {
        let snap = GcStatsSnapshot {
            gc_allocations: 5,
            gc_alloc_bytes: 240,
            region_allocations: 3,
            region_alloc_bytes: 96,
            gc_collections: 1,
            gc_pause_total_nanos: 1_000_000,
            gc_objects_freed: 2,
            gc_bytes_freed: 96,
        };
        let s = format!("{snap}");
        assert!(s.contains("GC allocations:"));
        assert!(s.contains("Region (bump) allocs:"));
        assert!(s.contains("GC collections:"));
        assert!(s.contains("Total GC pause time:"));
        assert!(s.contains("Objects freed by GC:"));
        assert!(s.contains("Bytes freed by GC:"));
    }
}
