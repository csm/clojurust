//! Ready-task tracking and a watchdog that can observe a blocked carrier.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::task::{Wake, Waker};
use std::time::{Duration, Instant};

use super::FiberId;
use cljrs_gc::fiber::SafepointError;

/// Read the machine stack, rather than an ASan fake-stack local's address.
#[inline(always)]
fn stack_pointer() -> usize {
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

thread_local! {
    pub(super) static SCHEDULER: Arc<Scheduler> = Scheduler::new();
}

pub(super) struct Scheduler {
    ready: AtomicUsize,
    running: Mutex<Option<Running>>,
    slice: Duration,
    stall: Duration,
    native: AtomicUsize,
}

struct Running {
    id: FiberId,
    heartbeat: Instant,
    reported: bool,
    wake: Weak<TaskWake>,
}

fn duration_env(name: &str, default: u64, micros: bool) -> Duration {
    let n = std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default);
    if micros {
        Duration::from_micros(n)
    } else {
        Duration::from_millis(n)
    }
}

/// Bound on distinct diagnostic names per generation. Runtime-built
/// closures allocate fresh names, so names are interned by content.
const MAX_NATIVE_NAMES: usize = 4096;
/// Frame tokens encode `generation * NAME_STRIDE + id`, with 1-based ids.
const NAME_STRIDE: usize = MAX_NATIVE_NAMES + 1;
const NAME_GENERATIONS: usize = usize::MAX / NAME_STRIDE;

/// Generation of the process-wide table, readable without its lock.
static NAME_GENERATION: AtomicUsize = AtomicUsize::new(0);

/// Process-wide native diagnostic names. Tokens are cached on each
/// `NativeFn`, so only a function's first call per generation locks. A full
/// table starts a new generation instead of losing new names for good;
/// tokens from an older generation are re-interned on their next call.
#[derive(Default)]
struct NativeNames {
    list: Vec<Arc<str>>,
    ids: HashMap<Arc<str>, usize>,
    generation: usize,
}

impl NativeNames {
    /// Intern by content, so equal names from fresh allocations share a token.
    fn intern(&mut self, name: &Arc<str>) -> usize {
        if let Some(token) = self.ids.get(name) {
            return *token;
        }
        if self.list.len() >= MAX_NATIVE_NAMES {
            self.list.clear();
            self.ids.clear();
            self.generation = (self.generation + 1) % NAME_GENERATIONS;
            NAME_GENERATION.store(self.generation, Ordering::Relaxed);
        }
        self.list.push(name.clone());
        let token = self.generation * NAME_STRIDE + self.list.len();
        self.ids.insert(name.clone(), token);
        token
    }

    /// The name for a frame token, if it belongs to the current generation.
    fn name(&self, token: usize) -> Option<&str> {
        if token == 0 || token / NAME_STRIDE != self.generation {
            return None;
        }
        self.list.get(token % NAME_STRIDE - 1).map(AsRef::as_ref)
    }
}

fn native_names() -> &'static Mutex<NativeNames> {
    static NAMES: OnceLock<Mutex<NativeNames>> = OnceLock::new();
    NAMES.get_or_init(Default::default)
}

struct Watches {
    schedulers: Vec<Weak<Scheduler>>,
    generation: u64,
}
struct Watchdog {
    state: Mutex<Watches>,
    changed: Condvar,
    /// Set before each scan; cleared once the scan finds a running fiber.
    idle: AtomicBool,
}
impl Watchdog {
    /// Called after publishing a running fiber. Only an idle watchdog, which
    /// may wait without a timeout, needs the lock and signal.
    fn notify_running(&self) {
        if !self.idle.swap(false, Ordering::SeqCst) {
            return;
        }
        let mut state = self.state.lock().unwrap();
        state.generation = state.generation.wrapping_add(1);
        self.changed.notify_one();
    }

