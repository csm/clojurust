//! Stackful, thread-pinned computations driven by a Tokio LocalSet.
//!
//! `park_on` can wait through arbitrary synchronous Rust/Clojure frames. The
//! future stays pinned on the fiber stack, so it may borrow stack locals. The
//! driver polls it with the fiber's mutator state installed. Cancellation
//! unwinds that stack with the same state installed before returning it to the
//! bounded stack pool.
//!
//! As with other Rust tasks, opaque closure/future captures are not precise GC
//! roots. Use `with_roots`/`spawn_rooted` for captured Values, and the runtime's
//! root guards for locals that must survive a park or a collecting safepoint.

#![allow(clippy::result_large_err)]

use std::cell::{Cell, RefCell};
use std::future::{Future, poll_fn};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::{Pin, pin};
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

use cljrs_gc::mutator::{MutatorState, assert_parkable};
use cljrs_runtime::env::error::{EvalError, EvalResult};
use cljrs_runtime::env::gc_roots::root_values;
use cljrs_value::Value;
use corosensei::stack::{DefaultStack, Stack};

mod scheduler;
use cljrs_gc::fiber::StackReservation;
use corosensei::{Coroutine, CoroutineResult, Yielder};
use scheduler::Scheduling;

const DEFAULT_STACK_BYTES: usize = 1024 * 1024;
const MAX_POOLED_STACKS: usize = 32;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

type FiberCoroutine = Coroutine<(), Park, EvalResult>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FiberId(u64);

#[derive(Clone, Copy)]
struct Current {
    id: FiberId,
    yielder: *const Yielder<(), Park>,
    scheduling: *const Scheduling,
}

thread_local! {
    // Driver metadata, never computation state: set only around resume/unwind.
    static CURRENT: Cell<Option<Current>> = const { Cell::new(None) };
    static STACK_POOL: RefCell<Vec<(usize, DefaultStack, StackReservation)>> = const { RefCell::new(Vec::new()) };
}

/// The running fiber, or None on the carrier thread (including future polls).
pub fn current() -> Option<FiberId> {
    CURRENT.with(|current| current.get().map(|c| c.id))
}

fn stack_bytes() -> Result<usize, EvalError> {
    match std::env::var("CLJRS_FIBER_STACK_KB") {
        Ok(value) => value
            .parse::<usize>()
            .ok()
            .and_then(|kb| kb.checked_mul(1024))
            .filter(|bytes| *bytes >= 64 * 1024)
            .ok_or_else(|| {
                EvalError::Runtime(
                    "CLJRS_FIBER_STACK_KB must be at least 64 and fit in usize".into(),
                )
            }),
        Err(std::env::VarError::NotPresent) => Ok(DEFAULT_STACK_BYTES),
        Err(_) => Err(EvalError::Runtime("invalid CLJRS_FIBER_STACK_KB".into())),
    }
}

// Private lifetime-erased borrow of a Future pinned on the suspended stack.
// It is used only until that stack resumes or unwinds, never owned or dropped
// by the carrier. Keeping this private prevents callers from extending it.
struct Park {
    ptr: *mut (),
    poll: unsafe fn(*mut (), &mut Context<'_>) -> Poll<()>,
}

impl Park {
    fn new<F: Future<Output = ()>>(future: Pin<&mut F>) -> Self {
        unsafe fn poll<F: Future<Output = ()>>(ptr: *mut (), cx: &mut Context<'_>) -> Poll<()> {
            // SAFETY: park_on pins F on the live coroutine stack; the driver
            // never resumes/unwinds the coroutine concurrently with this call.
            unsafe { Pin::new_unchecked(&mut *ptr.cast::<F>()) }.poll(cx)
        }
        Self {
            // SAFETY: the pointee stays pinned until the park returns.
            ptr: unsafe { future.get_unchecked_mut() } as *mut F as *mut (),
            poll: poll::<F>,
        }
    }
}

/// Suspend this fiber until `future` completes. May borrow fiber stack locals.
/// Panics outside a fiber or when a runtime lock/critical section is held.
/// Managed values retained by the future must have precise GC root guards.
pub fn park_on<F: Future>(future: F) -> F::Output {
    assert_parkable();
    let current = CURRENT
        .with(Cell::get)
        .expect("park_on called outside a fiber");
    let mut future = pin!(future);
    let mut output = None;
    {
        let mut wait = pin!(poll_fn(|cx| match future.as_mut().poll(cx) {
            Poll::Ready(value) => {
                output = Some(value);
                Poll::Ready(())
            }
            Poll::Pending => Poll::Pending,
        }));
        let park = Park::new(wait.as_mut());
        // SAFETY: CURRENT is installed only while this coroutine is resumed or
        // unwound; its Yielder remains at a stable address on its own stack.
        unsafe { &*current.yielder }.suspend(park);
    }
    output.expect("fiber resumed before its park future completed")
}

struct CurrentGuard(Option<Current>);
impl CurrentGuard {
    fn enter(current: Option<Current>) -> Self {
        Self(CURRENT.with(|slot| slot.replace(current)))
    }
}
impl Drop for CurrentGuard {
    fn drop(&mut self) {
        CURRENT.with(|slot| slot.set(self.0));
    }
}

/// An unstarted fiber. Construction is lazy; spawning requires a LocalSet.
/// All execution and destruction must stay on the creating isolate thread.
pub struct Fiber {
    task: FiberTask,
}

// The pointee belongs to the live FiberTask; it stays boxed across task moves.
fn scheduling() -> Option<&'static Scheduling> {
    CURRENT.with(Cell::get).map(|c| unsafe { &*c.scheduling })
}

