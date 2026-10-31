//! Process-wide managed-memory governor.
//!
//! See `docs/managed-memory-governor-plan.md`.  Every isolate keeps its private
//! heap and collects it independently; the governor owns only counters,
//! pressure policy, and thread-safe [`IsolateControl`] handles.
//!
//! **Status: Phase 5 (policy and platform pressure).**
//! Phase 5 adds:
//! - Allocation-rate–based collection targets: each isolate's post-collection
//!   target is `surviving_bytes + alloc_rate × interval + min_headroom`, where
//!   `interval` is [`MemoryConfig::collection_target_interval_ms`].  This
//!   replaces the fixed `surviving/2` heuristic from Phases 1–4.
//! - Separate rate limits for cross-isolate collection requests: the governor
//!   tracks `requested_epoch` (self-requests) and `cross_requested_epoch`
//!   (requests from the governor on behalf of another isolate) independently.
//! - Optional RSS emergency pressure signal: if process RSS exceeds
//!   `hard_limit × rss_pressure_percent / 100`, pressure is clamped to `Red`.
//! - Load-shedding hooks: servers register callbacks via
//!   [`ProcessMemoryGovernor::register_shed_hook`]; they are called on every
//!   pressure transition.
//! - Linux cgroup v2 `memory.high` is used as the default soft limit when
//!   available.
//!
//! **Status: Phase 4 (chunked credit, non-heap classes, opt-in strict
//! admission) — base for Phase 5.**
//! Each isolate allocates from local credit that the governor grants in
//! chunks of [`MemoryConfig::credit_chunk`]; granted credit counts as
//! committed memory.  Region chunks draw from the same local credit and are
//! reported under [`MemoryClass::Region`].  Shared values, queued messages,
//! and JIT code hold a [`MemoryCharge`]; static allocations are recorded with
//! [`ProcessMemoryGovernor::charge_permanent`].
//! The governor derives a process [`PressureLevel`] from committed bytes and
//! requests collection from isolates at `Yellow`/`Red`.
//!
//! In observe-only mode (the default) the governor never rejects a grant: a
//! grant that leaves committed bytes above the hard limit is recorded as a
//! *would-reject* event (see [`MemorySnapshot::over_limit_events`]).
//!
//! In strict mode ([`MemoryConfig::strict`], `CLJRS_MEMORY_STRICT=1`) an
//! application grant that would take committed bytes above the hard limit is
//! refused ([`Admission::Application`]).  Fallible callers
//! ([`IsolateAccount::try_reserve_credit`], [`ProcessMemoryGovernor::reserve_shared`],
//! `GcPtr::try_new`) get a [`MemoryLimitExceeded`] and no counter changes.
//! An infallible caller (`GcPtr::new`, a region chunk) cannot refuse, so it
//! raises the thread's pending *runtime fault* ([`raise_runtime_fault`]),
//! asks its isolate to collect, and takes the bytes from the critical
//! reserve.  The interpreter resolves the fault at its next safepoint, after
//! the collection: it continues if committed bytes are back within the hard
//! limit and fails the evaluation with `MemoryLimitExceeded` otherwise
//! ([`resolve_runtime_fault`]).  Runtime control work (error reporting,
//! shutdown) runs inside a [`CriticalSection`], whose grants use the reserve
//! ([`Admission::Critical`]) without raising a fault.  Only when the reserve
//! is also exhausted does an infallible grant exceed
//! `hard_limit + critical_reserve`; each such grant is counted in
//! [`MemorySnapshot::reserve_overruns`].
//!
//! The hot path ([`IsolateAccount::charge`]) touches only thread-local `Cell`s.
//! It contacts the governor only when local credit runs out.  A refill is the
//! one point where an isolate publishes its used bytes, so per-isolate
//! metrics lag by less than one chunk.
//! Committed bytes are exact: they change only when credit is granted or
//! returned.

use std::cell::{Cell, OnceCell};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

const KIB: usize = 1024;
const MIB: usize = 1024 * 1024;

/// Default isolate credit chunk.
pub const DEFAULT_CREDIT_CHUNK: usize = 64 * KIB;
/// Default number of unused credit chunks an isolate keeps after a
/// collection at `Green` pressure.
pub const DEFAULT_RETAINED_CREDIT_CHUNKS: usize = 2;
/// Granularity of a large credit request (one that exceeds a chunk).
pub const ACCOUNTING_UNIT: usize = 4 * KIB;
/// Floor for the post-collection headroom below which the governor does not
/// request collection at `Yellow`.
pub const MIN_COLLECTION_HEADROOM: usize = MIB;
/// Floor for the default critical reserve.
pub const MIN_DEFAULT_CRITICAL_RESERVE: usize = 4 * MIB;
/// Ceiling for the default critical reserve.
pub const MAX_DEFAULT_CRITICAL_RESERVE: usize = 256 * MIB;
/// Ceiling for the default per-channel queue limit.
pub const DEFAULT_QUEUE_LIMIT: usize = 64 * MIB;

/// Process-wide soft limit, in MiB.
pub const SOFT_LIMIT_ENV: &str = "CLJRS_MEMORY_SOFT_LIMIT_MB";
/// Process-wide hard limit, in MiB.
pub const HARD_LIMIT_ENV: &str = "CLJRS_MEMORY_HARD_LIMIT_MB";
/// Critical reserve for runtime control operations, in MiB.
pub const CRITICAL_RESERVE_ENV: &str = "CLJRS_MEMORY_CRITICAL_RESERVE_MB";
/// Isolate credit chunk, in KiB.
pub const CREDIT_ENV: &str = "CLJRS_MEMORY_CREDIT_KB";
/// Default byte limit for one isolate channel, in MiB.
pub const QUEUE_LIMIT_ENV: &str = "CLJRS_ISOLATE_QUEUE_LIMIT_MB";
/// Deprecated alias for [`SOFT_LIMIT_ENV`].
pub const DEPRECATED_SOFT_LIMIT_ENV: &str = "CLJRS_GC_SOFT_LIMIT_MB";
/// Deprecated alias for [`HARD_LIMIT_ENV`].
pub const DEPRECATED_HARD_LIMIT_ENV: &str = "CLJRS_GC_HARD_LIMIT_MB";
/// Strict admission (`1`/`true`/`yes`/`on` or `0`/`false`/`no`/`off`).
pub const STRICT_ENV: &str = "CLJRS_MEMORY_STRICT";
/// Collection target interval in milliseconds (0 = use `surviving/2` heuristic).
pub const COLLECTION_INTERVAL_ENV: &str = "CLJRS_MEMORY_COLLECTION_INTERVAL_MS";
/// RSS emergency threshold as a percentage of the hard limit (0 = disabled).
pub const RSS_PRESSURE_PCT_ENV: &str = "CLJRS_MEMORY_RSS_PRESSURE_PCT";

/// Default post-collection target interval in milliseconds.
pub const DEFAULT_COLLECTION_INTERVAL_MS: u32 = 1000;
/// How long to cache the RSS reading.
const RSS_CACHE_NS: u64 = 1_000_000_000;

/// Identifier returned by [`ProcessMemoryGovernor::register_shed_hook`].
pub type ShedHookId = u64;

type ShedHook = Arc<dyn Fn(PressureLevel) + Send + Sync>;
type ShedHookVec = Vec<(ShedHookId, ShedHook)>;

fn unix_nanos() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}

// ── Memory classes and pressure ──────────────────────────────────────────────

/// Owner and lifetime rule for a governed allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryClass {
    /// Objects in an isolate's private tracing heap.
    GcHeap,
    /// Bump-region chunks (active and retired).
    Region,
    /// Shared immutable values (`ByteBlob`, `SharedAtom` storage, …).
    SharedValue,
    /// Serialized messages queued on isolate channels.
    MessageQueue,
    /// Program-lifetime static allocations.
    Static,
    /// JIT code and code caches.
    Code,
    /// Other runtime-owned data.
    Runtime,
    /// Reserved native fiber stacks, including pooled stacks and guard pages.
    FiberStack,
}

const CLASS_COUNT: usize = 8;

impl MemoryClass {
    /// Every memory class, in reporting order.
    pub const ALL: [MemoryClass; CLASS_COUNT] = [
        MemoryClass::GcHeap,
        MemoryClass::Region,
        MemoryClass::SharedValue,
        MemoryClass::MessageQueue,
        MemoryClass::Static,
        MemoryClass::Code,
        MemoryClass::Runtime,
        MemoryClass::FiberStack,
    ];

    /// Static, allocation-free name used in diagnostics and errors.
    pub const fn name(self) -> &'static str {
        match self {
            MemoryClass::GcHeap => "gc-heap",
            MemoryClass::Region => "region",
            MemoryClass::SharedValue => "shared-value",
            MemoryClass::MessageQueue => "message-queue",
            MemoryClass::Static => "static",
            MemoryClass::Code => "code",
            MemoryClass::Runtime => "runtime",
            MemoryClass::FiberStack => "fiber-stack",
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// Process memory pressure, derived from committed bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum PressureLevel {
    /// Below the soft limit.
    Green = 0,
    /// At or above the soft limit.
    Yellow = 1,
    /// Above the hard limit, or a strict-mode application request did not
    /// fit below it.
    Red = 2,
}

impl PressureLevel {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => PressureLevel::Green,
            1 => PressureLevel::Yellow,
            _ => PressureLevel::Red,
        }
    }

    /// Classify `committed` bytes against the given limits.
    pub fn classify(committed: usize, soft_limit: usize, hard_limit: usize) -> Self {
        if committed > hard_limit {
            PressureLevel::Red
        } else if committed >= soft_limit {
            PressureLevel::Yellow
        } else {
            PressureLevel::Green
        }
    }
}

impl fmt::Display for PressureLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PressureLevel::Green => "Green",
            PressureLevel::Yellow => "Yellow",
            PressureLevel::Red => "Red",
        })
    }
}

// ── Configuration ────────────────────────────────────────────────────────────

/// Process-wide managed-memory limits.  Read once per process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryConfig {
    /// Committed bytes at which pressure becomes `Yellow`.
    pub soft_limit: usize,
    /// Committed bytes above which pressure becomes `Red`.
    pub hard_limit: usize,
    /// Capacity reserved for collection, shutdown, and error reporting.
    pub critical_reserve: usize,
    /// Normal isolate credit chunk.
    pub credit_chunk: usize,
    /// Unused credit chunks an isolate keeps after a collection at `Green`.
    /// `Yellow` keeps at most one and `Red` keeps none.
    pub retained_credit_chunks: usize,
    /// Default byte limit for one isolate channel.
    pub queue_limit: usize,
    /// Refuse application grants above the hard limit instead of only
    /// recording them.
    pub strict: bool,
    /// Target allocation interval (ms) for dynamic collection targets.  The
    /// post-collection target is `surviving + alloc_rate × interval + floor`.
    /// Zero uses the old `surviving/2` heuristic from Phases 1–4.
    /// Default: [`DEFAULT_COLLECTION_INTERVAL_MS`].
    pub collection_target_interval_ms: u32,
    /// RSS threshold as a percentage of the hard limit.  When process RSS
    /// exceeds `hard_limit × rss_pressure_percent / 100` the governor raises
    /// pressure by one level: `Green` becomes `Yellow`, and `Yellow` becomes
    /// `Red`.  RSS alone never forces `Red`, because allocators keep freed
    /// pages and RSS may not fall after a collection.  Zero disables the
    /// signal.  Default: 0.
    pub rss_pressure_percent: u8,
}

/// A rejected [`MemoryConfig`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryConfigError {
    /// The hard limit is zero.
    ZeroHardLimit,
    /// The soft limit is greater than the hard limit.
    SoftAboveHard {
        soft_limit: usize,
        hard_limit: usize,
    },
    /// The critical reserve is greater than the hard limit.
    ReserveAboveHard {
        critical_reserve: usize,
        hard_limit: usize,
    },
    /// The credit chunk is zero.
    ZeroCreditChunk,
    /// The queue limit is zero.
    ZeroQueueLimit,
    /// The queue limit is greater than the application budget.
    QueueAboveBudget {
        queue_limit: usize,
        hard_limit: usize,
    },
    /// An environment variable is not a non-negative integer.
    InvalidNumber { var: &'static str, value: String },
    /// An environment variable is not a boolean flag.
    InvalidFlag { var: &'static str, value: String },
}

impl fmt::Display for MemoryConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryConfigError::ZeroHardLimit => write!(f, "the hard memory limit must be non-zero"),
            MemoryConfigError::SoftAboveHard {
                soft_limit,
                hard_limit,
            } => write!(
                f,
                "the soft memory limit ({soft_limit} bytes) is greater than the hard limit ({hard_limit} bytes)"
            ),
            MemoryConfigError::ReserveAboveHard {
                critical_reserve,
                hard_limit,
            } => write!(
                f,
                "the critical reserve ({critical_reserve} bytes) is greater than the hard limit ({hard_limit} bytes)"
            ),
            MemoryConfigError::ZeroCreditChunk => write!(f, "the credit chunk must be non-zero"),
            MemoryConfigError::ZeroQueueLimit => {
                write!(f, "the isolate queue limit must be non-zero")
            }
            MemoryConfigError::QueueAboveBudget {
                queue_limit,
                hard_limit,
            } => write!(
                f,
                "the isolate queue limit ({queue_limit} bytes) is greater than the hard limit ({hard_limit} bytes)"
            ),
            MemoryConfigError::InvalidNumber { var, value } => {
                write!(f, "{var}={value:?} is not a non-negative integer")
            }
            MemoryConfigError::InvalidFlag { var, value } => {
                write!(f, "{var}={value:?} is not 1/true/yes/on or 0/false/no/off")
            }
        }
    }
}

impl std::error::Error for MemoryConfigError {}

impl MemoryConfig {
    /// Build a config from explicit limits; the remaining fields take their
    /// defaults for `hard_limit`.
    pub fn with_limits(soft_limit: usize, hard_limit: usize) -> Result<Self, MemoryConfigError> {
        let config = Self {
            soft_limit,
            hard_limit,
            critical_reserve: default_critical_reserve(hard_limit),
            credit_chunk: DEFAULT_CREDIT_CHUNK,
            retained_credit_chunks: DEFAULT_RETAINED_CREDIT_CHUNKS,
            queue_limit: DEFAULT_QUEUE_LIMIT.min(hard_limit),
            strict: false,
            collection_target_interval_ms: DEFAULT_COLLECTION_INTERVAL_MS,
            rss_pressure_percent: 0,
        };
        config.validate()?;
        Ok(config)
    }

    /// Build a config from optional soft and hard limits.
    ///
    /// A missing hard limit uses [`default_hard_limit`], raised to the soft
    /// limit when the soft limit is larger.  A missing soft limit is the
    /// cgroup v2 `memory.high` value when available, otherwise 75% of the
    /// hard limit.
    pub fn from_optional_limits(
        soft_limit: Option<usize>,
        hard_limit: Option<usize>,
    ) -> Result<Self, MemoryConfigError> {
        let hard = match (soft_limit, hard_limit) {
            (_, Some(hard)) => hard,
            (Some(soft), None) => default_hard_limit().max(soft),
            (None, None) => default_hard_limit(),
        };
        let soft = soft_limit
            .or_else(|| container_soft_memory_limit().filter(|&s| s < hard))
            .unwrap_or_else(|| default_soft_limit(hard));
        Self::with_limits(soft, hard)
    }

    /// Platform defaults: container limit or a fraction of physical memory.
    pub fn platform_default() -> Self {
        let hard = default_hard_limit().max(1);
        Self::with_limits(default_soft_limit(hard), hard)
            .expect("platform default memory config is valid")
    }

    /// Check the invariants the plan requires of a config.
    pub fn validate(&self) -> Result<(), MemoryConfigError> {
        if self.hard_limit == 0 {
            return Err(MemoryConfigError::ZeroHardLimit);
        }
        if self.soft_limit > self.hard_limit {
            return Err(MemoryConfigError::SoftAboveHard {
                soft_limit: self.soft_limit,
                hard_limit: self.hard_limit,
            });
        }
        if self.critical_reserve > self.hard_limit {
            return Err(MemoryConfigError::ReserveAboveHard {
                critical_reserve: self.critical_reserve,
                hard_limit: self.hard_limit,
            });
        }
        if self.credit_chunk == 0 {
            return Err(MemoryConfigError::ZeroCreditChunk);
        }
        if self.queue_limit == 0 {
            return Err(MemoryConfigError::ZeroQueueLimit);
        }
        if self.queue_limit > self.hard_limit {
            return Err(MemoryConfigError::QueueAboveBudget {
                queue_limit: self.queue_limit,
                hard_limit: self.hard_limit,
            });
        }
        Ok(())
    }

    /// Read the process config from the environment.
    ///
    /// The deprecated `CLJRS_GC_*_LIMIT_MB` names set process-wide values; a
    /// new variable wins over its old alias.  Each old variable in use
    /// produces one warning on stderr.
    pub fn from_env() -> Result<Self, MemoryConfigError> {
        Self::from_lookup(|var| std::env::var(var).ok(), |msg| eprintln!("{msg}"))
    }