    fn run(&self) {
        loop {
            // A run published after this store sees `idle` and signals.
            self.idle.store(true, Ordering::SeqCst);
            let (generation, live) = {
                let mut state = self.state.lock().unwrap();
                state.schedulers.retain(|s| s.strong_count() > 0);
                (
                    state.generation,
                    state
                        .schedulers
                        .iter()
                        .filter_map(Weak::upgrade)
                        .collect::<Vec<_>>(),
                )
            };
            let mut interval: Option<Duration> = None;
            for scheduler in live {
                if let Some(message) = scheduler.stall_message(Instant::now()) {
                    eprintln!("{message}");
                }
                if scheduler.running.lock().unwrap().is_some() {
                    interval = Some(interval.map_or(scheduler.stall, |n| n.min(scheduler.stall)));
                }
            }
            if interval.is_some() {
                self.idle.store(false, Ordering::SeqCst);
            }
            // Generation closes the race between the scan and waiting. Idle
            // processes sleep without a timer; active carriers need at most
            // one inspection per configured stall interval.
            let state = self.state.lock().unwrap();
            if state.generation != generation {
                continue;
            }
            if let Some(interval) = interval {
                drop(self.changed.wait_timeout(state, interval).unwrap());
            } else {
                drop(self.changed.wait(state).unwrap());
            }
        }
    }
}

fn watchdog() -> &'static Arc<Watchdog> {
    static WATCHDOG: OnceLock<Arc<Watchdog>> = OnceLock::new();
    WATCHDOG.get_or_init(|| {
        let watches = Arc::new(Watchdog {
            state: Mutex::new(Watches {
                schedulers: Vec::new(),
                generation: 0,
            }),
            changed: Condvar::new(),
            idle: AtomicBool::new(true),
        });
        let registry = watches.clone();
        // A LocalSet timer cannot run while its own carrier is blocked.
        std::thread::Builder::new()
            .name("cljrs-fiber-watchdog".into())
            .spawn(move || registry.run())
            .expect("cannot start fiber watchdog");
        watches
    })
}

impl Scheduler {
    fn new() -> Arc<Self> {
        let scheduler = Arc::new(Self {
            ready: AtomicUsize::new(0),
            native: AtomicUsize::new(0),
            running: Mutex::new(None),
            slice: duration_env("CLJRS_FIBER_SLICE_US", 10_000, true),
            stall: duration_env("CLJRS_FIBER_STALL_MS", 100, false),
        });
        watchdog()
            .state
            .lock()
            .unwrap()
            .schedulers
            .push(Arc::downgrade(&scheduler));
        scheduler
    }

    fn other_ready(&self, wake: &TaskWake) -> bool {
        self.ready.load(Ordering::Relaxed) > usize::from(wake.queued.load(Ordering::Relaxed))
    }

    fn stall_message(&self, now: Instant) -> Option<String> {
        let mut running = self.running.lock().unwrap();
        let run = running.as_mut()?;
        let wake = run.wake.upgrade()?;
        if run.reported
            || now.duration_since(run.heartbeat) < self.stall
            || !self.other_ready(&wake)
        {
            return None;
        }
        run.reported = true;
        Some(format!(
            "cljrs: fiber {} stalled its isolate for {:?}; native frame: {} (other fibers runnable)",
            run.id.0,
            now.duration_since(run.heartbeat),
            match self.native.load(Ordering::Relaxed) {
                0 => "<Rust body without safepoints>".to_string(),
                token => native_names()
                    .lock()
                    .unwrap()
                    .name(token)
                    .unwrap_or("<native function>")
                    .to_string(),
            }
        ))
    }
}

struct WakeState {
    closed: bool,
    parent: Option<Waker>,
}

pub(super) struct TaskWake {
    scheduler: Arc<Scheduler>,
    queued: AtomicBool,
    state: Mutex<WakeState>,
}

impl TaskWake {
    fn new(scheduler: Arc<Scheduler>) -> Arc<Self> {
        Arc::new(Self {
            scheduler,
            queued: AtomicBool::new(false),
            state: Mutex::new(WakeState {
                closed: false,
                parent: None,
            }),
        })
    }

    pub(super) fn schedule(self: &Arc<Self>) {
        self.wake_by_ref();
    }

    pub(super) fn poll(&self, parent: &Waker) {
        let mut state = self.state.lock().unwrap();
        if !state.parent.as_ref().is_some_and(|w| w.will_wake(parent)) {
            state.parent = Some(parent.clone());
        }
        if self.queued.swap(false, Ordering::Relaxed) {
            self.scheduler.ready.fetch_sub(1, Ordering::Relaxed);
        }
    }