fn install_parking_hooks() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        cljrs_gc::fiber::install(cljrs_gc::fiber::Hooks {
            safepoint: || scheduling().map_or(Ok(()), Scheduling::safepoint),
            enter_native: |name| {
                if let Some(s) = scheduling() {
                    s.native_frame(Some(name));
                }
            },
            exit_native: || {
                if let Some(s) = scheduling() {
                    s.native_frame(None);
                }
            },
        });
        cljrs_value::parking::install(cljrs_value::parking::Hooks {
            current: || current().map(|id| id.0),
            park: |future, timeout| {
                park_on(async {
                    match timeout {
                        Some(duration) => tokio::time::timeout(duration, future).await.is_ok(),
                        None => {
                            future.await;
                            true
                        }
                    }
                })
            },
            sleep: |duration| park_on(tokio::time::sleep(duration)),
            blocking: |job| {
                park_on(async {
                    crate::worker_pool::WorkerPool::global()
                        .handle()
                        .spawn_blocking(job)
                        .await
                        .expect("blocking worker panicked")
                })
            },
        });
    });
}

impl Fiber {
    pub fn new(body: impl FnOnce() -> EvalResult + 'static) -> Result<Self, EvalError> {
        Self::with_roots(Vec::new(), body)
    }

    /// Keep captured values rooted before the first poll and until completion.
    /// GC builds poison active regions for opaque captures. No-gc builds
    /// lease those regions only until task completion or cancellation.
    pub fn with_roots(
        roots: Vec<Value>,
        body: impl FnOnce() -> EvalResult + 'static,
    ) -> Result<Self, EvalError> {
        // SAFETY: this constructor requires a static body.
        unsafe { Self::with_stack_bytes(roots, body, stack_bytes()?) }
    }