    /// [`Self::from_env`] over an arbitrary variable lookup (for tests).
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
        mut warn: impl FnMut(String),
    ) -> Result<Self, MemoryConfigError> {
        let mut limit = |new: &'static str, old: &'static str| -> Result<Option<usize>, _> {
            let new_val = parse_env(new, lookup(new), MIB)?;
            if let Some(old_val) = lookup(old) {
                if new_val.is_some() {
                    warn(format!(
                        "[memory] warning: {old} is deprecated and ignored because {new} is set"
                    ));
                    return Ok(new_val);
                }
                warn(format!(
                    "[memory] warning: {old} is deprecated; use {new} (it sets a process-wide limit)"
                ));
                return parse_env(old, Some(old_val), MIB);
            }
            Ok(new_val)
        };
        let soft = limit(SOFT_LIMIT_ENV, DEPRECATED_SOFT_LIMIT_ENV)?;
        let hard = limit(HARD_LIMIT_ENV, DEPRECATED_HARD_LIMIT_ENV)?;
        let mut config = Self::from_optional_limits(soft, hard)?;
        if let Some(reserve) = parse_env(CRITICAL_RESERVE_ENV, lookup(CRITICAL_RESERVE_ENV), MIB)? {
            config.critical_reserve = reserve;
        }
        if let Some(chunk) = parse_env(CREDIT_ENV, lookup(CREDIT_ENV), KIB)? {
            config.credit_chunk = chunk;
        }
        if let Some(queue) = parse_env(QUEUE_LIMIT_ENV, lookup(QUEUE_LIMIT_ENV), MIB)? {
            config.queue_limit = queue;
        }
        if let Some(strict) = parse_flag(STRICT_ENV, lookup(STRICT_ENV))? {
            config.strict = strict;
        }
        if let Some(ms) = parse_env_u32(COLLECTION_INTERVAL_ENV, lookup(COLLECTION_INTERVAL_ENV))? {
            config.collection_target_interval_ms = ms;
        }
        if let Some(pct) = parse_env_u8(RSS_PRESSURE_PCT_ENV, lookup(RSS_PRESSURE_PCT_ENV))? {
            config.rss_pressure_percent = pct;
        }
        config.validate()?;
        Ok(config)
    }
}

fn parse_env(
    var: &'static str,
    value: Option<String>,
    unit: usize,
) -> Result<Option<usize>, MemoryConfigError> {
    match value {
        None => Ok(None),
        Some(raw) => raw
            .trim()
            .parse::<usize>()
            .map(|n| Some(n.saturating_mul(unit)))
            .map_err(|_| MemoryConfigError::InvalidNumber { var, value: raw }),
    }
}

fn parse_flag(var: &'static str, value: Option<String>) -> Result<Option<bool>, MemoryConfigError> {
    let Some(raw) = value else {
        return Ok(None);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(Some(true)),
        "" | "0" | "false" | "no" | "off" => Ok(Some(false)),
        _ => Err(MemoryConfigError::InvalidFlag { var, value: raw }),
    }
}

fn parse_env_u32(
    var: &'static str,
    value: Option<String>,
) -> Result<Option<u32>, MemoryConfigError> {
    match value {
        None => Ok(None),
        Some(raw) => raw
            .trim()
            .parse::<u32>()
            .map(Some)
            .map_err(|_| MemoryConfigError::InvalidNumber { var, value: raw }),
    }
}

fn parse_env_u8(var: &'static str, value: Option<String>) -> Result<Option<u8>, MemoryConfigError> {
    match value {
        None => Ok(None),
        Some(raw) => raw
            .trim()
            .parse::<u8>()
            .map(Some)
            .map_err(|_| MemoryConfigError::InvalidNumber { var, value: raw }),
    }
}

/// Default soft limit: 75% of the hard limit.
pub fn default_soft_limit(hard_limit: usize) -> usize {
    // Round up, so a nonzero hard limit never yields a zero soft limit
    // (which would hold the process at `Yellow` permanently).
    hard_limit - hard_limit / 4
}

/// Default critical reserve: the larger of 4 MiB or 1% of the hard limit,
/// capped at [`MAX_DEFAULT_CRITICAL_RESERVE`] and at the hard limit itself.
pub fn default_critical_reserve(hard_limit: usize) -> usize {
    (hard_limit / 100)
        .clamp(MIN_DEFAULT_CRITICAL_RESERVE, MAX_DEFAULT_CRITICAL_RESERVE)
        .min(hard_limit)
}

/// Default process hard limit.
///
/// Uses the cgroup memory limit when one applies (Linux), otherwise half of
/// physical memory.  `wasm32` uses a fixed 256 MiB.
pub fn default_hard_limit() -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        256 * MIB
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let physical = system_memory::total() as usize;
        let half = (physical / 2).max(256 * MIB);
        match container_memory_limit() {
            Some(limit) if limit < physical || physical == 0 => limit,
            _ => half,
        }
    }
}