    pub(super) fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        state.parent = None;
        if self.queued.swap(false, Ordering::Relaxed) {
            self.scheduler.ready.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl Wake for TaskWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        let parent = {
            let state = self.state.lock().unwrap();
            if state.closed {
                return;
            }
            if !self.queued.swap(true, Ordering::Relaxed) {
                self.scheduler.ready.fetch_add(1, Ordering::Relaxed);
            }
            state.parent.clone()
        };
        if let Some(parent) = parent {
            parent.wake();
        }
    }
}

pub(super) struct Scheduling {
    pub(super) wake: Arc<TaskWake>,
    /// `wake` as a Waker, built once rather than on every poll.
    pub(super) waker: Waker,
    id: FiberId,
    limit: usize,
    bytes: usize,
    slice_start: Cell<Instant>,
    ticks: Cell<u32>,
    cancelled: Cell<bool>,
    native: Cell<usize>,
}

impl Scheduling {
    pub(super) fn new(id: FiberId, limit: usize, bytes: usize) -> Self {
        let wake = TaskWake::new(SCHEDULER.with(Arc::clone));
        Self {
            waker: Waker::from(wake.clone()),
            wake,
            id,
            limit,
            bytes,
            slice_start: Cell::new(Instant::now()),
            ticks: Cell::new(0),
            cancelled: Cell::new(false),
            native: Cell::new(0),
        }
    }

    pub(super) fn enter(&self) -> RunGuard<'_> {
        let now = Instant::now();
        self.slice_start.set(now);
        let native = self
            .wake
            .scheduler
            .native
            .swap(self.native.get(), Ordering::Relaxed);
        // A fiber may drive or drop another on the same carrier; the guard
        // restores the outer fiber's entry when the inner resume returns.
        let outer = self
            .wake
            .scheduler
            .running
            .lock()
            .unwrap()
            .replace(Running {
                id: self.id,
                heartbeat: now,
                reported: false,
                wake: Arc::downgrade(&self.wake),
            });
        watchdog().notify_running();
        RunGuard {
            scheduling: self,
            outer,
            native,
        }
    }

    pub(super) fn enter_native(
        &self,
        name: &Arc<str>,
        cached: &cljrs_gc::fiber::NativeDiagnosticId,
    ) -> usize {
        let mut id = cached.get();
        if id == 0 || id / NAME_STRIDE != NAME_GENERATION.load(Ordering::Relaxed) {
            id = native_names().lock().unwrap().intern(name);
            cached.set(id);
        }
        let previous = self.native.replace(id);
        self.wake.scheduler.native.store(id, Ordering::Relaxed);
        previous
    }

    pub(super) fn exit_native(&self, previous: usize) {
        self.native.set(previous);
        self.wake
            .scheduler
            .native
            .store(previous, Ordering::Relaxed);
    }

    pub(super) fn cancel(&self) {
        self.cancelled.set(true);
    }
    pub(super) fn is_cancelled(&self) -> bool {
        self.cancelled.get()
    }

    pub(super) fn safepoint(&self) -> Result<bool, SafepointError> {
        // Reserve space for error construction and unwinding, including the
        // guard page (Stack::limit includes it). No signal-handler unwinding.
        let remaining = stack_pointer().saturating_sub(self.limit);
        if remaining < 256 * 1024 {
            return Err(SafepointError::StackOverflow(format!(
                "stack overflow in fiber {} ({} reserved bytes, {remaining} bytes remaining)",
                self.id.0, self.bytes
            )));
        }
        // The cancelled automatic yield already delivered the error. Let
        // cleanup (finally blocks) run to completion without yielding again;
        // an explicit park there unwinds to the nearest native boundary.
        if self.cancelled.get() {
            return Ok(false);
        }
        let ticks = self.ticks.get().wrapping_add(1);
        self.ticks.set(ticks);
        if !ticks.is_multiple_of(64) {
            return Ok(false);
        }
        let now = Instant::now();
        if let Some(run) = self.wake.scheduler.running.lock().unwrap().as_mut() {
            run.heartbeat = now;
            run.reported = false;
        }
        // Yield on every expired slice. Ready fibers are tracked, but other
        // LocalSet work (async state machines, delivery futures, timers, I/O
        // bridges) is not, and must not starve behind a CPU-bound fiber.
        Ok(
            now.duration_since(self.slice_start.get()) >= self.wake.scheduler.slice
                && cljrs_gc::mutator::can_park(),
        )
    }
}