    // SAFETY: borrowed bodies must be fully driven or dropped before returning
    // to their caller. Only run_root may use a non-static body.
    unsafe fn with_stack_bytes(
        roots: Vec<Value>,
        body: impl FnOnce() -> EvalResult,
        bytes: usize,
    ) -> Result<Self, EvalError> {
        install_parking_hooks();
        let cached = STACK_POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            pool.iter()
                .position(|(size, _, _)| *size == bytes)
                .map(|i| pool.swap_remove(i))
        });
        let (stack, mut reservation) = if let Some((_, stack, mut reservation)) = cached {
            reservation.set_pooled(false);
            (stack, reservation)
        } else {
            let reservation = StackReservation::new(bytes)
                .or_else(|_| {
                    // Idle stacks must not prevent admission of a differently sized stack.
                    let idle = STACK_POOL.with(|pool| std::mem::take(&mut *pool.borrow_mut()));
                    drop(idle);
                    StackReservation::new(bytes)
                })
                .map_err(EvalError::MemoryLimitExceeded)?;
            let stack = DefaultStack::new(bytes)
                .map_err(|e| EvalError::Runtime(format!("cannot allocate fiber stack: {e}")))?;
            (stack, reservation)
        };
        reservation
            .account_actual_bytes(stack.base().get() - stack.limit().get())
            .map_err(EvalError::MemoryLimitExceeded)?;
        let id = FiberId(NEXT_ID.fetch_add(1, Ordering::Relaxed));
        #[cfg(not(feature = "no-gc"))]
        cljrs_gc::region::poison_active_regions();
        #[cfg(feature = "no-gc")]
        let lease = cljrs_gc::region::RegionLease::capture_active();
        #[cfg(feature = "no-gc")]
        let allocation = cljrs_gc::region::AllocationSnapshot::capture();
        #[cfg(feature = "no-gc")]
        let lease = {
            let mut lease = lease;
            lease.extend(&cljrs_gc::region::RegionLease::for_values(&roots));
            lease
        };
        let reports = crate::channel::FailureReports::current();
        #[cfg(feature = "no-gc")]
        let result_lease = std::rc::Rc::new(RefCell::new(cljrs_gc::region::RegionLease::default()));
        let roots = roots.into_boxed_slice();
        let state = MutatorState::fresh();
        let scheduling = Box::new(Scheduling::new(id, stack.limit().get(), bytes));
        let scheduling_ptr = &*scheduling as *const Scheduling;
        // Register captures in the fiber's detached state, not the caller's.
        let mut task = FiberTask {
            co: None,
            scheduling,
            reservation: Some(reservation),
            state,
            pending: None,
            roots,
            roots_guard: None,
            bytes,
            id,
            yielder: std::ptr::null(),
            root: false,
            #[cfg(feature = "no-gc")]
            result_lease: result_lease.clone(),
            #[cfg(feature = "no-gc")]
            owned_result: false,
            #[cfg(feature = "no-gc")]
            _captures: lease.clone(),
            #[cfg(feature = "no-gc")]
            allocation: allocation.clone(),
        };
        task.state.swap_with_current();
        reports.install();
        task.roots_guard = Some(root_values(&task.roots));
        task.state.swap_with_current();
        task.co = Some(unsafe {
            Coroutine::with_stack_unchecked(stack, move |yielder, ()| {
                CURRENT.with(|slot| {
                    slot.set(Some(Current {
                        id,
                        yielder,
                        scheduling: scheduling_ptr,
                    }))
                });
                let _monitors = cljrs_runtime::env::monitor::MonitorCleanup;
                #[cfg(feature = "no-gc")]
                let _lease_scope = lease.enter();
                #[cfg(feature = "no-gc")]
                let _allocation_scope = allocation.enter();
                #[cfg(feature = "no-gc")]
                let _scratch = cljrs_gc::alloc_ctx::ScratchGuard::new();
                let result = body();
                #[cfg(feature = "no-gc")]
                match &result {
                    Ok(value) | Err(EvalError::Thrown(value)) => {
                        *result_lease.borrow_mut() = cljrs_gc::region::RegionLease::for_value(value)
                    }
                    _ => {}
                }
                result
            })
        });
        Ok(Self { task })
    }

    pub fn id(&self) -> FiberId {
        self.task.id
    }
    pub fn into_task(self) -> FiberTask {
        self.task.scheduling.wake.schedule();
        self.task
    }
}

/// A thread-pinned future that drives a stackful fiber.
pub struct FiberTask {
    co: Option<FiberCoroutine>,
    scheduling: Box<Scheduling>,
    reservation: Option<StackReservation>,
    state: MutatorState,
    pending: Option<Park>,
    // Boxed: addresses registered in state must survive moving the task.
    roots: Box<[Value]>,
    roots_guard: Option<cljrs_runtime::env::gc_roots::ValueRootGuard>,
    bytes: usize,
    id: FiberId,
    yielder: *const Yielder<(), Park>,
    root: bool,
    #[cfg(feature = "no-gc")]
    result_lease: std::rc::Rc<RefCell<cljrs_gc::region::RegionLease>>,
    #[cfg(feature = "no-gc")]
    owned_result: bool,
    #[cfg(feature = "no-gc")]
    _captures: cljrs_gc::region::RegionLease,
    #[cfg(feature = "no-gc")]
    allocation: cljrs_gc::region::AllocationSnapshot,
}

impl FiberTask {
    fn release_stack(&mut self) {
        let co = self.co.take().expect("fiber already completed");
        let stack = co.into_stack();
        self.scheduling.wake.close();
        let mut reservation = self.reservation.take().unwrap();
        STACK_POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            if pool.len() < MAX_POOLED_STACKS
                && cljrs_gc::governor().pressure() == cljrs_gc::PressureLevel::Green
            {
                reservation.set_pooled(true);
                pool.push((self.bytes, stack, reservation));
            }
        });
    }

    fn unwind(&mut self) {
        self.pending = None;
        if let Some(co) = &mut self.co {
            let _current = CurrentGuard::enter(if self.yielder.is_null() {
                None
            } else {
                Some(Current {
                    id: self.id,
                    yielder: self.yielder,
                    scheduling: &*self.scheduling,
                })
            });
            co.force_unwind();
        }
    }
}