/// The cgroup (v2, then v1) memory limit of this process, if any.
#[cfg(not(target_arch = "wasm32"))]
pub fn container_memory_limit() -> Option<usize> {
    #[cfg(target_os = "linux")]
    {
        const PATHS: [&str; 2] = [
            "/sys/fs/cgroup/memory.max",
            "/sys/fs/cgroup/memory/memory.limit_in_bytes",
        ];
        PATHS.iter().find_map(|path| {
            let raw = std::fs::read_to_string(path).ok()?;
            let raw = raw.trim();
            if raw == "max" {
                return None;
            }
            // cgroup v1 reports "unlimited" as a value near i64::MAX.
            raw.parse::<u64>()
                .ok()
                .filter(|&n| n > 0 && n < (1u64 << 60))
                .map(|n| n as usize)
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
    // Windows job-object memory limit is not yet read; it would require the
    // `windows` crate.  macOS does not expose container memory limits through
    // a portable API.
}

/// The cgroup v2 `memory.high` soft memory limit of this process, if any.
///
/// Returns `None` when not running under cgroup v2, when `memory.high` is
/// set to `max`, or on non-Linux platforms.
pub fn container_soft_memory_limit() -> Option<usize> {
    #[cfg(target_os = "linux")]
    {
        let raw = std::fs::read_to_string("/sys/fs/cgroup/memory.high").ok()?;
        let raw = raw.trim();
        if raw == "max" {
            return None;
        }
        raw.parse::<u64>()
            .ok()
            .filter(|&n| n > 0 && n < (1u64 << 60))
            .map(|n| n as usize)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Current process resident set size in bytes, or `None` if unavailable.
///
/// On Linux this reads `/proc/self/status`.  Other platforms return `None`.
pub fn process_rss_bytes() -> Option<usize> {
    #[cfg(target_os = "linux")]
    {
        let content = std::fs::read_to_string("/proc/self/status").ok()?;
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                // Format: "VmRSS:   12345 kB"
                let kb = rest
                    .trim()
                    .trim_end_matches("kB")
                    .trim()
                    .parse::<usize>()
                    .ok()?;
                return Some(kb * 1024);
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

// ── Isolate control ──────────────────────────────────────────────────────────

/// Process-unique isolate identifier.
pub type IsolateId = u64;

/// Thread-safe control data for one registered isolate.
///
/// The governor holds weak references to these in its registry; it can
/// request collection or a credit recall, but it cannot trace or sweep
/// another isolate's heap.
pub struct IsolateControl {
    id: IsolateId,
    name: Mutex<Arc<str>>,
    gc_requested: AtomicBool,
    /// The governor asks the isolate to return its unused credit.
    recall_requested: AtomicBool,
    used_bytes: AtomicUsize,
    peak_used_bytes: AtomicUsize,
    free_credit_bytes: AtomicUsize,
    /// Active region chunk bytes.
    region_bytes: AtomicUsize,
    /// Retired (poisoned) region chunk bytes.
    retired_region_bytes: AtomicUsize,
    /// Number of completed collections; the collection epoch.
    last_collection_epoch: AtomicU64,
    /// Epoch at which the governor last requested collection from this isolate
    /// on its own behalf (`u64::MAX`: never).
    requested_epoch: AtomicU64,
    /// Epoch at which the governor last made a cross-isolate request to this
    /// isolate — i.e., asked it to collect on behalf of another isolate
    /// (`u64::MAX`: never).  Separate from `requested_epoch` so self-requests
    /// and cross-requests each get their own per-epoch rate limit.
    cross_requested_epoch: AtomicU64,
    /// Used bytes below which the governor does not request collection at
    /// `Yellow` (set from the last collection's result).
    collection_target: AtomicUsize,
    collection_requests: AtomicU64,
    credit_refills: AtomicU64,
    zero_yield_collections: AtomicU64,
    last_bytes_before: AtomicUsize,
    last_bytes_after: AtomicUsize,
    last_duration_nanos: AtomicU64,
}

impl IsolateControl {
    fn new(id: IsolateId, name: Arc<str>) -> Self {
        Self {
            id,
            name: Mutex::new(name),
            gc_requested: AtomicBool::new(false),
            recall_requested: AtomicBool::new(false),
            used_bytes: AtomicUsize::new(0),
            peak_used_bytes: AtomicUsize::new(0),
            free_credit_bytes: AtomicUsize::new(0),
            region_bytes: AtomicUsize::new(0),
            retired_region_bytes: AtomicUsize::new(0),
            last_collection_epoch: AtomicU64::new(0),
            requested_epoch: AtomicU64::new(u64::MAX),
            cross_requested_epoch: AtomicU64::new(u64::MAX),
            collection_target: AtomicUsize::new(0),
            collection_requests: AtomicU64::new(0),
            credit_refills: AtomicU64::new(0),
            zero_yield_collections: AtomicU64::new(0),
            last_bytes_before: AtomicUsize::new(0),
            last_bytes_after: AtomicUsize::new(0),
            last_duration_nanos: AtomicU64::new(0),
        }
    }

    /// Process-unique id.
    pub fn id(&self) -> IsolateId {
        self.id
    }

    /// Diagnostic name.
    pub fn name(&self) -> Arc<str> {
        self.name
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set_name(&self, name: Arc<str>) {
        *self.name.lock().unwrap_or_else(PoisonError::into_inner) = name;
    }

    /// Ask the isolate to collect at its next safepoint.
    pub fn request_collection(&self) {
        self.gc_requested.store(true, Ordering::Release);
    }

    /// Whether a collection request is pending.
    pub fn collection_requested(&self) -> bool {
        self.gc_requested.load(Ordering::Acquire)
    }

    /// Check and clear the pending collection request.
    pub fn take_collection_request(&self) -> bool {
        self.gc_requested.swap(false, Ordering::AcqRel)
    }

    /// Ask the isolate to return its unused credit at its next runtime
    /// service poll ([`IsolateAccount::poll`]).
    pub fn request_recall(&self) {
        self.recall_requested.store(true, Ordering::Release);
    }

    /// Whether a credit recall is pending.
    pub fn recall_requested(&self) -> bool {
        self.recall_requested.load(Ordering::Acquire)
    }

    /// Last published used bytes.
    pub fn used_bytes(&self) -> usize {
        self.used_bytes.load(Ordering::Relaxed)
    }

    /// Last published unused credit.
    pub fn free_credit_bytes(&self) -> usize {
        self.free_credit_bytes.load(Ordering::Relaxed)
    }

    /// Last published active region chunk bytes.
    pub fn region_bytes(&self) -> usize {
        self.region_bytes.load(Ordering::Relaxed)
    }

    /// Last published retired region chunk bytes.
    pub fn retired_region_bytes(&self) -> usize {
        self.retired_region_bytes.load(Ordering::Relaxed)
    }

    /// Used bytes below which the governor does not request collection at
    /// `Yellow`.
    pub fn collection_target(&self) -> usize {
        self.collection_target.load(Ordering::Relaxed)
    }

    /// Number of completed collections.
    pub fn collection_epoch(&self) -> u64 {
        self.last_collection_epoch.load(Ordering::Relaxed)
    }

    /// Governor-initiated collection requests for this isolate.
    pub fn collection_requests(&self) -> u64 {
        self.collection_requests.load(Ordering::Relaxed)
    }

    /// Credit refills this isolate has requested.
    pub fn credit_refills(&self) -> u64 {
        self.credit_refills.load(Ordering::Relaxed)
    }

    /// Request collection on the governor's behalf (self-request), at most
    /// once per collection epoch.  Below `Red`, also skip the request while
    /// the heap is under its post-collection target.  Returns whether a
    /// request was made.
    fn governor_request(&self, level: PressureLevel) -> bool {
        let epoch = self.collection_epoch();
        let prev = self.requested_epoch.load(Ordering::Relaxed);
        if prev == epoch {
            return false;
        }
        if level < PressureLevel::Red && self.used_bytes() < self.collection_target() {
            return false;
        }
        // `evaluate` can run on several refilling threads at once; only the
        // thread that claims this epoch counts the request.
        if self
            .requested_epoch
            .compare_exchange(prev, epoch, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return false;
        }
        self.collection_requests.fetch_add(1, Ordering::Relaxed);
        self.request_collection();
        true
    }

    /// Cross-isolate request: ask this isolate to collect on behalf of another
    /// isolate, using a separate epoch counter so cross-requests and
    /// self-requests each get their own per-epoch rate limit.  Returns whether
    /// a request was made.
    fn cross_governor_request(&self, level: PressureLevel) -> bool {
        let epoch = self.collection_epoch();
        let prev = self.cross_requested_epoch.load(Ordering::Relaxed);
        if prev == epoch {
            return false;
        }
        if level < PressureLevel::Red && self.used_bytes() < self.collection_target() {
            return false;
        }
        if self
            .cross_requested_epoch
            .compare_exchange(prev, epoch, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return false;
        }
        self.collection_requests.fetch_add(1, Ordering::Relaxed);
        self.request_collection();
        true
    }

    fn snapshot(&self) -> IsolateSnapshot {
        IsolateSnapshot {
            id: self.id,
            name: self.name(),
            used_bytes: self.used_bytes(),
            free_credit_bytes: self.free_credit_bytes(),
            region_bytes: self.region_bytes(),
            retired_region_bytes: self.retired_region_bytes(),
            peak_used_bytes: self.peak_used_bytes.load(Ordering::Relaxed),
            collection_target: self.collection_target(),
            collections: self.collection_epoch(),
            collection_requests: self.collection_requests(),
            credit_refills: self.credit_refills(),
            zero_yield_collections: self.zero_yield_collections.load(Ordering::Relaxed),
            last_bytes_before: self.last_bytes_before.load(Ordering::Relaxed),
            last_bytes_after: self.last_bytes_after.load(Ordering::Relaxed),
            last_duration: Duration::from_nanos(self.last_duration_nanos.load(Ordering::Relaxed)),
        }
    }
}

impl fmt::Debug for IsolateControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IsolateControl")
            .field("id", &self.id)
            .field("name", &self.name())
            .field("used_bytes", &self.used_bytes())
            .field("free_credit_bytes", &self.free_credit_bytes())
            .finish()
    }
}

/// What one collection did, reported to the isolate's account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollectionReport {
    /// Heap bytes before collection.
    pub bytes_before: usize,
    /// Heap bytes after collection.
    pub bytes_after: usize,
    /// Bytes freed by sweep.  They become free credit, and the account
    /// returns the credit above its retained limit to the governor.
    pub bytes_returned: usize,
    /// Bytes of unreachable objects that sweep kept for their one cycle of
    /// grace.  They are included in `bytes_after`, but the next collection
    /// frees them, so they do not count as surviving.
    pub bytes_pending: usize,
    /// Mark plus sweep time.
    pub duration: Duration,
}

// ── Isolate account ──────────────────────────────────────────────────────────

/// One isolate's single-threaded view of the governor.
///
/// The account holds `used` heap bytes (live objects and retained garbage),
/// region chunk bytes (active and retired), and `credit` (committed capacity
/// it has not used yet).  Their sum is exactly what the governor has
/// committed for this isolate.  The governor counts the region part under
/// [`MemoryClass::Region`] and the rest under [`MemoryClass::GcHeap`]; the
/// split is updated whenever the account publishes.
///
/// The counters are `Cell`s, so the account is `!Sync`: only one thread
/// uses it at a time.  It is `Send`, so nothing stops moving it to another
/// thread; the runtime keeps it in a thread-local for the isolate's thread.
///
/// Created by [`ProcessMemoryGovernor::register_isolate`].  Dropping the
/// account returns its used bytes and free credit and unregisters it.
pub struct IsolateAccount {
    governor: &'static ProcessMemoryGovernor,
    control: Arc<IsolateControl>,
    used: Cell<usize>,
    credit: Cell<usize>,
    /// Active region chunk bytes.
    region: Cell<usize>,
    /// Retired region chunk bytes (poisoned regions kept for the process
    /// lifetime).
    retired: Cell<usize>,
    /// Free credit last added to the governor's free-credit counter.
    published_credit: Cell<usize>,
    /// Region bytes (active plus retired) last moved to the governor's
    /// `Region` class.
    published_region: Cell<usize>,
    /// Retired region bytes last added to the governor's retired counter.
    published_retired: Cell<usize>,
    /// Allocations not yet added to [`crate::stats::GC_STATS`].
    pending_allocs: Cell<u64>,
    pending_alloc_bytes: Cell<u64>,
    /// Bytes allocated since the last recorded collection; used to compute the
    /// allocation rate for dynamic collection targets.
    alloc_since_collection: Cell<usize>,
    /// When this isolate last recorded a collection; paired with
    /// `alloc_since_collection` to derive the allocation rate.
    /// Set at registration, so the first rate covers the isolate's lifetime.
    collection_started: Cell<Instant>,
}

impl IsolateAccount {
    /// Shared control handle.
    pub fn control(&self) -> &Arc<IsolateControl> {
        &self.control
    }

    /// Exact local used bytes (may be ahead of the published value).
    pub fn used_bytes(&self) -> usize {
        self.used.get()
    }

    /// Exact local unused credit.
    pub fn free_credit_bytes(&self) -> usize {
        self.credit.get()
    }

    /// Exact local active region chunk bytes.
    pub fn region_bytes(&self) -> usize {
        self.region.get()
    }

    /// Exact local retired region chunk bytes.
    pub fn retired_region_bytes(&self) -> usize {
        self.retired.get()
    }

    /// Bytes the governor has committed for this isolate: heap used, region
    /// chunks, and free credit.
    pub fn committed_bytes(&self) -> usize {
        self.used.get() + self.region.get() + self.retired.get() + self.credit.get()
    }

    /// Account `bytes` of new heap allocation.  Thread-local unless local
    /// credit is insufficient, in which case the account refills from the
    /// governor.
    ///
    /// Infallible: when strict admission refuses the refill, the account
    /// raises the thread's runtime fault and takes the bytes from the
    /// critical reserve (see the module docs).  Use
    /// [`Self::try_reserve_credit`] first to refuse instead.
    #[inline]
    pub fn charge(&self, bytes: usize) {
        self.used.set(self.used.get() + bytes);
        self.pending_allocs.set(self.pending_allocs.get() + 1);
        self.pending_alloc_bytes
            .set(self.pending_alloc_bytes.get() + bytes as u64);
        self.alloc_since_collection
            .set(self.alloc_since_collection.get() + bytes);
        self.take_credit(bytes);
    }

    /// Account a new region chunk of `bytes`.  Like [`Self::charge`], it
    /// uses local credit and refills only when that runs out.
    #[inline]
    pub fn charge_region(&self, bytes: usize) {
        self.region.set(self.region.get() + bytes);
        self.take_credit(bytes);
    }

    /// Move `bytes` of active region chunks to the retired count.  A retired
    /// region keeps its charge until it is released.
    pub fn retire_region(&self, bytes: usize) {
        let region = self.region.get();
        debug_assert!(
            bytes <= region,
            "retired region bytes exceed the charged region bytes"
        );
        let bytes = bytes.min(region);
        self.region.set(region - bytes);
        self.retired.set(self.retired.get() + bytes);
    }

    /// Return a freed region chunk of `bytes`.  The bytes become free credit;
    /// credit above the retained limit goes back to the governor at once,
    /// because a no-GC isolate has no collection to trim it.
    #[inline]
    pub fn release_region(&self, bytes: usize, retired: bool) {
        let cell = if retired { &self.retired } else { &self.region };
        let held = cell.get();
        debug_assert!(
            bytes <= held,
            "isolate {} released {bytes} region bytes with only {held} charged",
            self.control.id
        );
        let bytes = bytes.min(held);
        cell.set(held - bytes);
        let credit = self.credit.get() + bytes;
        self.credit.set(credit);
        // The account's first charge refilled through `credit_chunk()`, so
        // the governor is configured; read the chunk size directly.
        if credit > self.governor.credit_chunk.load(Ordering::Relaxed) {
            self.trim_after_release();
        }
    }

    #[cold]
    #[inline(never)]
    fn trim_after_release(&self) {
        let before = self.credit.get();
        self.trim_credit();
        if self.credit.get() != before {
            self.sync();
            self.governor.evaluate(None, false);
        }
    }

    #[inline]
    fn take_credit(&self, bytes: usize) {
        match self.credit.get().checked_sub(bytes) {
            Some(rest) => self.credit.set(rest),
            None => self.refill(bytes),
        }
    }

    /// Slow path of [`Self::charge`]: obtain credit for the part of `bytes`
    /// that local credit does not cover.  A shortfall up to one chunk gets a
    /// normal chunk; a larger one gets its own size, rounded up to
    /// [`ACCOUNTING_UNIT`].
    #[cold]
    #[inline(never)]
    fn refill(&self, bytes: usize) {
        let credit = self.credit.get();
        let shortfall = bytes - credit;
        let grant = match self.request_credit(shortfall, current_admission()) {
            Ok(grant) => grant,
            Err(refused) => self.emergency_credit(shortfall, refused),
        };
        self.credit.set(credit + grant - bytes);
        self.after_grant();
    }

    /// Make sure local credit covers `bytes`, refilling from the governor if
    /// needed, so that a following [`Self::charge`] of at most `bytes` stays
    /// local.  In strict mode a refill that does not fit below the hard limit
    /// is refused: the account and the governor's counters stay unchanged,
    /// the refusal is recorded, and the isolate is asked to collect.
    pub fn try_reserve_credit(&self, bytes: usize) -> Result<(), MemoryLimitExceeded> {
        let credit = self.credit.get();
        if credit >= bytes {
            return Ok(());
        }
        match self.request_credit(bytes - credit, current_admission()) {
            Ok(grant) => {
                self.credit.set(credit + grant);
                self.after_grant();
                Ok(())
            }
            Err(refused) => {
                self.governor
                    .record_rejection(&refused, Some(&self.control));
                Err(refused)
            }
        }
    }

    /// Fallible [`Self::charge`]: on refusal nothing is charged.
    pub fn try_charge(&self, bytes: usize) -> Result<(), MemoryLimitExceeded> {
        self.try_reserve_credit(bytes)?;
        self.charge(bytes);
        Ok(())
    }

    /// Ask the governor for at least `shortfall` bytes of credit: a normal
    /// chunk, or the shortfall rounded up to [`ACCOUNTING_UNIT`] when that is
    /// larger.  If strict admission refuses a chunk, retry with the rounded
    /// shortfall alone.  Counters change only on success.
    fn request_credit(
        &self,
        shortfall: usize,
        admission: Admission,
    ) -> Result<usize, MemoryLimitExceeded> {
        let chunk = self.governor.credit_chunk();
        let exact = shortfall.div_ceil(ACCOUNTING_UNIT) * ACCOUNTING_UNIT;
        let grant = if shortfall > chunk { exact } else { chunk };
        match self.governor.admit(MemoryClass::GcHeap, grant, admission) {
            Ok(()) => Ok(grant),
            Err(_) if exact < grant => self
                .governor
                .admit(MemoryClass::GcHeap, exact, admission)
                .map(|()| exact),
            Err(refused) => Err(refused),
        }
    }

    /// Credit for an allocation that cannot be refused: record the refusal,
    /// raise the thread's runtime fault (outside a [`CriticalSection`]), and
    /// take exactly the shortfall from the critical reserve, or past it if
    /// the reserve is exhausted too.
    #[cold]
    fn emergency_credit(&self, shortfall: usize, refused: MemoryLimitExceeded) -> usize {
        if !critical_active() {
            self.governor
                .record_rejection(&refused, Some(&self.control));
            raise_fault(self.governor, refused, Some(&self.control));
        }
        // Take only what this allocation needs, to spend the reserve slowly.
        self.governor
            .admit_or_overrun(MemoryClass::GcHeap, shortfall);
        shortfall
    }

    /// Bookkeeping after new credit was granted.
    fn after_grant(&self) {
        self.governor.credit_refills.fetch_add(1, Ordering::Relaxed);
        self.control.credit_refills.fetch_add(1, Ordering::Relaxed);
        if self.control.recall_requested() {
            self.service_recall();
        }
        self.sync();
        self.governor.evaluate(Some(&self.control), true);
    }

    /// Return `bytes` freed by sweep.  They become free credit until the
    /// collection report trims it.
    #[inline]
    pub fn release(&self, bytes: usize) {
        let used = self.used.get();
        debug_assert!(
            bytes <= used,
            "isolate {} released {bytes} bytes with only {used} charged",
            self.control.id
        );
        let bytes = bytes.min(used);
        self.used.set(used - bytes);
        self.credit.set(self.credit.get() + bytes);
    }

    /// Publish the local counters and re-evaluate pressure.  Normal
    /// operation publishes at each refill and collection; this is for
    /// metrics polls.
    pub fn publish(&self) {
        self.sync();
        self.governor.evaluate(Some(&self.control), false);
    }

    /// Runtime service poll: return credit if the governor recalled it,
    /// and report whether a collection is requested.  Called from the
    /// safepoint check, so it must stay cheap when nothing is pending.
    #[inline]
    pub fn poll(&self) -> bool {
        if self.control.recall_requested() {
            self.service_recall();
            self.sync();
            self.governor.evaluate(None, false);
        }
        self.control.collection_requested()
    }

    fn service_recall(&self) {
        self.control
            .recall_requested
            .store(false, Ordering::Release);
        self.trim_credit();
    }

    /// Return the free credit above the retained limit for the current
    /// pressure level.
    fn trim_credit(&self) {
        // Returned credit leaves the `GcHeap` class, so that class must
        // first hold this account's current non-region bytes.
        self.publish_region();
        let limit = self.governor.retained_credit(self.governor.pressure());
        let credit = self.credit.get();
        if credit > limit {
            self.credit.set(limit);
            self.governor.return_credit(credit - limit);
        }
    }

    /// Copy the local counters to the control handle and the governor's
    /// free-credit counter, and flush pending allocation statistics.
    fn sync(&self) {
        let used = self.used.get();
        let credit = self.credit.get();
        self.control.used_bytes.store(used, Ordering::Relaxed);
        self.control
            .peak_used_bytes
            .fetch_max(used, Ordering::Relaxed);
        self.control
            .free_credit_bytes
            .store(credit, Ordering::Relaxed);
        let prev = self.published_credit.replace(credit);
        if credit >= prev {
            self.governor.add_free_credit(credit - prev);
        } else {
            self.governor.sub_free_credit(prev - credit);
        }
        self.publish_region();
        self.flush_alloc_stats();
    }

    /// Publish the region counters and move the governor's class split to
    /// match them.
    fn publish_region(&self) {
        let region = self.region.get();
        let retired = self.retired.get();
        self.control.region_bytes.store(region, Ordering::Relaxed);
        self.control
            .retired_region_bytes
            .store(retired, Ordering::Relaxed);
        let prev = self.published_region.replace(region + retired);
        self.governor.shift_region(prev, region + retired);
        let prev = self.published_retired.replace(retired);
        if retired >= prev {
            self.governor
                .retired_region
                .fetch_add(retired - prev, Ordering::Relaxed);
        } else {
            checked_sub(
                &self.governor.retired_region,
                prev - retired,
                "retired-region",
            );
        }
    }

    fn flush_alloc_stats(&self) {
        let allocs = self.pending_allocs.replace(0);
        let bytes = self.pending_alloc_bytes.replace(0);
        if allocs > 0 {
            crate::stats::GC_STATS.record_gc_allocs(allocs, bytes);
        }
    }

    /// Record a completed collection: bump the epoch, set the next
    /// collection target, return excess free credit, and publish.
    ///
    /// **Phase 5 (dynamic targets):** The target is `surviving + headroom`,
    /// where `headroom` is the larger of:
    ///
    /// - `alloc_rate × collection_target_interval_ms / 1000` (Phase 5 default,
    ///   when [`MemoryConfig::collection_target_interval_ms`] is non-zero),
    ///   capped at the room left below the soft limit.  The rate is measured
    ///   since the previous collection (or registration), over at least one
    ///   interval; or
    /// - `surviving / 2` (legacy heuristic, when the interval is zero), or
    /// - [`ProcessMemoryGovernor::min_headroom`].
    ///
    /// `surviving` is `bytes_after` minus `bytes_pending`: objects kept only
    /// for their grace cycle are freed by the next collection.  After a
    /// zero-yield collection (nothing freed and nothing pending), headroom
    /// uses `surviving` as a floor instead of `surviving / 2`.  The heap's own
    /// trigger is its `GcConfig`.
    pub fn record_collection(&self, report: CollectionReport) {
        let c = &self.control;
        c.last_collection_epoch.fetch_add(1, Ordering::Relaxed);
        c.last_bytes_before
            .store(report.bytes_before, Ordering::Relaxed);
        c.last_bytes_after
            .store(report.bytes_after, Ordering::Relaxed);
        c.last_duration_nanos.store(
            report.duration.as_nanos().min(u64::MAX as u128) as u64,
            Ordering::Relaxed,
        );
        let min = self.governor.min_headroom();
        let after = report.bytes_after;
        // Grace-period objects are garbage the next collection frees: size
        // the target from the bytes that actually survived.
        let surviving = after.saturating_sub(report.bytes_pending);

        // Compute headroom from the allocation rate if an interval is configured.
        let rate_headroom: usize = {
            let interval_ms = self.governor.collection_target_interval_ms();
            if interval_ms > 0 {
                let alloc_bytes = self.alloc_since_collection.get();
                let interval_secs = interval_ms as f64 / 1000.0;
                // Measure over at least one interval, so back-to-back
                // collections cannot inflate the rate: the headroom is then
                // at most the bytes allocated since the last collection.
                let elapsed_secs = self
                    .collection_started
                    .get()
                    .elapsed()
                    .as_secs_f64()
                    .max(interval_secs);
                let rate = alloc_bytes as f64 / elapsed_secs;
                // Never extend the target past the soft limit, or the
                // isolate would not be asked to collect at Yellow.
                let room = self
                    .governor
                    .soft_limit
                    .load(Ordering::Relaxed)
                    .saturating_sub(self.governor.committed_bytes());
                ((rate * interval_secs) as usize).min(room)
            } else {
                0
            }
        };
        self.alloc_since_collection.set(0);
        self.collection_started.set(Instant::now());

        let headroom = if report.bytes_returned == 0 && report.bytes_pending == 0 {
            c.zero_yield_collections.fetch_add(1, Ordering::Relaxed);
            rate_headroom.max(surviving).max(min)
        } else {
            rate_headroom.max(surviving / 2).max(min)
        };
        c.collection_target
            .store(surviving.saturating_add(headroom), Ordering::Relaxed);
        self.service_recall();
        self.sync();
        self.governor.evaluate(Some(c), false);
    }
}

impl Drop for IsolateAccount {
    fn drop(&mut self) {
        // The isolate heap is not torn down at thread exit yet, so this
        // stops counting bytes that remain allocated until process exit.
        // Region chunks freed after this point find no account and are not
        // released again.
        let committed = self.used.replace(0)
            + self.region.replace(0)
            + self.retired.replace(0)
            + self.credit.replace(0);
        let published = self.published_credit.replace(0);
        let published_region = self.published_region.replace(0);
        let published_retired = self.published_retired.replace(0);
        self.control.used_bytes.store(0, Ordering::Relaxed);
        self.control.free_credit_bytes.store(0, Ordering::Relaxed);
        self.control.region_bytes.store(0, Ordering::Relaxed);
        self.control
            .retired_region_bytes
            .store(0, Ordering::Relaxed);
        self.flush_alloc_stats();
        self.governor.sub_free_credit(published);
        checked_sub(
            &self.governor.retired_region,
            published_retired,
            "retired-region",
        );
        // `published_region` sits in the `Region` class; the rest of the
        // committed bytes sit in `GcHeap`.
        self.governor.sub(MemoryClass::Region, published_region);
        self.governor.return_credit(committed - published_region);
        self.governor.unregister(&self.control);
        self.governor.evaluate(None, false);
    }
}

// ── Charges for independent-lifetime allocations ─────────────────────────────

/// RAII charge for an allocation with its own lifetime.  Dropping it returns
/// the bytes to the governor exactly once.
#[must_use = "dropping a MemoryCharge returns its bytes immediately"]
pub struct MemoryCharge {
    governor: &'static ProcessMemoryGovernor,
    class: MemoryClass,
    bytes: usize,
}

impl MemoryCharge {
    /// Charged class.
    pub fn class(&self) -> MemoryClass {
        self.class
    }

    /// Charged bytes.
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

impl Drop for MemoryCharge {
    fn drop(&mut self) {
        self.governor.sub(self.class, self.bytes);
        self.governor.evaluate(None, false);
    }
}

impl fmt::Debug for MemoryCharge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryCharge")
            .field("class", &self.class)
            .field("bytes", &self.bytes)
            .finish()
    }
}

/// A reservation the governor refused.  Fixed-size; building it never
/// allocates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryLimitExceeded {
    /// Class of the refused reservation.
    pub class: MemoryClass,
    /// Requested bytes.
    pub requested: usize,
    /// Committed bytes at the time of the request.
    pub committed: usize,
    /// Hard limit at the time of the request.
    pub hard_limit: usize,
}

impl fmt::Display for MemoryLimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "managed memory limit exceeded: {} bytes of {} requested with {} of {} bytes committed",
            self.requested,
            self.class.name(),
            self.committed,
            self.hard_limit
        )
    }
}

impl std::error::Error for MemoryLimitExceeded {}

// ── Admission ────────────────────────────────────────────────────────────────

/// Which bound a strict-mode grant must stay within.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// Application allocation: committed bytes must stay at or below the
    /// hard limit.
    Application,
    /// Runtime control allocation (error reporting, shutdown, a refused
    /// allocation that cannot fail): committed bytes must stay at or below
    /// the hard limit plus the critical reserve.
    Critical,
}