pub(super) struct RunGuard<'a> {
    scheduling: &'a Scheduling,
    outer: Option<Running>,
    native: usize,
}
impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        let scheduler = &self.scheduling.wake.scheduler;
        // Restore the outer entry unchanged: an outer fiber that keeps
        // driving inner ones without reaching its own safepoint is stalled.
        *scheduler.running.lock().unwrap() = self.outer.take();
        scheduler.native.store(self.native, Ordering::Relaxed);
    }
}

impl Drop for Scheduling {
    fn drop(&mut self) {
        self.wake.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        static TEST_BORROW: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    cljrs_gc::mutator_slot!("cljrs-async/TEST_BORROW", refcell TEST_BORROW: Vec<u8>);

    #[test]
    fn native_name_tables_stay_bounded() {
        let mut names = NativeNames::default();
        // Fresh allocations of one name share a token.
        let first = names.intern(&Arc::from("partial-wrapper"));
        for _ in 0..10_000 {
            assert_eq!(names.intern(&Arc::from("partial-wrapper")), first);
        }
        assert_eq!(names.list.len(), 1);
        assert_eq!(names.name(first), Some("partial-wrapper"));
        // Distinct names stay bounded; a full table starts a new generation,
        // so later names are still reported and stale tokens are not misread.
        for i in 0..2 * MAX_NATIVE_NAMES {
            names.intern(&Arc::from(format!("generated-{i}")));
        }
        assert!(names.list.len() <= MAX_NATIVE_NAMES);
        assert_eq!(names.name(first), None);
        let late = names.intern(&Arc::from("late-name"));
        assert_eq!(names.name(late), Some("late-name"));
    }

    #[test]
    fn native_frames_cache_their_name_id_on_the_function() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        tokio::task::LocalSet::new().block_on(&rt, async {
            super::super::spawn(|| {
                let id = cljrs_gc::fiber::NativeDiagnosticId::default();
                let name: Arc<str> = Arc::from("cached-native");
                drop(cljrs_gc::fiber::native_frame(&name, &id));
                let assigned = id.get();
                assert_ne!(assigned, 0);
                // Warm calls reuse the cached id; the frame publishes it.
                let _frame = cljrs_gc::fiber::native_frame(&name, &id);
                assert_eq!(id.get(), assigned);
                assert_eq!(
                    SCHEDULER.with(|s| s.native.load(Ordering::Relaxed)),
                    assigned
                );
                Ok(cljrs_value::Value::Nil)
            })
            .unwrap()
            .await
            .unwrap()
            .into_result()
            .unwrap();
        });
    }

