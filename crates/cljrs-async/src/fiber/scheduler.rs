//! Ready-task tracking and a watchdog that can observe a blocked carrier.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::task::{Wake, Waker};
use std::time::{Duration, Instant};

use super::FiberId;
use cljrs_gc::fiber::SafepointError;

thread_local! {
    pub(super) static SCHEDULER: Arc<Scheduler> = Scheduler::new();
}

pub(super) struct Scheduler {
    ready: AtomicUsize,
    running: Mutex<Option<Running>>,
    slice: Duration,
    stall: Duration,
}

struct Running {
    id: FiberId,
    heartbeat: Instant,
    reported: bool,
    native: Option<Arc<str>>,
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
            run.native
                .as_deref()
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
    native: RefCell<Vec<Arc<str>>>,
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
            native: RefCell::new(Vec::new()),
        }
    }

    pub(super) fn enter(&self) -> RunGuard<'_> {
        let now = Instant::now();
        self.slice_start.set(now);
        *self.wake.scheduler.running.lock().unwrap() = Some(Running {
            id: self.id,
            heartbeat: now,
            reported: false,
            native: self.native.borrow().last().cloned(),
            wake: Arc::downgrade(&self.wake),
        });
        RunGuard(self)
    }

    pub(super) fn native_frame(&self, name: Option<&Arc<str>>) {
        let frame = {
            let mut frames = self.native.borrow_mut();
            if let Some(name) = name {
                frames.push(name.clone());
            } else {
                frames.pop();
            }
            frames.last().cloned()
        };
        if let Some(run) = self.wake.scheduler.running.lock().unwrap().as_mut() {
            run.native = frame;
        }
    }

    pub(super) fn safepoint(&self) -> Result<(), SafepointError> {
        if self.cancelled.get() {
            return Err(SafepointError::Cancelled);
        }
        let marker = 0u8;
        // Reserve space for error construction and unwinding, including the
        // guard page (Stack::limit includes it). No signal-handler unwinding.
        let remaining = (&marker as *const u8 as usize).saturating_sub(self.limit);
        if remaining < 256 * 1024 {
            return Err(SafepointError::StackOverflow(format!(
                "stack overflow in fiber {} ({} reserved bytes, {remaining} bytes remaining)",
                self.id.0, self.bytes
            )));
        }
        let ticks = self.ticks.get().wrapping_add(1);
        self.ticks.set(ticks);
        if !ticks.is_multiple_of(64) {
            return Ok(());
        }
        let now = Instant::now();
        if let Some(run) = self.wake.scheduler.running.lock().unwrap().as_mut() {
            run.heartbeat = now;
            run.reported = false;
        }
        if now.duration_since(self.slice_start.get()) >= self.wake.scheduler.slice
            && self.wake.scheduler.other_ready(&self.wake)
            && cljrs_gc::mutator::guards_held() == 0
            && !cljrs_gc::governor::in_critical_section()
        {
            cljrs_gc::mutator::assert_parkable();
            // Forced cancellation must not unwind across JIT/AOT C ABI frames.
            // Carry it out as an uncatchable resource failure instead, so those
            // frames return normally and the fiber's Rust scopes can clean up.
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                super::park_on(tokio::task::yield_now());
            }))
            .is_err()
            {
                self.cancelled.set(true);
                return Err(SafepointError::Cancelled);
            }
        }
        Ok(())
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