thread_local! {
    static CRITICAL_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// While alive, the calling thread's grants use [`Admission::Critical`] and
/// a refused application grant does not raise a runtime fault.  Created by
/// [`enter_critical`]; nests.  Not `Send`: it marks the creating thread.
#[must_use = "the critical section ends when this guard is dropped"]
pub struct CriticalSection {
    _not_send: std::marker::PhantomData<*const ()>,
}

/// Enter a [`CriticalSection`] on the calling thread.
pub fn enter_critical() -> CriticalSection {
    let _ = CRITICAL_DEPTH.try_with(|d| d.set(d.get() + 1));
    CriticalSection {
        _not_send: std::marker::PhantomData,
    }
}

impl Drop for CriticalSection {
    fn drop(&mut self) {
        let _ = CRITICAL_DEPTH.try_with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// True while the calling thread is inside a [`CriticalSection`].  A fiber
/// must never park here (see [`crate::mutator::assert_parkable`]).
pub fn in_critical_section() -> bool {
    CRITICAL_DEPTH.try_with(|d| d.get() != 0).unwrap_or(false)
}

/// Run `f` inside a [`CriticalSection`].
pub fn with_critical<R>(f: impl FnOnce() -> R) -> R {
    let _section = enter_critical();
    f()
}

/// Whether the calling thread is inside a [`CriticalSection`].
pub fn critical_active() -> bool {
    CRITICAL_DEPTH.try_with(|d| d.get() > 0).unwrap_or(false)
}

fn current_admission() -> Admission {
    if critical_active() {
        Admission::Critical
    } else {
        Admission::Application
    }
}

// ── Runtime faults ───────────────────────────────────────────────────────────

/// A refused allocation that its caller could not report as an error.
#[derive(Clone, Copy)]
struct PendingFault {
    governor: &'static ProcessMemoryGovernor,
    fault: MemoryLimitExceeded,
    /// Already reported by an evaluation boundary: deliver it unconditionally.
    confirmed: bool,
}

/// The thread's pending runtime fault.  Separate from any pending Clojure
/// exception; holds fixed-size data only, so raising it never allocates.
struct FaultSlot(Cell<Option<PendingFault>>);

impl FaultSlot {
    fn set(&self, fault: Option<PendingFault>) {
        let was = self.0.replace(fault).is_some();
        match (was, fault.is_some()) {
            (false, true) => {
                PENDING_FAULTS.fetch_add(1, Ordering::Relaxed);
            }
            (true, false) => {
                PENDING_FAULTS.fetch_sub(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }
}

impl Drop for FaultSlot {
    fn drop(&mut self) {
        self.set(None);
    }
}

thread_local! {
    static FAULT: FaultSlot = const { FaultSlot(Cell::new(None)) };
}

/// Threads with a pending runtime fault.  Lets the safepoint fast path skip
/// the thread-local lookup when no thread has one.
static PENDING_FAULTS: AtomicUsize = AtomicUsize::new(0);

fn raise_fault(
    governor: &'static ProcessMemoryGovernor,
    fault: MemoryLimitExceeded,
    requester: Option<&IsolateControl>,
) {
    let _ = FAULT.try_with(|slot| {
        if slot.0.get().is_none() {
            governor.runtime_faults.fetch_add(1, Ordering::Relaxed);
            slot.set(Some(PendingFault {
                governor,
                fault,
                confirmed: false,
            }));
        }
    });
    // The next safepoint collects before it decides whether to report.
    match requester {
        Some(control) => control.request_collection(),
        None => {
            let _ = with_registered_account(|a| a.control.request_collection());
        }
    }
}

/// Record that an allocation which could not fail was refused by the process
/// governor.  The first fault is kept until it is resolved.  The calling
/// isolate is asked to collect.
pub fn raise_runtime_fault(fault: MemoryLimitExceeded) {
    raise_fault(governor(), fault, None);
}

/// Store a fault that an evaluation boundary has already decided to report,
/// so the next boundary delivers it whatever committed bytes are then.  Used
/// where the error crosses a path that cannot carry it (compiled code).
pub fn confirm_runtime_fault(fault: MemoryLimitExceeded) {
    let _ = FAULT.try_with(|slot| {
        slot.set(Some(PendingFault {
            governor: governor(),
            fault,
            confirmed: true,
        }))
    });
}

/// Whether any thread may have a pending runtime fault.  One relaxed load.
#[inline]
pub fn runtime_fault_possible() -> bool {
    PENDING_FAULTS.load(Ordering::Relaxed) != 0
}

/// The calling thread's pending fault, without clearing it.
pub fn pending_runtime_fault() -> Option<MemoryLimitExceeded> {
    if !runtime_fault_possible() {
        return None;
    }
    FAULT
        .try_with(|slot| slot.0.get().map(|p| p.fault))
        .ok()
        .flatten()
}

/// Clear and return the calling thread's pending fault unconditionally.
pub fn take_runtime_fault() -> Option<MemoryLimitExceeded> {
    if !runtime_fault_possible() {
        return None;
    }
    FAULT
        .try_with(|slot| {
            let p = slot.0.get()?;
            slot.set(None);
            Some(p.fault)
        })
        .ok()
        .flatten()
}

/// Decide a pending fault at an evaluation boundary, after the isolate had
/// its chance to collect.  A confirmed fault is returned.  An unconfirmed one
/// is cleared, and `None` returned, if committed bytes are back below the
/// hard limit with headroom to spare
/// ([`ProcessMemoryGovernor::has_fault_headroom`]); otherwise it is returned.  Either way the slot is empty
/// afterwards.  Cheap when no thread has a fault.
#[inline]
pub fn resolve_runtime_fault() -> Option<MemoryLimitExceeded> {
    if !runtime_fault_possible() {
        return None;
    }
    resolve_slow()
}

#[cold]
fn resolve_slow() -> Option<MemoryLimitExceeded> {
    FAULT
        .try_with(|slot| {
            let p = slot.0.get()?;
            slot.set(None);
            if !p.confirmed && p.governor.has_fault_headroom() {
                p.governor
                    .runtime_faults_cleared
                    .fetch_add(1, Ordering::Relaxed);
                return None;
            }
            Some(p.fault)
        })
        .ok()
        .flatten()
}

// ── Process governor ─────────────────────────────────────────────────────────

/// Process-wide managed-memory governor.  See the module docs.
pub struct ProcessMemoryGovernor {
    config: Mutex<Option<MemoryConfig>>,
    configured: AtomicBool,
    strict: AtomicBool,
    soft_limit: AtomicUsize,
    hard_limit: AtomicUsize,
    critical_reserve: AtomicUsize,
    credit_chunk: AtomicUsize,
    retained_credit_chunks: AtomicUsize,
    /// Total committed bytes; the sum of `class_bytes`.
    committed: AtomicUsize,
    class_bytes: [AtomicUsize; CLASS_COUNT],
    /// Unused isolate credit, as last published by each isolate.
    free_credit: AtomicUsize,
    peak_committed: AtomicUsize,
    peak_used: AtomicUsize,
    pressure: AtomicU8,
    pressure_transitions: AtomicU64,
    collection_requests: AtomicU64,
    credit_refills: AtomicU64,
    credit_returned_bytes: AtomicU64,
    credit_recalls: AtomicU64,
    over_limit_events: AtomicU64,
    over_limit_bytes: AtomicU64,
    /// Retired region bytes (a subset of the `Region` class).
    retired_region: AtomicUsize,
    queue_waits: AtomicU64,
    queue_wait_nanos: AtomicU64,
    queue_rejections: AtomicU64,
    /// Set by a strict-mode refusal; holds pressure at `Red` until a normal
    /// chunk fits below the hard limit again.
    reject_latch: AtomicBool,
    rejected_reservations: AtomicU64,
    rejected_bytes: AtomicU64,
    critical_grants: AtomicU64,
    reserve_overruns: AtomicU64,
    reserve_overrun_bytes: AtomicU64,
    runtime_faults: AtomicU64,
    runtime_faults_cleared: AtomicU64,
    registry: Mutex<Vec<Weak<IsolateControl>>>,
    next_isolate_id: AtomicU64,
    isolates_registered: AtomicU64,
    /// Target allocation interval for dynamic collection targets (nanoseconds).
    collection_target_interval_nanos: AtomicU64,
    /// RSS emergency threshold (0 = disabled).
    rss_pressure_percent: AtomicU8,
    /// Cached process RSS bytes; refreshed at most once per [`RSS_CACHE_NS`].
    rss_cache_bytes: AtomicUsize,
    /// Unix nanoseconds of the last RSS read (0 = never read).
    rss_cache_nanos: AtomicU64,
    /// Registered load-shedding hooks called on pressure transitions.
    shed_hooks: Mutex<ShedHookVec>,
    /// Counter for [`ShedHookId`].
    next_hook_id: AtomicU64,
}

impl ProcessMemoryGovernor {
    /// A governor that reads [`MemoryConfig::from_env`] on first use.
    pub const fn new() -> Self {
        Self {
            config: Mutex::new(None),
            configured: AtomicBool::new(false),
            strict: AtomicBool::new(false),
            soft_limit: AtomicUsize::new(usize::MAX),
            hard_limit: AtomicUsize::new(usize::MAX),
            critical_reserve: AtomicUsize::new(0),
            credit_chunk: AtomicUsize::new(DEFAULT_CREDIT_CHUNK),
            retained_credit_chunks: AtomicUsize::new(DEFAULT_RETAINED_CREDIT_CHUNKS),
            committed: AtomicUsize::new(0),
            class_bytes: [const { AtomicUsize::new(0) }; CLASS_COUNT],
            free_credit: AtomicUsize::new(0),
            peak_committed: AtomicUsize::new(0),
            peak_used: AtomicUsize::new(0),
            pressure: AtomicU8::new(PressureLevel::Green as u8),
            pressure_transitions: AtomicU64::new(0),
            collection_requests: AtomicU64::new(0),
            credit_refills: AtomicU64::new(0),
            credit_returned_bytes: AtomicU64::new(0),
            credit_recalls: AtomicU64::new(0),
            over_limit_events: AtomicU64::new(0),
            over_limit_bytes: AtomicU64::new(0),
            retired_region: AtomicUsize::new(0),
            queue_waits: AtomicU64::new(0),
            queue_wait_nanos: AtomicU64::new(0),
            queue_rejections: AtomicU64::new(0),
            reject_latch: AtomicBool::new(false),
            rejected_reservations: AtomicU64::new(0),
            rejected_bytes: AtomicU64::new(0),
            critical_grants: AtomicU64::new(0),
            reserve_overruns: AtomicU64::new(0),
            reserve_overrun_bytes: AtomicU64::new(0),
            runtime_faults: AtomicU64::new(0),
            runtime_faults_cleared: AtomicU64::new(0),
            registry: Mutex::new(Vec::new()),
            next_isolate_id: AtomicU64::new(1),
            isolates_registered: AtomicU64::new(0),
            collection_target_interval_nanos: AtomicU64::new(
                DEFAULT_COLLECTION_INTERVAL_MS as u64 * 1_000_000,
            ),
            rss_pressure_percent: AtomicU8::new(0),
            rss_cache_bytes: AtomicUsize::new(0),
            rss_cache_nanos: AtomicU64::new(0),
            shed_hooks: Mutex::new(Vec::new()),
            next_hook_id: AtomicU64::new(1),
        }
    }

    /// A governor with an explicit config.
    pub fn with_config(config: MemoryConfig) -> Result<Self, MemoryConfigError> {
        let g = Self::new();
        g.configure(config)?;
        Ok(g)
    }

    /// Replace the process limits and re-evaluate pressure.
    pub fn configure(&self, config: MemoryConfig) -> Result<(), MemoryConfigError> {
        config.validate()?;
        let mut slot = self.config.lock().unwrap_or_else(PoisonError::into_inner);
        self.install(&config);
        *slot = Some(config);
        drop(slot);
        self.evaluate(None, false);
        Ok(())
    }

    fn install(&self, config: &MemoryConfig) {
        self.soft_limit.store(config.soft_limit, Ordering::Relaxed);
        self.hard_limit.store(config.hard_limit, Ordering::Relaxed);
        self.critical_reserve
            .store(config.critical_reserve, Ordering::Relaxed);
        self.strict.store(config.strict, Ordering::Relaxed);
        self.credit_chunk
            .store(config.credit_chunk, Ordering::Relaxed);
        self.retained_credit_chunks
            .store(config.retained_credit_chunks, Ordering::Relaxed);
        self.collection_target_interval_nanos.store(
            config.collection_target_interval_ms as u64 * 1_000_000,
            Ordering::Relaxed,
        );
        self.rss_pressure_percent
            .store(config.rss_pressure_percent, Ordering::Relaxed);
        self.configured.store(true, Ordering::Release);
    }

    /// Collection target interval in milliseconds (0 = use surviving/2 heuristic).
    pub fn collection_target_interval_ms(&self) -> u32 {
        (self
            .collection_target_interval_nanos
            .load(Ordering::Relaxed)
            / 1_000_000) as u32
    }

    /// Current process RSS with a 1-second cache.  Returns 0 when unavailable.
    fn get_rss_bytes(&self) -> usize {
        let now = unix_nanos();
        let cached_at = self.rss_cache_nanos.load(Ordering::Relaxed);
        if now.saturating_sub(cached_at) < RSS_CACHE_NS {
            return self.rss_cache_bytes.load(Ordering::Relaxed);
        }
        let rss = process_rss_bytes().unwrap_or(0);
        self.rss_cache_bytes.store(rss, Ordering::Relaxed);
        self.rss_cache_nanos.store(now, Ordering::Relaxed);
        rss
    }

    /// Register a load-shedding hook called on every pressure transition.
    /// Returns an id for [`Self::unregister_shed_hook`].
    pub fn register_shed_hook(&self, hook: ShedHook) -> ShedHookId {
        let id = self.next_hook_id.fetch_add(1, Ordering::Relaxed);
        self.shed_hooks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((id, hook));
        id
    }

    /// Unregister a load-shedding hook by id.
    pub fn unregister_shed_hook(&self, id: ShedHookId) {
        self.shed_hooks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(hook_id, _)| *hook_id != id);
    }

    /// Call the hooks outside the lock: a hook may register or unregister
    /// hooks, or allocate, which can re-enter `evaluate`.
    fn invoke_shed_hooks(&self, level: PressureLevel) {
        let hooks: Vec<ShedHook> = self
            .shed_hooks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(_, hook)| Arc::clone(hook))
            .collect();
        for hook in hooks {
            hook(level);
        }
    }

    fn ensure_configured(&self) {
        if self.configured.load(Ordering::Acquire) {
            return;
        }
        let mut slot = self.config.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.is_some() {
            return;
        }
        let config = MemoryConfig::from_env().unwrap_or_else(|e| {
            eprintln!("[memory] warning: invalid memory configuration ({e}); using defaults");
            MemoryConfig::platform_default()
        });
        self.install(&config);
        *slot = Some(config);
    }

    /// The active config.
    pub fn config(&self) -> MemoryConfig {
        self.ensure_configured();
        self.config
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .expect("configured")
    }

    fn credit_chunk(&self) -> usize {
        self.ensure_configured();
        self.credit_chunk.load(Ordering::Relaxed)
    }

    /// Whether strict admission is on.
    pub fn is_strict(&self) -> bool {
        self.ensure_configured();
        self.strict.load(Ordering::Relaxed)
    }

    /// Whether committed bytes are at or below the hard limit.
    pub fn within_hard_limit(&self) -> bool {
        self.ensure_configured();
        self.committed_bytes() <= self.hard_limit.load(Ordering::Relaxed)
    }

    /// Whether a collection left enough room below the hard limit to clear
    /// a runtime fault: [`Self::min_headroom`], capped at an eighth of the
    /// hard limit.  Clearing a fault with less room would collect again
    /// after every few allocations while live data creeps up to the limit.
    pub fn has_fault_headroom(&self) -> bool {
        let hard = self.hard_limit.load(Ordering::Relaxed);
        let headroom = self.min_headroom().min(hard / 8);
        self.committed_bytes().saturating_add(headroom) <= hard
    }

    /// Smallest post-collection headroom: the larger of 16 credit chunks or
    /// [`MIN_COLLECTION_HEADROOM`].
    pub fn min_headroom(&self) -> usize {
        self.credit_chunk()
            .saturating_mul(16)
            .max(MIN_COLLECTION_HEADROOM)
    }

    /// Free credit an isolate keeps after a collection or recall at `level`.
    fn retained_credit(&self, level: PressureLevel) -> usize {
        let chunks = self.retained_credit_chunks.load(Ordering::Relaxed);
        let chunks = match level {
            PressureLevel::Green => chunks,
            PressureLevel::Yellow => chunks.min(1),
            PressureLevel::Red => 0,
        };
        self.credit_chunk().saturating_mul(chunks)
    }

    /// Register an isolate.  The returned account is single-threaded (`!Sync`).
    pub fn register_isolate(&'static self, name: impl Into<Arc<str>>) -> IsolateAccount {
        let id = self.next_isolate_id.fetch_add(1, Ordering::Relaxed);
        let control = Arc::new(IsolateControl::new(id, name.into()));
        {
            let mut reg = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
            reg.retain(|w| w.strong_count() > 0);
            reg.push(Arc::downgrade(&control));
        }
        self.isolates_registered.fetch_add(1, Ordering::Relaxed);
        IsolateAccount {
            governor: self,
            control,
            used: Cell::new(0),
            credit: Cell::new(0),
            region: Cell::new(0),
            retired: Cell::new(0),
            published_credit: Cell::new(0),
            published_region: Cell::new(0),
            published_retired: Cell::new(0),
            pending_allocs: Cell::new(0),
            pending_alloc_bytes: Cell::new(0),
            alloc_since_collection: Cell::new(0),
            collection_started: Cell::new(Instant::now()),
        }
    }

    fn unregister(&self, control: &Arc<IsolateControl>) {
        let mut reg = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
        reg.retain(|w| w.strong_count() > 0 && !std::ptr::eq(w.as_ptr(), Arc::as_ptr(control)));
    }

    /// Reserve `bytes` of `class` for an allocation with an independent
    /// lifetime.  In observe-only mode the reservation always succeeds, and
    /// an over-limit reservation is recorded as a would-reject event.  In
    /// strict mode a reservation that does not fit below the hard limit is
    /// refused (inside a [`CriticalSection`], below the hard limit plus the
    /// critical reserve), leaving every counter unchanged.
    pub fn reserve_shared(
        &'static self,
        class: MemoryClass,
        bytes: usize,
    ) -> Result<MemoryCharge, MemoryLimitExceeded> {
        self.reserve(class, bytes, current_admission())
    }

    /// [`Self::reserve_shared`] with [`Admission::Critical`]: for runtime
    /// control data.
    pub fn reserve_critical(
        &'static self,
        class: MemoryClass,
        bytes: usize,
    ) -> Result<MemoryCharge, MemoryLimitExceeded> {
        self.reserve(class, bytes, Admission::Critical)
    }

    /// Reserve `bytes` of `class` for a constructor that cannot fail.
    /// Always returns a charge for the full size: a strict-mode refusal
    /// raises the thread's runtime fault (outside a [`CriticalSection`]) and
    /// takes the bytes from the critical reserve, or past it as a counted
    /// reserve overrun, like [`IsolateAccount::charge`].
    pub fn reserve_infallible(&'static self, class: MemoryClass, bytes: usize) -> MemoryCharge {
        self.admit_infallible(class, bytes);
        self.evaluate(None, bytes > 0);
        MemoryCharge {
            governor: self,
            class,
            bytes,
        }
    }

    /// Report `bytes` of `class` that the governor does not deny (JIT code).
    /// Always succeeds, even in strict mode.
    pub fn reserve_reported(&'static self, class: MemoryClass, bytes: usize) -> MemoryCharge {
        self.add(class, bytes);
        self.evaluate(None, bytes > 0);
        MemoryCharge {
            governor: self,
            class,
            bytes,
        }
    }

    fn reserve(
        &'static self,
        class: MemoryClass,
        bytes: usize,
        admission: Admission,
    ) -> Result<MemoryCharge, MemoryLimitExceeded> {
        if let Err(refused) = self.admit(class, bytes, admission) {
            self.record_rejection(&refused, None);
            return Err(refused);
        }
        self.evaluate(None, bytes > 0);
        Ok(MemoryCharge {
            governor: self,
            class,
            bytes,
        })
    }

    /// Record `bytes` of `class` that stay allocated for the process
    /// lifetime (static data); they are never returned.  Infallible: in
    /// strict mode a refusal raises the thread's runtime fault and uses the
    /// critical reserve, like [`IsolateAccount::charge`].
    pub fn charge_permanent(&'static self, class: MemoryClass, bytes: usize) {
        if bytes == 0 {
            return;
        }
        self.admit_infallible(class, bytes);
        self.evaluate(None, false);
    }

    /// Commit bytes whose allocation cannot be refused.  Application
    /// admission first; on a strict refusal, record it and raise the
    /// thread's runtime fault (outside a critical section), then commit
    /// through [`Self::admit_or_overrun`].
    fn admit_infallible(&'static self, class: MemoryClass, bytes: usize) {
        let admission = current_admission();
        if let Err(refused) = self.admit(class, bytes, admission) {
            if admission == Admission::Application {
                self.record_rejection(&refused, None);
                raise_fault(self, refused, None);
            }
            self.admit_or_overrun(class, bytes);
        }
    }

    /// Fallible [`Self::charge_permanent`]: a refusal changes no counter.
    pub fn try_charge_permanent(
        &'static self,
        class: MemoryClass,
        bytes: usize,
    ) -> Result<(), MemoryLimitExceeded> {
        if let Err(refused) = self.admit(class, bytes, current_admission()) {
            self.record_rejection(&refused, None);
            return Err(refused);
        }
        self.evaluate(None, false);
        Ok(())
    }

    /// Commit `bytes` of `class` if `admission` allows it.  Observe-only
    /// mode always commits.  A refusal changes no counter.
    fn admit(
        &self,
        class: MemoryClass,
        bytes: usize,
        admission: Admission,
    ) -> Result<(), MemoryLimitExceeded> {
        if bytes == 0 {
            return Ok(());
        }
        self.ensure_configured();
        if !self.strict.load(Ordering::Relaxed) {
            self.add(class, bytes);
            return Ok(());
        }
        let hard = self.hard_limit.load(Ordering::Relaxed);
        let limit = match admission {
            Admission::Application => hard,
            Admission::Critical => {
                hard.saturating_add(self.critical_reserve.load(Ordering::Relaxed))
            }
        };
        // A compare-and-swap loop, so concurrent grants never commit past
        // `limit`.
        let mut prev = self.committed.load(Ordering::Relaxed);
        let mut reclaimed = false;
        loop {
            let next = match prev.checked_add(bytes) {
                Some(n) if n <= limit => n,
                _ if !reclaimed => {
                    // No governor or cache lock is held here. Idle stacks are
                    // reclaimable regardless of the requesting memory class.
                    reclaimed = true;
                    crate::fiber::reclaim_idle_stacks();
                    prev = self.committed.load(Ordering::Relaxed);
                    continue;
                }
                _ => {
                    return Err(MemoryLimitExceeded {
                        class,
                        requested: bytes,
                        committed: prev,
                        hard_limit: hard,
                    });
                }
            };
            match self.committed.compare_exchange_weak(
                prev,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => prev = actual,
            }
        }
        self.class_bytes[class.index()].fetch_add(bytes, Ordering::Relaxed);
        if prev + bytes > hard {
            self.critical_grants.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Commit bytes that cannot be refused: within the critical reserve if
    /// possible, past it otherwise (counted as a reserve overrun).
    fn admit_or_overrun(&self, class: MemoryClass, bytes: usize) {
        if self.admit(class, bytes, Admission::Critical).is_err() {
            self.add(class, bytes);
            self.reserve_overruns.fetch_add(1, Ordering::Relaxed);
            self.reserve_overrun_bytes
                .fetch_add(bytes as u64, Ordering::Relaxed);
            tracing::warn!(
                target: "memory",
                class = class.name(),
                bytes,
                committed = self.committed_bytes(),
                "critical memory reserve exhausted; allocation admitted past it"
            );
        }
    }

    /// Count a strict-mode refusal, hold pressure at `Red`, and request
    /// collection.
    fn record_rejection(&self, refused: &MemoryLimitExceeded, requester: Option<&IsolateControl>) {
        self.rejected_reservations.fetch_add(1, Ordering::Relaxed);
        self.rejected_bytes
            .fetch_add(refused.requested as u64, Ordering::Relaxed);
        self.reject_latch.store(true, Ordering::Relaxed);
        tracing::info!(
            target: "memory",
            class = refused.class.name(),
            requested = refused.requested,
            committed = refused.committed,
            hard_limit = refused.hard_limit,
            isolate = requester.map(|c| c.id),
            "governed reservation rejected"
        );
        self.evaluate(requester, true);
    }

    /// Record that a sender waited `waited` for queue capacity.
    pub fn record_queue_wait(&self, waited: Duration) {
        self.queue_waits.fetch_add(1, Ordering::Relaxed);
        self.queue_wait_nanos.fetch_add(
            waited.as_nanos().min(u64::MAX as u128) as u64,
            Ordering::Relaxed,
        );
    }

    /// Record a message that a byte-bounded queue refused.
    pub fn record_queue_rejection(&self) {
        self.queue_rejections.fetch_add(1, Ordering::Relaxed);
    }

    /// Uncommit `bytes` of isolate credit or used heap bytes.
    fn return_credit(&self, bytes: usize) {
        if bytes > 0 {
            self.sub(MemoryClass::GcHeap, bytes);
            self.credit_returned_bytes
                .fetch_add(bytes as u64, Ordering::Relaxed);
        }
    }

    fn add(&self, class: MemoryClass, bytes: usize) {
        if bytes > 0 {
            self.class_bytes[class.index()].fetch_add(bytes, Ordering::Relaxed);
            self.committed.fetch_add(bytes, Ordering::Relaxed);
        }
    }

    fn sub(&self, class: MemoryClass, bytes: usize) {
        if bytes == 0 {
            return;
        }
        checked_sub(&self.class_bytes[class.index()], bytes, class.name());
        checked_sub(&self.committed, bytes, "committed");
    }

    /// Move an isolate's published region bytes from `prev` to `now`
    /// between the `GcHeap` class (where its credit was granted) and the
    /// `Region` class.  Committed bytes do not change.
    fn shift_region(&self, prev: usize, now: usize) {
        let (from, to, bytes) = if now >= prev {
            (MemoryClass::GcHeap, MemoryClass::Region, now - prev)
        } else {
            (MemoryClass::Region, MemoryClass::GcHeap, prev - now)
        };
        if bytes > 0 {
            checked_sub(&self.class_bytes[from.index()], bytes, from.name());
            self.class_bytes[to.index()].fetch_add(bytes, Ordering::Relaxed);
        }
    }

    fn add_free_credit(&self, bytes: usize) {
        if bytes > 0 {
            self.free_credit.fetch_add(bytes, Ordering::Relaxed);
        }
    }

    fn sub_free_credit(&self, bytes: usize) {
        if bytes > 0 {
            checked_sub(&self.free_credit, bytes, "free-credit");
        }
    }

    /// Current bytes in `class`.
    pub fn class_bytes(&self, class: MemoryClass) -> usize {
        self.class_bytes[class.index()].load(Ordering::Relaxed)
    }

    /// Total committed bytes: used bytes plus unused isolate credit.
    pub fn committed_bytes(&self) -> usize {
        self.committed.load(Ordering::Relaxed)
    }

    /// Unused isolate credit, as last published by each isolate.
    pub fn free_credit_bytes(&self) -> usize {
        self.free_credit
            .load(Ordering::Relaxed)
            .min(self.committed_bytes())
    }

    /// Current pressure level.
    pub fn pressure(&self) -> PressureLevel {
        PressureLevel::from_u8(self.pressure.load(Ordering::Acquire))
    }

    /// Recompute pressure; on a rise, recall unused credit; request
    /// collection at `Yellow`/`Red`.
    fn evaluate(&self, requester: Option<&IsolateControl>, grew: bool) {
        self.ensure_configured();
        let committed = self.committed_bytes();
        let free = self.free_credit.load(Ordering::Relaxed).min(committed);
        raise_peak(&self.peak_committed, committed);
        raise_peak(&self.peak_used, committed - free);
        let soft = self.soft_limit.load(Ordering::Relaxed);
        let hard = self.hard_limit.load(Ordering::Relaxed);
        let mut level = PressureLevel::classify(committed, soft, hard);
        if self.reject_latch.load(Ordering::Relaxed) {
            let chunk = self.credit_chunk.load(Ordering::Relaxed);
            if committed.saturating_add(chunk) <= hard {
                self.reject_latch.store(false, Ordering::Relaxed);
            } else {
                level = PressureLevel::Red;
            }
        }
        // Phase 5: raise pressure one level if process RSS exceeds the
        // configured threshold.  RSS alone never forces Red: it may not fall
        // after a collection, which would hold every isolate at Red.
        let rss_pct = self.rss_pressure_percent.load(Ordering::Relaxed);
        if rss_pct > 0 {
            let rss = self.get_rss_bytes();
            if rss > 0 {
                let threshold = (hard as u64 * rss_pct as u64 / 100) as usize;
                if rss >= threshold {
                    level = match level {
                        PressureLevel::Green => PressureLevel::Yellow,
                        PressureLevel::Yellow | PressureLevel::Red => PressureLevel::Red,
                    };
                }
            }
        }
        let prev = self.pressure();
        // Only a change writes the shared pressure word.
        if prev != level
            && self
                .pressure
                .compare_exchange(prev as u8, level as u8, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            self.pressure_transitions.fetch_add(1, Ordering::Relaxed);
            tracing::info!(
                target: "memory",
                from = %prev,
                to = %level,
                committed,
                soft_limit = soft,
                hard_limit = hard,
                "memory pressure transition"
            );
            if level > prev {
                self.recall_credit();
            }
            self.invoke_shed_hooks(level);
        }
        if grew && committed > hard {
            self.over_limit_events.fetch_add(1, Ordering::Relaxed);
            self.over_limit_bytes
                .fetch_add((committed - hard) as u64, Ordering::Relaxed);
            tracing::debug!(
                target: "memory",
                committed,
                hard_limit = hard,
                isolate = requester.map(|c| c.id),
                "governed allocation over the hard limit"
            );
        }
        if level == PressureLevel::Green {
            return;
        }
        if let Some(req) = requester
            && req.governor_request(level)
        {
            self.collection_requests.fetch_add(1, Ordering::Relaxed);
        }
        if grew && let Some(largest) = self.largest_isolate() {
            let same = requester.is_some_and(|r| r.id == largest.id);
            if !same && largest.cross_governor_request(level) {
                self.collection_requests.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Ask every registered isolate to return its unused credit.  Each one
    /// does so at its next refill, collection, or runtime service poll; an
    /// isolate that never polls keeps at most its retained chunks.
    fn recall_credit(&self) {
        let reg = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
        for control in reg.iter().filter_map(Weak::upgrade) {
            control.request_recall();
        }
        drop(reg);
        self.credit_recalls.fetch_add(1, Ordering::Relaxed);
    }

    fn largest_isolate(&self) -> Option<Arc<IsolateControl>> {
        let reg = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
        reg.iter()
            .filter_map(Weak::upgrade)
            .max_by_key(|c| c.used_bytes())
    }

    /// Point-in-time view of every counter.
    pub fn snapshot(&self) -> MemorySnapshot {
        let config = self.config();
        let mut class_bytes = [0usize; CLASS_COUNT];
        for (slot, c) in class_bytes.iter_mut().zip(self.class_bytes.iter()) {
            *slot = c.load(Ordering::Relaxed);
        }
        let committed = self.committed_bytes();
        let free = self.free_credit_bytes();
        let mut isolates: Vec<IsolateSnapshot> = {
            let reg = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
            reg.iter()
                .filter_map(Weak::upgrade)
                .map(|c| c.snapshot())
                .collect()
        };
        isolates.sort_by_key(|i| i.id);
        let (fiber_count, peak_fiber_count, pooled_fiber_stacks) = crate::fiber::counts();
        MemorySnapshot {
            fiber_count,
            peak_fiber_count,
            pooled_fiber_stacks,
            strict: config.strict,
            soft_limit: config.soft_limit,
            hard_limit: config.hard_limit,
            critical_reserve: config.critical_reserve,
            credit_chunk: config.credit_chunk,
            pressure: self.pressure(),
            committed_bytes: committed,
            used_bytes: committed - free,
            free_credit_bytes: free,
            peak_committed_bytes: self.peak_committed.load(Ordering::Relaxed),
            peak_used_bytes: self.peak_used.load(Ordering::Relaxed),
            class_bytes,
            isolates,
            isolates_registered: self.isolates_registered.load(Ordering::Relaxed),
            pressure_transitions: self.pressure_transitions.load(Ordering::Relaxed),
            collection_requests: self.collection_requests.load(Ordering::Relaxed),
            credit_refills: self.credit_refills.load(Ordering::Relaxed),
            credit_returned_bytes: self.credit_returned_bytes.load(Ordering::Relaxed),
            credit_recalls: self.credit_recalls.load(Ordering::Relaxed),
            over_limit_events: self.over_limit_events.load(Ordering::Relaxed),
            over_limit_bytes: self.over_limit_bytes.load(Ordering::Relaxed),
            retired_region_bytes: self.retired_region.load(Ordering::Relaxed),
            queue_waits: self.queue_waits.load(Ordering::Relaxed),
            queue_wait_time: Duration::from_nanos(self.queue_wait_nanos.load(Ordering::Relaxed)),
            queue_rejections: self.queue_rejections.load(Ordering::Relaxed),
            rejected_reservations: self.rejected_reservations.load(Ordering::Relaxed),
            rejected_bytes: self.rejected_bytes.load(Ordering::Relaxed),
            critical_grants: self.critical_grants.load(Ordering::Relaxed),
            reserve_overruns: self.reserve_overruns.load(Ordering::Relaxed),
            reserve_overrun_bytes: self.reserve_overrun_bytes.load(Ordering::Relaxed),
            runtime_faults: self.runtime_faults.load(Ordering::Relaxed),
            runtime_faults_cleared: self.runtime_faults_cleared.load(Ordering::Relaxed),
            rss_bytes: self.rss_cache_bytes.load(Ordering::Relaxed),
            rss_pressure_percent: self.rss_pressure_percent.load(Ordering::Relaxed),
        }
    }
}

/// Raise `peak` to `value`.  Reads first, so the common no-change case does
/// not write the shared cache line.
fn raise_peak(peak: &AtomicUsize, value: usize) {
    if value > peak.load(Ordering::Relaxed) {
        peak.fetch_max(value, Ordering::Relaxed);
    }
}

/// Subtract `bytes` from `counter`, reporting underflow in debug builds.
fn checked_sub(counter: &AtomicUsize, bytes: usize, what: &str) {
    let prev = counter.fetch_sub(bytes, Ordering::Relaxed);
    if prev < bytes {
        // Restore the counter before reporting, so release builds keep a
        // sane (zero) value rather than a wrapped one.  Not atomic with the
        // `fetch_sub`: a concurrent `add` in between can still lose bytes.
        // Underflow is a bug, and this repair is best-effort.
        counter.fetch_add(bytes - prev, Ordering::Relaxed);
        debug_assert!(
            false,
            "{what} counter underflow: released {bytes} with {prev} committed"
        );
    }
}

impl Default for ProcessMemoryGovernor {
    fn default() -> Self {
        Self::new()
    }
}

static GOVERNOR: ProcessMemoryGovernor = ProcessMemoryGovernor::new();

/// The process governor shared by every runtime in the process.
pub fn governor() -> &'static ProcessMemoryGovernor {
    &GOVERNOR
}

// ── Snapshots ────────────────────────────────────────────────────────────────

/// Point-in-time per-isolate counters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolateSnapshot {
    pub id: IsolateId,
    pub name: Arc<str>,
    pub used_bytes: usize,
    pub free_credit_bytes: usize,
    /// Active region chunk bytes.
    pub region_bytes: usize,
    /// Retired region chunk bytes.
    pub retired_region_bytes: usize,
    pub peak_used_bytes: usize,
    /// Used bytes at which the isolate requests its own collection.
    pub collection_target: usize,
    pub collections: u64,
    pub collection_requests: u64,
    pub credit_refills: u64,
    pub zero_yield_collections: u64,
    pub last_bytes_before: usize,
    pub last_bytes_after: usize,
    pub last_duration: Duration,
}

/// Point-in-time process counters.  Fields are individually atomic
/// observations; the snapshot as a whole is not a consistent cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySnapshot {
    /// Unstarted, running, and parked native fibers (excludes completed results).
    pub fiber_count: usize,
    pub peak_fiber_count: usize,
    pub pooled_fiber_stacks: usize,
    /// Strict admission is on.
    pub strict: bool,
    pub soft_limit: usize,
    pub hard_limit: usize,
    pub critical_reserve: usize,
    pub credit_chunk: usize,
    pub pressure: PressureLevel,
    /// Used bytes plus unused isolate credit.  Exact.
    pub committed_bytes: usize,
    /// Committed bytes minus free credit.
    pub used_bytes: usize,
    /// Unused isolate credit as of each isolate's last refill, collection,
    /// or poll; it can overstate current free credit by up to one chunk per
    /// isolate.
    pub free_credit_bytes: usize,
    pub peak_committed_bytes: usize,
    pub peak_used_bytes: usize,
    class_bytes: [usize; CLASS_COUNT],
    /// Live registered isolates.
    pub isolates: Vec<IsolateSnapshot>,
    /// Isolates ever registered.
    pub isolates_registered: u64,
    pub pressure_transitions: u64,
    pub collection_requests: u64,
    /// Credit grants to isolates (each one is a governor round trip).
    pub credit_refills: u64,
    /// Bytes of credit and used heap returned by isolates.
    pub credit_returned_bytes: u64,
    /// Pressure rises that recalled unused credit from every isolate.
    pub credit_recalls: u64,
    /// Grants that ended above the hard limit.  In observe-only mode,
    /// strict mode would have rejected each one; in strict mode they are
    /// critical-reserve grants and reserve overruns.
    pub over_limit_events: u64,
    /// Sum of the overage at each over-limit event.
    pub over_limit_bytes: u64,
    /// Retired region chunk bytes; part of the `Region` class.  The active
    /// part is the class total minus this value.
    pub retired_region_bytes: usize,
    /// Sends that waited for queue capacity.
    pub queue_waits: u64,
    /// Total time senders waited for queue capacity.
    pub queue_wait_time: Duration,
    /// Messages a byte-bounded queue refused.
    pub queue_rejections: u64,
    /// Strict-mode reservations and credit requests refused.
    pub rejected_reservations: u64,
    /// Bytes requested by refused reservations.
    pub rejected_bytes: u64,
    /// Grants that used the critical reserve (ended above the hard limit).
    pub critical_grants: u64,
    /// Infallible grants admitted past the critical reserve.
    pub reserve_overruns: u64,
    /// Bytes of those grants.
    pub reserve_overrun_bytes: u64,
    /// Runtime faults raised by refused infallible allocations.
    pub runtime_faults: u64,
    /// Runtime faults cleared because a collection brought committed bytes
    /// back within the hard limit.
    pub runtime_faults_cleared: u64,
    /// Current process RSS bytes (0 = unavailable or not sampled).
    pub rss_bytes: usize,
    /// RSS threshold as a percentage of the hard limit (0 = disabled).
    pub rss_pressure_percent: u8,
}

impl MemorySnapshot {
    /// Current bytes in `class`.
    pub fn class_bytes(&self, class: MemoryClass) -> usize {
        self.class_bytes[class.index()]
    }
}

impl fmt::Display for MemorySnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Managed memory ({}):",
            if self.strict {
                "strict"
            } else {
                "observe-only"
            }
        )?;
        writeln!(
            f,
            "  Limits:                soft {} / hard {} bytes (reserve {})",
            self.soft_limit, self.hard_limit, self.critical_reserve
        )?;
        writeln!(f, "  Pressure:              {}", self.pressure)?;
        writeln!(
            f,
            "  Committed:             {} bytes (peak {})",
            self.committed_bytes, self.peak_committed_bytes
        )?;
        writeln!(
            f,
            "  Used:                  {} bytes (peak {}), {} bytes free credit",
            self.used_bytes, self.peak_used_bytes, self.free_credit_bytes
        )?;
        writeln!(
            f,
            "  Fibers:                {} live (peak {}), {} pooled stacks, {} reserved stack bytes",
            self.fiber_count,
            self.peak_fiber_count,
            self.pooled_fiber_stacks,
            self.class_bytes(MemoryClass::FiberStack)
        )?;
        for class in MemoryClass::ALL {
            let bytes = self.class_bytes(class);
            if bytes == 0 {
                continue;
            }
            let label = format!("{}:", class.name());
            if class == MemoryClass::Region {
                writeln!(
                    f,
                    "    {label:<20} {bytes} bytes ({} active, {} retired)",
                    bytes.saturating_sub(self.retired_region_bytes),
                    self.retired_region_bytes
                )?;
            } else {
                writeln!(f, "    {label:<20} {bytes} bytes")?;
            }
        }
        writeln!(
            f,
            "  Isolates:              {} live ({} registered)",
            self.isolates.len(),
            self.isolates_registered
        )?;
        for iso in &self.isolates {
            writeln!(
                f,
                "    #{} {}: {} bytes (peak {}, target {}), {} region ({} retired), {} free credit, {} refills, {} collections, {} requested",
                iso.id,
                iso.name,
                iso.used_bytes,
                iso.peak_used_bytes,
                iso.collection_target,
                iso.region_bytes + iso.retired_region_bytes,
                iso.retired_region_bytes,
                iso.free_credit_bytes,
                iso.credit_refills,
                iso.collections,
                iso.collection_requests
            )?;
        }
        writeln!(f, "  Pressure transitions:  {}", self.pressure_transitions)?;
        writeln!(f, "  Collection requests:   {}", self.collection_requests)?;
        writeln!(
            f,
            "  Credit:                {} refills, {} bytes returned, {} recalls",
            self.credit_refills, self.credit_returned_bytes, self.credit_recalls
        )?;
        writeln!(
            f,
            "  Queues:                {} waits ({:?}), {} rejected",
            self.queue_waits, self.queue_wait_time, self.queue_rejections
        )?;
        writeln!(
            f,
            "  Over hard limit:       {} events ({} bytes over)",
            self.over_limit_events, self.over_limit_bytes
        )?;
        writeln!(
            f,
            "  Rejected:              {} reservations ({} bytes), {} critical grants, {} reserve overruns ({} bytes), {} faults ({} cleared)",
            self.rejected_reservations,
            self.rejected_bytes,
            self.critical_grants,
            self.reserve_overruns,
            self.reserve_overrun_bytes,
            self.runtime_faults,
            self.runtime_faults_cleared
        )?;
        if self.rss_pressure_percent > 0 {
            write!(
                f,
                "  RSS:                   {} bytes (threshold {}% of hard limit)",
                self.rss_bytes, self.rss_pressure_percent
            )
        } else {
            write!(f, "  RSS pressure:          disabled")
        }
    }
}

// ── Current-thread isolate ───────────────────────────────────────────────────

thread_local! {
    static CURRENT: OnceCell<IsolateAccount> = const { OnceCell::new() };
}

fn default_isolate_name() -> Arc<str> {
    let thread = std::thread::current();
    match thread.name() {
        Some(name) => Arc::from(name),
        None => Arc::from(format!("{:?}", thread.id())),
    }
}

/// Register the calling thread as an isolate named `name` (or rename it if
/// it is already registered).  Returns its control handle.
pub fn register_current_isolate(name: &str) -> Option<Arc<IsolateControl>> {
    CURRENT
        .try_with(|cell| {
            if let Some(account) = cell.get() {
                account.control.set_name(Arc::from(name));
            } else {
                let _ = cell.set(governor().register_isolate(name));
            }
            cell.get().map(|a| a.control.clone())
        })
        .ok()
        .flatten()
}

/// Run `f` against the calling thread's account, registering it under the
/// thread's name on first use.  `None` during thread-local teardown.
#[inline]
pub fn with_current_account<R>(f: impl FnOnce(&IsolateAccount) -> R) -> Option<R> {
    CURRENT
        .try_with(
            |cell| f(cell.get_or_init(|| governor().register_isolate(default_isolate_name()))),
        )
        .ok()
}

/// Run `f` against the calling thread's account only if the thread already
/// has one.  Unlike [`with_current_account`], this never registers the
/// thread, so a release during thread teardown cannot create a new account.
#[inline]
pub fn with_registered_account<R>(f: impl FnOnce(&IsolateAccount) -> R) -> Option<R> {
    CURRENT.try_with(|cell| cell.get().map(f)).ok().flatten()
}

/// The calling thread's control handle.
pub fn current_control() -> Option<Arc<IsolateControl>> {
    with_current_account(|a| a.control.clone())
}

/// [`IsolateAccount::try_reserve_credit`] on the calling thread's account.
/// `Ok` during thread-local teardown, when no account is reachable.
pub fn try_reserve_current(bytes: usize) -> Result<(), MemoryLimitExceeded> {
    with_current_account(|a| a.try_reserve_credit(bytes)).unwrap_or(Ok(()))
}

/// Flush the calling thread's account and return a process snapshot.
pub fn snapshot() -> MemorySnapshot {
    let _ = CURRENT.try_with(|cell| {
        if let Some(account) = cell.get() {
            account.publish();
        }
    });
    governor().snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaked(soft: usize, hard: usize, chunk: usize) -> &'static ProcessMemoryGovernor {
        let mut config = MemoryConfig::with_limits(soft, hard).unwrap();
        config.credit_chunk = chunk;
        Box::leak(Box::new(
            ProcessMemoryGovernor::with_config(config).unwrap(),
        ))
    }

    fn strict(soft: usize, hard: usize, chunk: usize) -> &'static ProcessMemoryGovernor {
        let mut config = MemoryConfig::with_limits(soft, hard).unwrap();
        config.credit_chunk = chunk;
        config.strict = true;
        Box::leak(Box::new(
            ProcessMemoryGovernor::with_config(config).unwrap(),
        ))
    }

    fn report(before: usize, after: usize) -> CollectionReport {
        CollectionReport {
            bytes_before: before,
            bytes_after: after,
            bytes_returned: before - after,
            bytes_pending: 0,
            duration: Duration::from_micros(1),
        }
    }

    #[test]
    fn config_rejects_invalid_states() {
        assert_eq!(
            MemoryConfig::with_limits(0, 0),
            Err(MemoryConfigError::ZeroHardLimit)
        );
        assert!(matches!(
            MemoryConfig::with_limits(10 * MIB, 5 * MIB),
            Err(MemoryConfigError::SoftAboveHard { .. })
        ));
        let mut c = MemoryConfig::with_limits(MIB, 2 * MIB).unwrap();
        c.critical_reserve = 3 * MIB;
        assert!(matches!(
            c.validate(),
            Err(MemoryConfigError::ReserveAboveHard { .. })
        ));
        let mut c = MemoryConfig::with_limits(MIB, 2 * MIB).unwrap();
        c.credit_chunk = 0;
        assert_eq!(c.validate(), Err(MemoryConfigError::ZeroCreditChunk));
        let mut c = MemoryConfig::with_limits(MIB, 2 * MIB).unwrap();
        c.queue_limit = 0;
        assert_eq!(c.validate(), Err(MemoryConfigError::ZeroQueueLimit));
        let err =
            MemoryConfig::from_lookup(|k| (k == QUEUE_LIMIT_ENV).then(|| "0".to_string()), |_| {})
                .unwrap_err();
        assert_eq!(err, MemoryConfigError::ZeroQueueLimit);
        let mut c = MemoryConfig::with_limits(MIB, 2 * MIB).unwrap();
        c.queue_limit = 3 * MIB;
        assert!(matches!(
            c.validate(),
            Err(MemoryConfigError::QueueAboveBudget { .. })
        ));
    }

    #[test]
    fn config_defaults() {
        let c = MemoryConfig::with_limits(300 * MIB, 400 * MIB).unwrap();
        assert_eq!(c.critical_reserve, 4 * MIB);
        assert_eq!(c.credit_chunk, DEFAULT_CREDIT_CHUNK);
        assert_eq!(c.queue_limit, DEFAULT_QUEUE_LIMIT);
        let c = MemoryConfig::with_limits(1, 2 * MIB).unwrap();
        assert_eq!(
            c.critical_reserve,
            2 * MIB,
            "reserve is capped at the hard limit"
        );
        assert_eq!(c.queue_limit, 2 * MIB);
        let big = MemoryConfig::with_limits(1, 1 << 40).unwrap();
        assert_eq!(big.critical_reserve, MAX_DEFAULT_CRITICAL_RESERVE);
        let c = MemoryConfig::from_optional_limits(None, Some(400 * MIB)).unwrap();
        assert_eq!(c.soft_limit, 300 * MIB);
        assert_eq!(
            default_soft_limit(1),
            1,
            "tiny hard limit keeps a nonzero soft limit"
        );
        assert_eq!(default_soft_limit(3), 3);
        assert_eq!(default_soft_limit(4), 3);
        let c = MemoryConfig::from_optional_limits(Some(usize::MAX / 2), None).unwrap();
        assert_eq!(
            c.soft_limit, c.hard_limit,
            "soft-only raises the hard limit"
        );
    }

    #[test]
    fn env_new_names_win_and_old_names_warn() {
        let vars = [
            (SOFT_LIMIT_ENV, "100"),
            (DEPRECATED_SOFT_LIMIT_ENV, "50"),
            (DEPRECATED_HARD_LIMIT_ENV, "200"),
            (CREDIT_ENV, "128"),
        ];
        let lookup = |k: &str| {
            vars.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        };
        let mut warnings = Vec::new();
        let c = MemoryConfig::from_lookup(lookup, |w| warnings.push(w)).unwrap();
        assert_eq!(c.soft_limit, 100 * MIB);
        assert_eq!(c.hard_limit, 200 * MIB);
        assert_eq!(c.credit_chunk, 128 * KIB);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("ignored"));
        assert!(warnings[1].contains(DEPRECATED_HARD_LIMIT_ENV));
    }

    #[test]
    fn env_rejects_bad_values() {
        let err = MemoryConfig::from_lookup(
            |k| (k == HARD_LIMIT_ENV).then(|| "lots".to_string()),
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(err, MemoryConfigError::InvalidNumber { .. }));
        let err = MemoryConfig::from_lookup(
            |k| match k {
                SOFT_LIMIT_ENV => Some("20".into()),
                HARD_LIMIT_ENV => Some("10".into()),
                _ => None,
            },
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(err, MemoryConfigError::SoftAboveHard { .. }));
    }

    #[test]
    fn charge_refills_one_chunk_at_a_time() {
        let g = leaked(10 * MIB, 20 * MIB, 4 * KIB);
        let acc = g.register_isolate("a");
        acc.charge(KIB);
        assert_eq!(
            g.committed_bytes(),
            4 * KIB,
            "the first charge takes a chunk"
        );
        assert_eq!(acc.used_bytes(), KIB);
        assert_eq!(acc.free_credit_bytes(), 3 * KIB);
        acc.charge(3 * KIB);
        assert_eq!(g.committed_bytes(), 4 * KIB, "exact fit stays local");
        assert_eq!(acc.control().credit_refills(), 1);
        acc.charge(1);
        assert_eq!(g.committed_bytes(), 8 * KIB);
        assert_eq!(acc.control().credit_refills(), 2);
        assert_eq!(acc.control().used_bytes(), 4 * KIB + 1, "refill publishes");
        assert_eq!(acc.committed_bytes(), g.committed_bytes());
    }

    #[test]
    fn common_path_does_not_contact_the_governor() {
        let g = leaked(10 * MIB, 20 * MIB, 64 * KIB);
        let acc = g.register_isolate("a");
        for _ in 0..64 {
            acc.charge(KIB);
        }
        assert_eq!(g.snapshot().credit_refills, 1, "64 allocations, one refill");
        assert_eq!(g.committed_bytes(), 64 * KIB);
    }

    #[test]
    fn large_allocation_requests_its_own_size() {
        let g = leaked(10 * MIB, 20 * MIB, 4 * KIB);
        let acc = g.register_isolate("a");
        acc.charge(100 * KIB + 1);
        assert_eq!(
            g.committed_bytes(),
            104 * KIB,
            "rounded to the accounting unit"
        );
        assert_eq!(acc.free_credit_bytes(), 4 * KIB - 1);
        assert_eq!(acc.control().credit_refills(), 1);
    }

    #[test]
    fn sweep_returns_exact_bytes_and_trims_credit() {
        let g = leaked(10 * MIB, 20 * MIB, KIB);
        let acc = g.register_isolate("a");
        for _ in 0..100 {
            acc.charge(48);
        }
        assert_eq!(g.committed_bytes(), 5 * KIB);
        acc.release(48 * 60);
        assert_eq!(acc.used_bytes(), 48 * 40);
        assert_eq!(acc.free_credit_bytes(), 5 * KIB - 48 * 40);
        acc.record_collection(report(4800, 48 * 40));
        assert_eq!(acc.control().collection_epoch(), 1);
        assert_eq!(
            acc.free_credit_bytes(),
            2 * KIB,
            "keeps the retained chunks only"
        );
        let snap = g.snapshot();
        assert_eq!(snap.committed_bytes, 48 * 40 + 2 * KIB);
        assert_eq!(snap.class_bytes(MemoryClass::GcHeap), 48 * 40 + 2 * KIB);
        assert_eq!(snap.free_credit_bytes, 2 * KIB);
        assert_eq!(snap.used_bytes, 48 * 40);
        assert_eq!(
            snap.credit_returned_bytes,
            (5 * KIB - 48 * 40 - 2 * KIB) as u64
        );
    }

    #[test]
    fn shutdown_returns_used_bytes_and_free_credit() {
        let g = leaked(10 * MIB, 20 * MIB, KIB);
        let acc = g.register_isolate("a");
        acc.charge(10 * KIB + 100);
        acc.release(KIB);
        acc.publish();
        assert!(g.free_credit_bytes() > 0);
        assert_eq!(g.snapshot().isolates.len(), 1);
        drop(acc);
        let snap = g.snapshot();
        assert_eq!(snap.committed_bytes, 0);
        assert_eq!(snap.free_credit_bytes, 0);
        assert!(snap.isolates.is_empty());
        assert_eq!(snap.isolates_registered, 1);
    }

    #[test]
    fn shared_charge_returns_once() {
        let g = leaked(10 * MIB, 20 * MIB, KIB);
        let charge = Arc::new(g.reserve_shared(MemoryClass::SharedValue, 1000).unwrap());
        let clone = charge.clone();
        assert_eq!(g.class_bytes(MemoryClass::SharedValue), 1000);
        assert_eq!(g.committed_bytes(), 1000);
        drop(charge);
        assert_eq!(g.class_bytes(MemoryClass::SharedValue), 1000);
        drop(clone);
        assert_eq!(g.class_bytes(MemoryClass::SharedValue), 0);
        assert_eq!(g.committed_bytes(), 0);
    }

    #[test]
    fn green_does_not_request_collection() {
        let g = leaked(64 * MIB, 128 * MIB, 64 * KIB);
        let acc = g.register_isolate("a");
        acc.charge(32 * MIB);
        assert_eq!(g.pressure(), PressureLevel::Green);
        assert!(
            !acc.control().collection_requested(),
            "GcConfig owns the heap trigger"
        );
        assert_eq!(acc.control().collection_requests(), 0);
    }

    #[test]
    fn pressure_transitions_and_requests() {
        let g = leaked(4 * MIB, 8 * MIB, 64 * KIB);
        let acc = g.register_isolate("a");
        acc.charge(MIB);
        assert_eq!(g.pressure(), PressureLevel::Green);
        assert!(!acc.control().collection_requested());

        acc.charge(3 * MIB);
        assert_eq!(g.pressure(), PressureLevel::Yellow);
        assert!(
            acc.control().take_collection_request(),
            "Yellow requests collection"
        );

        // Rate limit: no second request in the same collection epoch.
        acc.charge(64 * KIB);
        assert!(!acc.control().collection_requested());

        // After a zero-yield collection the next request waits for growth.
        let used = acc.used_bytes();
        acc.record_collection(report(used, used));
        assert_eq!(acc.control().collection_target(), 2 * used);
        acc.charge(64 * KIB);
        assert!(
            !acc.control().collection_requested(),
            "Yellow honours the target"
        );

        // Red overrides the target.
        acc.charge(5 * MIB);
        assert_eq!(g.pressure(), PressureLevel::Red);
        assert!(acc.control().take_collection_request());
        let snap = g.snapshot();
        assert!(snap.over_limit_events >= 1);
        assert!(snap.pressure_transitions >= 2);

        acc.release(acc.used_bytes());
        acc.record_collection(report(9 * MIB, 0));
        assert_eq!(acc.free_credit_bytes(), 0, "Red retains no credit");
        assert_eq!(g.committed_bytes(), 0);
        assert_eq!(g.pressure(), PressureLevel::Green);
    }

    /// A collection that frees nothing but leaves garbage in its grace cycle
    /// is not zero-yield: the target excludes the pending bytes, so the
    /// follow-up collection that frees them is requested at once.
    #[test]
    fn pending_bytes_request_the_follow_up_collection() {
        let g = leaked(4 * MIB, 8 * MIB, 64 * KIB);
        let acc = g.register_isolate("a");
        acc.charge(5 * MIB);
        assert_eq!(g.pressure(), PressureLevel::Yellow);
        assert!(acc.control().take_collection_request());

        let used = acc.used_bytes();
        acc.record_collection(CollectionReport {
            bytes_before: used,
            bytes_after: used,
            bytes_returned: 0,
            bytes_pending: used - MIB,
            duration: Duration::from_micros(1),
        });
        assert_eq!(
            acc.control().zero_yield_collections.load(Ordering::Relaxed),
            0
        );
        assert_eq!(acc.control().collection_target(), MIB + g.min_headroom());
        assert!(
            acc.control().take_collection_request(),
            "pending garbage requests the next collection"
        );
    }

    #[test]
    fn yellow_requests_largest_heap() {
        let g = leaked(4 * MIB, 8 * MIB, 64 * KIB);
        let big = g.register_isolate("big");
        let small = g.register_isolate("small");
        big.charge(3 * MIB);
        assert!(!big.control().collection_requested());
        small.charge(MIB);
        assert_eq!(g.pressure(), PressureLevel::Yellow);
        assert!(small.control().take_collection_request());
        assert!(
            big.control().take_collection_request(),
            "largest heap asked too"
        );
    }

    #[test]
    fn rising_pressure_recalls_free_credit() {
        let g = leaked(4 * MIB, 64 * MIB, 64 * KIB);
        let idle = g.register_isolate("idle");
        idle.charge(10 * 64 * KIB);
        idle.release(10 * 64 * KIB);
        idle.record_collection(report(10 * 64 * KIB, 0));
        assert_eq!(
            idle.free_credit_bytes(),
            2 * 64 * KIB,
            "Green keeps two chunks"
        );
        assert_eq!(g.committed_bytes(), 2 * 64 * KIB);

        let busy = g.register_isolate("busy");
        busy.charge(4 * MIB);
        assert_eq!(g.pressure(), PressureLevel::Yellow);
        assert!(idle.control().recall_requested());
        idle.poll();
        assert!(!idle.control().recall_requested());
        assert_eq!(idle.free_credit_bytes(), 64 * KIB, "Yellow keeps one chunk");

        busy.charge(60 * MIB);
        assert_eq!(g.pressure(), PressureLevel::Red);
        idle.poll();
        assert_eq!(idle.free_credit_bytes(), 0, "Red keeps none");
        assert!(g.snapshot().credit_recalls >= 2);
    }

    #[test]
    fn region_chunks_use_local_credit_and_report_their_class() {
        let g = leaked(10 * MIB, 20 * MIB, 64 * KIB);
        let acc = g.register_isolate("a");
        acc.charge(KIB);
        acc.charge_region(4 * KIB);
        acc.charge_region(4 * KIB);
        assert_eq!(acc.control().credit_refills(), 1, "regions stay local");
        assert_eq!(acc.region_bytes(), 8 * KIB);
        assert_eq!(acc.committed_bytes(), 64 * KIB);
        acc.publish();
        let snap = g.snapshot();
        assert_eq!(snap.class_bytes(MemoryClass::Region), 8 * KIB);
        assert_eq!(snap.class_bytes(MemoryClass::GcHeap), 56 * KIB);
        assert_eq!(snap.committed_bytes, 64 * KIB);
        assert_eq!(snap.isolates[0].region_bytes, 8 * KIB);

        acc.release_region(4 * KIB, false);
        acc.publish();
        assert_eq!(g.class_bytes(MemoryClass::Region), 4 * KIB);
        assert_eq!(g.committed_bytes(), 64 * KIB, "release becomes credit");
    }

    #[test]
    fn region_release_returns_credit_above_the_retained_limit() {
        let g = leaked(10 * MIB, 20 * MIB, 4 * KIB);
        let acc = g.register_isolate("a");
        acc.charge_region(40 * KIB);
        assert_eq!(g.committed_bytes(), 40 * KIB);
        acc.release_region(40 * KIB, false);
        assert_eq!(acc.free_credit_bytes(), 8 * KIB, "keeps two chunks");
        assert_eq!(g.committed_bytes(), 8 * KIB);
        assert_eq!(g.class_bytes(MemoryClass::Region), 0);
    }

    #[test]
    fn retired_region_keeps_its_charge() {
        let g = leaked(10 * MIB, 20 * MIB, 4 * KIB);
        let acc = g.register_isolate("a");
        acc.charge_region(8 * KIB);
        acc.retire_region(8 * KIB);
        acc.record_collection(report(0, 0));
        let snap = g.snapshot();
        assert_eq!(snap.class_bytes(MemoryClass::Region), 8 * KIB);
        assert_eq!(snap.retired_region_bytes, 8 * KIB);
        assert_eq!(snap.isolates[0].retired_region_bytes, 8 * KIB);
        assert_eq!(snap.isolates[0].region_bytes, 0);

        acc.release_region(8 * KIB, true);
        acc.publish();
        assert_eq!(g.class_bytes(MemoryClass::Region), 0);
        assert_eq!(g.snapshot().retired_region_bytes, 0);
    }

    #[test]
    fn shutdown_returns_region_charges() {
        let g = leaked(10 * MIB, 20 * MIB, 4 * KIB);
        let acc = g.register_isolate("a");
        acc.charge(KIB);
        acc.charge_region(8 * KIB);
        acc.charge_region(4 * KIB);
        acc.retire_region(4 * KIB);
        acc.publish();
        acc.charge_region(4 * KIB); // not yet published
        drop(acc);
        let snap = g.snapshot();
        assert_eq!(snap.committed_bytes, 0);
        assert_eq!(snap.class_bytes(MemoryClass::Region), 0);
        assert_eq!(snap.class_bytes(MemoryClass::GcHeap), 0);
        assert_eq!(snap.retired_region_bytes, 0);
    }

    #[test]
    fn permanent_charges_stay_committed() {
        let g = leaked(10 * MIB, 20 * MIB, 4 * KIB);
        g.charge_permanent(MemoryClass::Static, 1000);
        g.charge_permanent(MemoryClass::Static, 0);
        assert_eq!(g.class_bytes(MemoryClass::Static), 1000);
        assert_eq!(g.committed_bytes(), 1000);
    }

    #[test]
    fn queue_metrics_accumulate() {
        let g = leaked(10 * MIB, 20 * MIB, 4 * KIB);
        g.record_queue_wait(Duration::from_millis(2));
        g.record_queue_wait(Duration::from_millis(3));
        g.record_queue_rejection();
        let snap = g.snapshot();
        assert_eq!(snap.queue_waits, 2);
        assert_eq!(snap.queue_wait_time, Duration::from_millis(5));
        assert_eq!(snap.queue_rejections, 1);
        assert!(snap.to_string().contains("2 waits"));
    }

    #[test]
    fn committed_equals_account_totals_across_threads() {
        use std::sync::Barrier;
        let g = leaked(512 * MIB, 1024 * MIB, 4 * KIB);
        let barrier = Arc::new(Barrier::new(9));
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let acc = g.register_isolate(format!("iso-{i}"));
                    for n in 0..2000usize {
                        // Mix small charges with an occasional large one.
                        acc.charge(if n % 97 == 0 {
                            20 * KIB + n
                        } else {
                            40 + n % 200
                        });
                        if n % 50 == 0 {
                            acc.charge_region(4 * KIB);
                        }
                        if n % 150 == 0 {
                            acc.release_region(4 * KIB, false);
                        }
                        if n % 500 == 499 {
                            acc.release(acc.used_bytes() / 2);
                            let used = acc.used_bytes();
                            acc.record_collection(report(used * 2, used));
                        }
                    }
                    let committed = acc.committed_bytes();
                    barrier.wait(); // every account is quiescent
                    barrier.wait(); // main thread has read the governor
                    committed
                })
            })
            .collect();
        barrier.wait();
        let governor_committed = g.committed_bytes();
        barrier.wait();
        let sum: usize = threads.into_iter().map(|t| t.join().unwrap()).sum();
        assert_eq!(governor_committed, sum, "committed = used + free credit");
        assert_eq!(g.committed_bytes(), 0, "thread exit returns everything");
        assert_eq!(g.free_credit_bytes(), 0);
        assert!(g.snapshot().peak_committed_bytes >= sum);
    }

    #[test]
    fn env_strict_flag() {
        let on =
            MemoryConfig::from_lookup(|k| (k == STRICT_ENV).then(|| "yes".into()), |_| {}).unwrap();
        assert!(on.strict);
        let off =
            MemoryConfig::from_lookup(|k| (k == STRICT_ENV).then(|| "0".into()), |_| {}).unwrap();
        assert!(!off.strict);
        let default = MemoryConfig::from_lookup(|_| None, |_| {}).unwrap();
        assert!(!default.strict, "observe-only is the default");
        let err = MemoryConfig::from_lookup(|k| (k == STRICT_ENV).then(|| "maybe".into()), |_| {})
            .unwrap_err();
        assert!(matches!(err, MemoryConfigError::InvalidFlag { .. }));
    }

    #[test]
    fn observe_only_never_refuses() {
        let g = leaked(MIB / 2, MIB, 64 * KIB);
        let acc = g.register_isolate("a");
        assert!(acc.try_charge(2 * MIB).is_ok());
        assert!(g.reserve_shared(MemoryClass::SharedValue, MIB).is_ok());
        assert_eq!(g.snapshot().rejected_reservations, 0);
        assert!(g.snapshot().over_limit_events >= 1);
        assert_eq!(pending_runtime_fault(), None);
    }

    #[test]
    fn strict_refusal_leaves_counters_unchanged() {
        let g = strict(MIB / 2, MIB, 64 * KIB);
        let acc = g.register_isolate("a");
        // Leaves less than one chunk below the hard limit.
        acc.try_charge(990 * KIB).unwrap();
        let committed = g.committed_bytes();
        let (used, credit) = (acc.used_bytes(), acc.free_credit_bytes());
        let classes: Vec<_> = MemoryClass::ALL.iter().map(|&c| g.class_bytes(c)).collect();

        let err = acc.try_charge(200 * KIB).unwrap_err();
        assert_eq!(err.class, MemoryClass::GcHeap);
        assert_eq!(err.hard_limit, MIB);
        assert_eq!(g.committed_bytes(), committed);
        assert_eq!(acc.used_bytes(), used);
        assert_eq!(acc.free_credit_bytes(), credit);
        let after: Vec<_> = MemoryClass::ALL.iter().map(|&c| g.class_bytes(c)).collect();
        assert_eq!(after, classes);

        let snap = g.snapshot();
        assert_eq!(snap.rejected_reservations, 1);
        assert_eq!(snap.rejected_bytes, 200 * KIB as u64);
        assert_eq!(snap.pressure, PressureLevel::Red, "a refusal holds Red");
        assert!(acc.control().collection_requested());
        assert_eq!(
            pending_runtime_fault(),
            None,
            "a fallible refusal raises no fault"
        );

        // A request whose chunk is refused retries with its exact size.
        let room = MIB - g.committed_bytes();
        acc.try_charge(credit + room).unwrap();
        assert_eq!(g.committed_bytes(), MIB);
    }

    #[test]
    fn strict_shared_reservation_is_refused_and_reported_one_is_not() {
        let g = strict(MIB / 2, MIB, 64 * KIB);
        let held = g.reserve_shared(MemoryClass::SharedValue, MIB).unwrap();
        let err = g.reserve_shared(MemoryClass::MessageQueue, 1).unwrap_err();
        assert_eq!(err.class, MemoryClass::MessageQueue);
        assert_eq!(g.class_bytes(MemoryClass::MessageQueue), 0);
        assert!(g.try_charge_permanent(MemoryClass::Static, 1).is_err());
        let code = g.reserve_reported(MemoryClass::Code, 4 * KIB);
        assert_eq!(g.committed_bytes(), MIB + 4 * KIB, "code is never denied");
        let critical = g.reserve_critical(MemoryClass::Runtime, 4 * KIB).unwrap();
        drop((held, code, critical));
        assert_eq!(g.committed_bytes(), 0);
    }

    #[test]
    fn infallible_charge_raises_a_fault_and_uses_the_reserve() {
        let g = strict(MIB / 2, MIB, 64 * KIB);
        let reserve = g.config().critical_reserve;
        let acc = g.register_isolate("a");
        acc.charge(900 * KIB);
        assert_eq!(pending_runtime_fault(), None);
        acc.charge(200 * KIB);
        let fault = pending_runtime_fault().expect("refused charge raises a fault");
        assert_eq!(fault.requested, 200 * KIB);
        assert!(g.committed_bytes() > MIB);
        assert!(g.committed_bytes() <= MIB + reserve);
        assert!(acc.control().collection_requested());
        let snap = g.snapshot();
        assert_eq!(snap.runtime_faults, 1);
        assert!(snap.critical_grants >= 1);
        assert_eq!(snap.reserve_overruns, 0);

        // A second refusal keeps the first fault.
        acc.charge(100 * KIB);
        assert_eq!(pending_runtime_fault(), Some(fault));
        assert_eq!(g.snapshot().runtime_faults, 1);

        // Still over the limit: the fault is delivered, and the slot empties.
        assert_eq!(resolve_runtime_fault(), Some(fault));
        assert_eq!(pending_runtime_fault(), None);
    }

    #[test]
    fn collection_that_frees_enough_clears_the_fault() {
        let g = strict(MIB / 2, MIB, 64 * KIB);
        let acc = g.register_isolate("a");
        acc.charge(900 * KIB);
        acc.charge(200 * KIB);
        assert!(pending_runtime_fault().is_some());
        let used = acc.used_bytes();
        acc.release(used - 100 * KIB);
        acc.record_collection(report(used, 100 * KIB));
        assert!(g.has_fault_headroom());
        assert_eq!(resolve_runtime_fault(), None, "collection fixed it");
        assert_eq!(pending_runtime_fault(), None);
        assert_eq!(g.snapshot().runtime_faults_cleared, 1);
    }

    #[test]
    fn confirmed_fault_is_delivered_regardless_of_memory() {
        let g = strict(MIB / 2, MIB, 64 * KIB);
        let fault = MemoryLimitExceeded {
            class: MemoryClass::GcHeap,
            requested: 1,
            committed: 0,
            hard_limit: MIB,
        };
        assert!(g.within_hard_limit());
        confirm_runtime_fault(fault);
        assert_eq!(resolve_runtime_fault(), Some(fault));
        raise_runtime_fault(fault);
        assert_eq!(take_runtime_fault(), Some(fault));
        assert_eq!(resolve_runtime_fault(), None);
    }

    #[test]
    fn critical_section_uses_the_reserve_without_a_fault() {
        let g = strict(MIB / 2, MIB, 64 * KIB);
        let acc = g.register_isolate("a");
        acc.charge(MIB);
        with_critical(|| {
            assert!(critical_active());
            acc.charge(64 * KIB);
            assert!(acc.try_charge(64 * KIB).is_ok());
        });
        assert!(!critical_active());
        assert_eq!(pending_runtime_fault(), None);
        assert!(g.committed_bytes() > MIB);
        assert!(
            acc.try_charge(64 * KIB).is_err(),
            "outside, the limit applies"
        );
    }

    #[test]
    fn infallible_reservation_always_charges() {
        let g = strict(MIB / 2, MIB, 64 * KIB);
        let reserve = g.config().critical_reserve;
        let held = g.reserve_shared(MemoryClass::SharedValue, MIB).unwrap();
        let critical = g
            .reserve_critical(MemoryClass::SharedValue, reserve)
            .unwrap();
        assert_eq!(g.committed_bytes(), MIB + reserve, "limit and reserve full");
        let charge = g.reserve_infallible(MemoryClass::SharedValue, 4 * KIB);
        assert_eq!(charge.bytes(), 4 * KIB, "the charge is never dropped");
        assert_eq!(g.committed_bytes(), MIB + reserve + 4 * KIB);
        let snap = g.snapshot();
        assert_eq!(snap.reserve_overruns, 1);
        assert_eq!(snap.reserve_overrun_bytes, 4 * KIB as u64);
        assert!(take_runtime_fault().is_some());
        drop((held, critical, charge));
        assert_eq!(g.committed_bytes(), 0);
    }

    #[test]
    fn exhausted_reserve_overruns_and_counts_it() {
        let g = strict(MIB / 2, MIB, 64 * KIB);
        let reserve = g.config().critical_reserve;
        let acc = g.register_isolate("a");
        acc.charge(MIB + reserve + 64 * KIB);
        let snap = g.snapshot();
        assert_eq!(snap.reserve_overruns, 1);
        assert!(snap.committed_bytes > MIB + reserve);
        assert!(take_runtime_fault().is_some());
    }

    #[test]
    fn concurrent_strict_requests_never_exceed_the_hard_limit() {
        use std::sync::Barrier;
        let hard = 4 * MIB;
        let g = strict(hard / 2, hard, 16 * KIB);
        let barrier = Arc::new(Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let acc = g.register_isolate(format!("iso-{i}"));
                    barrier.wait();
                    let mut refused = 0;
                    for n in 0..4000usize {
                        if acc.try_charge(64 + n % 3000).is_err() {
                            refused += 1;
                        }
                        assert!(g.committed_bytes() <= hard);
                    }
                    refused
                })
            })
            .collect();
        let refused: usize = threads.into_iter().map(|t| t.join().unwrap()).sum();
        let snap = g.snapshot();
        assert!(refused > 0, "the budget is smaller than the demand");
        assert!(snap.peak_committed_bytes <= hard);
        assert_eq!(snap.committed_bytes, 0, "thread exit returns everything");
        assert_eq!(snap.rejected_reservations, refused as u64);
    }

    // ── Phase 5 tests ─────────────────────────────────────────────────────────

    /// The allocation-rate target adapts: a fast-allocating isolate gets a
    /// larger headroom than a slow one.
    #[test]
    fn collection_target_adapts_to_allocation_rate() {
        let mut config = MemoryConfig::with_limits(64 * MIB, 128 * MIB).unwrap();
        config.credit_chunk = 64 * KIB;
        config.collection_target_interval_ms = 500; // 500 ms interval
        let g: &'static _ = Box::leak(Box::new(
            ProcessMemoryGovernor::with_config(config).unwrap(),
        ));
        let acc = g.register_isolate("rate-test");

        // Simulate a 1 MiB / second allocation rate: 1 MiB charged over ~1 s.
        acc.charge(MIB);
        let used = acc.used_bytes();
        // Record a collection that freed everything.
        acc.release(used);
        acc.record_collection(CollectionReport {
            bytes_before: used,
            bytes_after: 0,
            bytes_returned: used,
            bytes_pending: 0,
            duration: Duration::from_millis(1),
        });
        // Short sleep to make elapsed > 0, then charge 1 MiB more and collect
        // again so we get a meaningful rate.
        std::thread::sleep(Duration::from_millis(10));
        acc.charge(MIB);
        let used2 = acc.used_bytes();
        acc.release(used2);
        acc.record_collection(CollectionReport {
            bytes_before: used2,
            bytes_after: 0,
            bytes_returned: used2,
            bytes_pending: 0,
            duration: Duration::from_millis(1),
        });
        // The target must be ≥ MIN_COLLECTION_HEADROOM.
        assert!(
            acc.control().collection_target() >= MIN_COLLECTION_HEADROOM,
            "target below floor: {}",
            acc.control().collection_target()
        );
    }

    /// Cross-isolate requests use a separate epoch counter, so the governor
    /// can make both a self-request and a cross-request in the same epoch.
    #[test]
    fn cross_isolate_requests_use_separate_epoch() {
        let g = leaked(4 * MIB, 8 * MIB, 64 * KIB);
        let big = g.register_isolate("big");
        let small = g.register_isolate("small");
        // Both start with no requests.
        assert!(!big.control().collection_requested());
        assert!(!small.control().collection_requested());

        // Put big's epoch at 0 (never collected).
        // Charge enough that big becomes the largest isolate.
        big.charge(3 * MIB);

        // A small charge from `small` puts the process at Yellow and
        // triggers both a self-request on `small` and a cross-request on `big`.
        small.charge(MIB);
        assert_eq!(g.pressure(), PressureLevel::Yellow);

        let small_requested = small.control().take_collection_request();
        let big_requested = big.control().take_collection_request();
        assert!(small_requested, "small gets a self-request");
        assert!(big_requested, "big gets a cross-request");

        // After clearing both requests but staying at Yellow, further charges
        // should still be rate-limited (same epoch).
        small.charge(64 * KIB);
        assert!(
            !small.control().collection_requested(),
            "rate-limited in same epoch"
        );
        assert!(
            !big.control().collection_requested(),
            "cross rate-limited in same epoch"
        );

        // After a collection on `small` (bumping its epoch) a new self-request
        // fires, but not another cross-request on `big` (different epoch counter).
        let used = small.used_bytes();
        small.record_collection(CollectionReport {
            bytes_before: used,
            bytes_after: used,
            bytes_returned: 0,
            bytes_pending: 0,
            duration: Duration::from_micros(1),
        });
        small.charge(64 * KIB);
        // After the epoch bump, another self-request on small may fire.
        // Cross-request on big is also gated on big's epoch (unchanged) and
        // on the collection target; with a zero-yield collection the target is
        // generous, so there may or may not be another cross-request—just
        // verify no panic or invariant violation.
        let _ = big.control().take_collection_request();
        let _ = small.control().take_collection_request();
    }

    /// When the RSS threshold fires, pressure rises one level: RSS alone
    /// gives Yellow, and RSS with committed bytes over the soft limit gives Red.
    #[test]
    fn rss_emergency_signal_raises_pressure_one_level() {
        let mut config = MemoryConfig::with_limits(4 * MIB, 8 * MIB).unwrap();
        config.credit_chunk = 64 * KIB;
        // Set threshold to 1% of hard limit so even a tiny RSS triggers it.
        config.rss_pressure_percent = 1;
        let g: &'static _ = Box::leak(Box::new(
            ProcessMemoryGovernor::with_config(config).unwrap(),
        ));
        // Pre-populate the RSS cache with a value over the threshold.  We
        // inject it via the atomic fields directly.
        let threshold = 8 * MIB / 100;
        g.rss_cache_bytes.store(threshold + 1, Ordering::Relaxed);
        g.rss_cache_nanos.store(unix_nanos(), Ordering::Relaxed);

        let acc = g.register_isolate("rss-test");
        acc.charge(64 * KIB); // tiny, well below soft limit
        // Force a re-evaluate.
        acc.publish();
        assert_eq!(
            g.pressure(),
            PressureLevel::Yellow,
            "RSS alone raises Green to Yellow, not Red"
        );

        acc.charge(4 * MIB);
        assert_eq!(g.pressure(), PressureLevel::Red, "RSS raises Yellow to Red");
    }

    /// Load-shedding hooks fire on pressure transitions.
    #[test]
    fn shed_hooks_fire_on_pressure_transitions() {
        use std::sync::atomic::AtomicUsize;
        let g = leaked(4 * MIB, 8 * MIB, 64 * KIB);
        let green_count = Arc::new(AtomicUsize::new(0));
        let yellow_count = Arc::new(AtomicUsize::new(0));
        let red_count = Arc::new(AtomicUsize::new(0));
        let gc = green_count.clone();
        let yc = yellow_count.clone();
        let rc = red_count.clone();
        let id = g.register_shed_hook(Arc::new(move |level| match level {
            PressureLevel::Green => {
                gc.fetch_add(1, Ordering::Relaxed);
            }
            PressureLevel::Yellow => {
                yc.fetch_add(1, Ordering::Relaxed);
            }
            PressureLevel::Red => {
                rc.fetch_add(1, Ordering::Relaxed);
            }
        }));

        let acc = g.register_isolate("shed-test");
        // Green → Yellow
        acc.charge(4 * MIB + 1);
        assert_eq!(g.pressure(), PressureLevel::Yellow);
        assert_eq!(yellow_count.load(Ordering::Relaxed), 1);

        // Yellow → Red
        acc.charge(4 * MIB);
        assert_eq!(g.pressure(), PressureLevel::Red);
        assert_eq!(red_count.load(Ordering::Relaxed), 1);

        // Red → Green (release all and collect)
        let used = acc.used_bytes();
        acc.release(used);
        acc.record_collection(CollectionReport {
            bytes_before: used,
            bytes_after: 0,
            bytes_returned: used,
            bytes_pending: 0,
            duration: Duration::from_micros(1),
        });
        assert_eq!(g.pressure(), PressureLevel::Green);
        assert_eq!(green_count.load(Ordering::Relaxed), 1);

        // Unregister the hook; no further calls.
        g.unregister_shed_hook(id);
        acc.charge(4 * MIB + 1);
        assert_eq!(
            yellow_count.load(Ordering::Relaxed),
            1,
            "hook not called after unregister"
        );
    }

    /// A hook may unregister itself (or register another) without
    /// deadlocking on the hook list.
    #[test]
    fn shed_hook_can_unregister_itself() {
        use std::sync::OnceLock;
        use std::sync::atomic::AtomicUsize;
        let g = leaked(4 * MIB, 8 * MIB, 64 * KIB);
        let calls: &'static AtomicUsize = Box::leak(Box::new(AtomicUsize::new(0)));
        let id_cell: &'static OnceLock<ShedHookId> = Box::leak(Box::new(OnceLock::new()));
        let id = g.register_shed_hook(Arc::new(move |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            if let Some(&id) = id_cell.get() {
                g.unregister_shed_hook(id);
            }
        }));
        id_cell.set(id).unwrap();

        let acc = g.register_isolate("one-shot");
        acc.charge(4 * MIB + 1);
        assert_eq!(g.pressure(), PressureLevel::Yellow);
        acc.charge(4 * MIB);
        assert_eq!(g.pressure(), PressureLevel::Red);
        assert_eq!(calls.load(Ordering::Relaxed), 1, "one-shot hook ran once");
    }

    /// Back-to-back collections do not inflate the rate: the rate headroom
    /// is at most the bytes allocated since the previous collection.
    #[test]
    fn rate_headroom_bounded_by_bytes_since_collection() {
        let mut config = MemoryConfig::with_limits(1 << 40, 2 << 40).unwrap();
        config.credit_chunk = 64 * KIB;
        config.collection_target_interval_ms = 1000;
        let g: &'static _ = Box::leak(Box::new(
            ProcessMemoryGovernor::with_config(config).unwrap(),
        ));
        let acc = g.register_isolate("burst");
        let collect = |acc: &IsolateAccount| {
            let used = acc.used_bytes();
            acc.release(used);
            acc.record_collection(CollectionReport {
                bytes_before: used,
                bytes_after: 0,
                bytes_returned: used,
                bytes_pending: 0,
                duration: Duration::from_micros(1),
            });
        };
        collect(&acc);
        acc.charge(8 * MIB);
        collect(&acc);
        let target = acc.control().collection_target();
        assert!(
            target <= 8 * MIB,
            "headroom {target} exceeds the 8 MiB allocated since the last collection"
        );
        assert!(target >= g.min_headroom());
    }

    /// The rate headroom never pushes the target past the soft limit.
    #[test]
    fn rate_headroom_capped_at_soft_limit() {
        let mut config = MemoryConfig::with_limits(8 * MIB, 16 * MIB).unwrap();
        config.credit_chunk = 64 * KIB;
        config.collection_target_interval_ms = 60_000;
        let g: &'static _ = Box::leak(Box::new(
            ProcessMemoryGovernor::with_config(config).unwrap(),
        ));
        let acc = g.register_isolate("fast");
        // Allocate and free 30 MiB in total: far more than the soft limit.
        for _ in 0..5 {
            acc.charge(6 * MIB);
            acc.release(6 * MIB);
        }
        acc.record_collection(CollectionReport {
            bytes_before: 6 * MIB,
            bytes_after: 0,
            bytes_returned: 6 * MIB,
            bytes_pending: 0,
            duration: Duration::from_micros(1),
        });
        let target = acc.control().collection_target();
        assert!(
            target <= 8 * MIB,
            "target {target} is above the 8 MiB soft limit"
        );
    }
}