    #[test]
    fn nested_resumes_restore_the_outer_running_fiber() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        tokio::task::LocalSet::new().block_on(&rt, async {
            let outer = super::super::Fiber::new(|| {
                let running_id =
                    || SCHEDULER.with(|s| s.running.lock().unwrap().as_ref().map(|run| run.id));
                let outer_id = running_id().expect("outer fiber not running");
                let _native =
                    cljrs_gc::fiber::native_frame(&Arc::from("outer-native"), &Default::default());
                let native = SCHEDULER.with(|s| s.native.load(Ordering::Relaxed));
                // Drive another fiber to completion on this carrier.
                let mut inner = std::pin::pin!(
                    super::super::Fiber::new(|| { Ok(cljrs_value::Value::Nil) })
                        .unwrap()
                        .into_task()
                );
                let waker = std::task::Waker::noop();
                let mut cx = std::task::Context::from_waker(waker);
                assert!(inner.as_mut().poll(&mut cx).is_ready());
                assert_eq!(running_id(), Some(outer_id));
                assert_eq!(SCHEDULER.with(|s| s.native.load(Ordering::Relaxed)), native);
                Ok(cljrs_value::Value::Nil)
            })
            .unwrap();
            outer.into_task().await.unwrap();
        });
    }

    fn expire_slice() {
        let current = super::super::scheduling().unwrap();
        current
            .slice_start
            .set(Instant::now() - Duration::from_secs(1));
        current.ticks.set(63);
    }

    #[test]
    fn optional_yields_defer_for_locks_borrows_and_critical_sections() {
        use std::rc::Rc;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        tokio::task::LocalSet::new().block_on(&rt, async {
            let ran = Rc::new(Cell::new(false));
            let observed = ran.clone();
            let first = super::super::spawn(move || {
                let mutex = cljrs_gc::mutator::TrackedMutex::new(());
                {
                    let _held = mutex.lock().unwrap();
                    expire_slice();
                    cljrs_gc::fiber::safepoint().unwrap();
                    assert!(!observed.get());
                }
                TEST_BORROW.with(|slot| {
                    let _borrow = slot.borrow_mut();
                    expire_slice();
                    cljrs_gc::fiber::safepoint().unwrap();
                    assert!(!observed.get());
                });
                {
                    let _critical = cljrs_gc::enter_critical();
                    expire_slice();
                    cljrs_gc::fiber::safepoint().unwrap();
                    assert!(!observed.get());
                }
                expire_slice();
                let mut rooted = false;
                cljrs_gc::fiber::safepoint_with(|yield_now| {
                    rooted = true;
                    yield_now()
                })
                .unwrap();
                assert!(rooted && observed.get());
                Ok(cljrs_value::Value::Nil)
            })
            .unwrap();
            let peer = super::super::spawn(move || {
                ran.set(true);
                Ok(cljrs_value::Value::Nil)
            })
            .unwrap();
            first.await.unwrap().into_result().unwrap();
            peer.await.unwrap().into_result().unwrap();
            cljrs_gc::fiber::safepoint_with(|_| panic!("rooted a carrier safepoint")).unwrap();
        });
    }

    #[test]
    fn automatic_yield_preserves_unrelated_panic_payloads() {
        let payload = std::panic::catch_unwind(super::super::yield_slice).unwrap_err();
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap();
        assert!(message.contains("outside a fiber"), "{message}");
    }

    #[test]
    fn cancellation_does_not_resume_user_code_or_park_during_cleanup() {
        use std::rc::Rc;
        struct Cleanup(Rc<Cell<bool>>);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                self.0.set(true);
                super::super::park_on(std::future::pending::<()>());
                panic!("cancelled cleanup resumed");
            }
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        tokio::task::LocalSet::new().block_on(&rt, async {
            let cleaned = Rc::new(Cell::new(false));
            let done = cleaned.clone();
            let first = super::super::spawn(move || {
                let _cleanup = Cleanup(done);
                expire_slice();
                cljrs_gc::fiber::safepoint()
                    .map_err(|_| cljrs_runtime::env::error::EvalError::GasExhausted)?;
                panic!("cancelled body continued");
            })
            .unwrap();
            let abort = first.abort_handle();
            let peer = super::super::spawn(move || {
                abort.abort();
                Ok(cljrs_value::Value::Nil)
            })
            .unwrap();
            assert!(first.await.err().unwrap().is_cancelled());
            peer.await.unwrap().into_result().unwrap();
            assert!(cleaned.get());
        });
    }

    #[test]
    fn stack_pointer_stays_within_the_machine_stack_across_resumes() {
        use corosensei::stack::{DefaultStack, Stack};
        use corosensei::{Coroutine, CoroutineResult};

        let stack = DefaultStack::new(1024 * 1024).unwrap();
        let bounds = stack.limit().get()..stack.base().get();
        let mut coroutine = Coroutine::with_stack(stack, |yielder, ()| {
            yielder.suspend(stack_pointer());
            stack_pointer()
        });
        let CoroutineResult::Yield(before) = coroutine.resume(()) else {
            panic!("coroutine did not suspend");
        };
        let CoroutineResult::Return(after) = coroutine.resume(()) else {
            panic!("coroutine did not return");
        };
        assert!(bounds.contains(&before));
        assert!(bounds.contains(&after));
    }

    #[test]
    fn ready_registration_is_idempotent_and_closes_after_remote_wakes() {
        let scheduler = Scheduler::new();
        let wake = TaskWake::new(scheduler.clone());
        assert_eq!(scheduler.ready.load(Ordering::Relaxed), 0);
        wake.schedule();
        wake.schedule();
        assert_eq!(scheduler.ready.load(Ordering::Relaxed), 1);
        wake.poll(Waker::noop());
        assert_eq!(scheduler.ready.load(Ordering::Relaxed), 0);
        let remote = wake.clone();
        std::thread::spawn(move || {
            remote.wake_by_ref();
            remote.wake_by_ref();
        })
        .join()
        .unwrap();
        assert_eq!(scheduler.ready.load(Ordering::Relaxed), 1);
        wake.close();
        wake.wake_by_ref();
        assert_eq!(scheduler.ready.load(Ordering::Relaxed), 0);
    }
}