impl Future for FiberTask {
    type Output = EvalResult;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        assert!(this.co.is_some(), "fiber polled after completion");
        assert_parkable();
        this.scheduling.wake.poll(cx.waker());
        let waker = std::task::Waker::from(this.scheduling.wake.clone());
        let cx = &mut Context::from_waker(&waker);
        this.state.swap_with_current();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let _current = CurrentGuard::enter(None);
            if let Some(park) = &mut this.pending {
                // SAFETY: this coroutine has not resumed since yielding Park.
                if unsafe { (park.poll)(park.ptr, cx) }.is_pending() {
                    return Poll::Pending;
                }
                this.pending = None;
            }
            loop {
                if !this.yielder.is_null() {
                    CURRENT.with(|slot| {
                        slot.set(Some(Current {
                            id: this.id,
                            yielder: this.yielder,
                            scheduling: &*this.scheduling,
                        }))
                    });
                }
                let outcome = {
                    let _running = this.scheduling.enter();
                    this.co.as_mut().unwrap().resume(())
                };
                match outcome {
                    CoroutineResult::Return(result) => return Poll::Ready(result),
                    CoroutineResult::Yield(park) => {
                        this.yielder = CURRENT.with(|slot| slot.get().unwrap().yielder);
                        CURRENT.with(|slot| slot.set(None));
                        // Poll immediately so Pending registers the task's waker.
                        if unsafe { (park.poll)(park.ptr, cx) }.is_pending() {
                            this.pending = Some(park);
                            return Poll::Pending;
                        }
                    }
                }
            }
        }));
        let result = match outcome {
            Ok(result) => result,
            Err(payload) => {
                this.unwind();
                let message = payload
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| payload.downcast_ref::<&str>().copied())
                    .unwrap_or("non-string panic");
                Poll::Ready(Err(EvalError::Runtime(format!(
                    "fiber panicked: {message}"
                ))))
            }
        };
        if result.is_ready() {
            this.roots_guard.take();
        }
        this.state.swap_with_current();
        if result.is_ready() {
            #[cfg(feature = "no-gc")]
            let _handoff = this._captures.enter();
            #[cfg(feature = "no-gc")]
            if !this.root && !this.owned_result {
                this.result_lease.borrow().publish_return();
            }
            if !this.root {
                this.state = MutatorState::fresh();
            }
            this.release_stack();
        }
        result
    }
}

impl FiberTask {
    fn cancel(&mut self) {
        if self.co.is_some() {
            self.state.swap_with_current();
            self.unwind();
            self.roots_guard.take();
            self.state.swap_with_current();
            if !self.root {
                self.state = MutatorState::fresh();
            }
            self.release_stack();
        }
    }
}

impl Drop for FiberTask {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// A completed result kept precisely rooted while queued in a JoinHandle.
/// Consume it immediately before evaluating or rooting the returned value.
pub struct FiberResult {
    result: Option<EvalResult>,
    state: MutatorState,
    guard: Option<cljrs_runtime::env::gc_roots::ValueRootGuard>,
    _roots: Box<[Value]>,
    #[cfg(feature = "no-gc")]
    leases: Vec<cljrs_gc::region::RegionLease>,
}

impl FiberResult {
    fn new(result: EvalResult) -> Self {
        let roots: Box<[Value]> = match &result {
            Ok(value) | Err(EvalError::Thrown(value)) => vec![value.clone()].into_boxed_slice(),
            Err(EvalError::Recur(values)) => values.clone().into_boxed_slice(),
            _ => Box::new([]),
        };
        #[cfg(feature = "no-gc")]
        let leases = roots
            .iter()
            .map(cljrs_gc::region::RegionLease::for_value)
            .collect();
        let mut state = MutatorState::fresh();
        state.swap_with_current();
        let guard = root_values(&roots);
        state.swap_with_current();
        Self {
            result: Some(result),
            state,
            guard: Some(guard),
            _roots: roots,
            #[cfg(feature = "no-gc")]
            leases,
        }
    }

