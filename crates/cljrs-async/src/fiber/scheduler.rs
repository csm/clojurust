//! Ready-task tracking and a watchdog that can observe a blocked carrier.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
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
    static NATIVE_NAMES: RefCell<HashMap<Arc<str>, usize>> = RefCell::new(HashMap::new());
}

pub(super) struct Scheduler {
    ready: AtomicUsize,
    running: Mutex<Option<Running>>,
    slice: Duration,
    stall: Duration,
    names: Mutex<Vec<Arc<str>>>,
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

impl Scheduler {
    fn new() -> Arc<Self> {
        let scheduler = Arc::new(Self {
            ready: AtomicUsize::new(0),
            names: Mutex::new(Vec::new()),
            native: AtomicUsize::new(0),
            running: Mutex::new(None),
            slice: duration_env("CLJRS_FIBER_SLICE_US", 10_000, true),
            stall: duration_env("CLJRS_FIBER_STALL_MS", 100, false),
        });
        static WATCHES: OnceLock<Arc<Mutex<Vec<Weak<Scheduler>>>>> = OnceLock::new();
        let watches = WATCHES.get_or_init(|| {
            let watches = Arc::new(Mutex::new(Vec::<Weak<Scheduler>>::new()));
            let registry = watches.clone();
            // A LocalSet timer cannot run while its own carrier is blocked.
            std::thread::Builder::new()
                .name("cljrs-fiber-watchdog".into())
                .spawn(move || {
                    loop {
                        std::thread::sleep(Duration::from_millis(10));
                        let live = {
                            let mut registry = registry.lock().unwrap();
                            registry.retain(|s| s.strong_count() > 0);
                            registry
                                .iter()
                                .filter_map(Weak::upgrade)
                                .collect::<Vec<_>>()
                        };
                        for scheduler in live {
                            if let Some(message) = scheduler.stall_message(Instant::now()) {
                                eprintln!("{message}");
                            }
                        }
                    }
                })
                .expect("cannot start fiber watchdog");
            watches
        });
        watches.lock().unwrap().push(Arc::downgrade(&scheduler));
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
            self.names
                .lock()
                .unwrap()
                .get(self.native.load(Ordering::Relaxed).wrapping_sub(1))
                .map(AsRef::as_ref)
                .unwrap_or("<Rust body without safepoints>")
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
        state.parent = Some(parent.clone());
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
        Self {
            wake: TaskWake::new(SCHEDULER.with(Arc::clone)),
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
        self.wake
            .scheduler
            .native
            .store(self.native.get(), Ordering::Relaxed);
        *self.wake.scheduler.running.lock().unwrap() = Some(Running {
            id: self.id,
            heartbeat: now,
            reported: false,
            wake: Arc::downgrade(&self.wake),
        });
        RunGuard(self)
    }

    pub(super) fn enter_native(&self, name: &Arc<str>) -> usize {
        let id = NATIVE_NAMES.with(|cache| {
            let mut cache = cache.borrow_mut();
            if let Some(id) = cache.get(name) {
                return *id;
            }
            let mut names = self.wake.scheduler.names.lock().unwrap();
            names.push(name.clone());
            let id = names.len();
            cache.insert(name.clone(), id);
            id
        });
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
        if self.cancelled.get() {
            return Err(SafepointError::Cancelled);
        }
        // Reserve space for error construction and unwinding, including the
        // guard page (Stack::limit includes it). No signal-handler unwinding.
        let remaining = stack_pointer().saturating_sub(self.limit);
        if remaining < 256 * 1024 {
            return Err(SafepointError::StackOverflow(format!(
                "stack overflow in fiber {} ({} reserved bytes, {remaining} bytes remaining)",
                self.id.0, self.bytes
            )));
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
        Ok(
            now.duration_since(self.slice_start.get()) >= self.wake.scheduler.slice
                && self.wake.scheduler.other_ready(&self.wake)
                && cljrs_gc::mutator::can_park(),
        )
    }
}

pub(super) struct RunGuard<'a>(&'a Scheduling);
impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        *self.0.wake.scheduler.running.lock().unwrap() = None;
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
        static TEST_BORROW: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }
    cljrs_gc::mutator_slot!("cljrs-async/TEST_BORROW", refcell TEST_BORROW: Vec<u8>);

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