    pub fn into_result(mut self) -> EvalResult {
        #[cfg(feature = "no-gc")]
        for lease in &self.leases {
            lease.publish_return();
        }
        self.result.take().unwrap()
    }
}

impl Drop for FiberResult {
    fn drop(&mut self) {
        self.state.swap_with_current();
        self.guard.take();
        self.state.swap_with_current();
    }
}

/// Spawn a concurrent fiber on the current LocalSet.
pub fn spawn(
    body: impl FnOnce() -> EvalResult + 'static,
) -> Result<tokio::task::JoinHandle<FiberResult>, EvalError> {
    spawn_rooted(Vec::new(), body)
}

/// Spawn with precise roots for values captured by the opaque Rust closure.
pub fn spawn_rooted(
    roots: Vec<Value>,
    body: impl FnOnce() -> EvalResult + 'static,
) -> Result<tokio::task::JoinHandle<FiberResult>, EvalError> {
    let task = Fiber::with_roots(roots, body)?.into_task();
    #[cfg(feature = "no-gc")]
    let allocation = task.allocation.clone();
    Ok(tokio::task::spawn_local(async move {
        #[cfg(feature = "no-gc")]
        let result = {
            let mut task = task;
            task.owned_result = true;
            let result = std::future::poll_fn(|cx| {
                let _context = allocation.enter();
                Pin::new(&mut task).poll(cx)
            })
            .await;
            // Capture result storage while the completed task still pins its
            // original scopes. The JoinHandle can outlive those lexical scopes.
            FiberResult::new(result)
        };
        #[cfg(not(feature = "no-gc"))]
        let result = FiberResult::new(task.await);
        result
    }))
}

/// Drive a borrowed top-level computation on a 64 MiB root stack.
/// Transfers the host's mutator state into the root and restores it afterward.
/// `carrier` retains executor task roots between drives on the same LocalSet.
/// Call outside the Tokio runtime and outside any fiber.
pub fn run_root(
    rt: &tokio::runtime::Runtime,
    local: &tokio::task::LocalSet,
    carrier: &mut MutatorState,
    body: impl FnOnce() -> EvalResult,
) -> EvalResult {
    assert!(current().is_none(), "nested root fiber");
    // The root and every task it spawns share this carrier's report registry.
    carrier.swap_with_current();
    let reports = crate::channel::FailureReports::current();
    carrier.swap_with_current();
    struct RestoreReports(crate::channel::FailureReports);
    impl Drop for RestoreReports {
        fn drop(&mut self) {
            self.0.install();
        }
    }
    let _reports = RestoreReports(reports.install());
    // SAFETY: the task never escapes this synchronous function. Drive drops
    // (and unwinds) it on every exit, including executor panics.
    let mut task =
        unsafe { Fiber::with_stack_bytes(Vec::new(), body, 64 * 1024 * 1024)? }.into_task();
    task.state.swap_with_current();
    task.roots_guard.take();
    task.state.swap_with_current();
    task.state = MutatorState::save();
    task.root = true;
    carrier.swap_with_current();
    // Restore both states even if the executor panics. Pending legacy async
    // tasks retain their carrier roots separately from the host's LIFO guards.
    struct Drive<'a> {
        task: FiberTask,
        carrier: &'a mut MutatorState,
    }
    impl Drop for Drive<'_> {
        fn drop(&mut self) {
            self.task.cancel();
            self.carrier.swap_with_current();
            self.task.state.swap_with_current();
        }
    }
    let mut drive = Drive { task, carrier };
    let result = local.block_on(rt, &mut drive.task);
    #[cfg(feature = "no-gc")]
    let ownership = drive.task.result_lease.borrow().clone();
    drop(drive);
    #[cfg(feature = "no-gc")]
    ownership.publish_return();
    result
}

/// Destroy local tasks with their carrier state installed, restoring the caller
/// even if task destruction unwinds.
pub fn drop_local_with_carrier(local: tokio::task::LocalSet, carrier: &mut MutatorState) {
    carrier.swap_with_current();
    struct Restore<'a>(&'a mut MutatorState);
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            self.0.swap_with_current();
        }
    }
    let _restore = Restore(carrier);
    drop(local);
    crate::channel::report_unobserved_thread_failures();
}

/// Drive one root and destroy its executor with the matching carrier state.
pub fn run_root_owned(
    rt: &tokio::runtime::Runtime,
    local: tokio::task::LocalSet,
    body: impl FnOnce() -> EvalResult,
) -> EvalResult {
    struct Driver {
        local: Option<tokio::task::LocalSet>,
        carrier: MutatorState,
    }
    impl Drop for Driver {
        fn drop(&mut self) {
            drop_local_with_carrier(self.local.take().unwrap(), &mut self.carrier);
        }
    }
    let mut driver = Driver {
        local: Some(local),
        carrier: MutatorState::fresh(),
    };
    run_root(
        rt,
        driver.local.as_ref().unwrap(),
        &mut driver.carrier,
        body,
    )
}
