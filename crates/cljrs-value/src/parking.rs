//! Optional stackful parking hooks. Core crates do not depend on an executor.
//!
//! Hooks run only on a fiber; ordinary callers retain blocking behavior.
use std::any::Any;
use std::future::Future;
use std::pin::{Pin, pin};
use std::sync::{Condvar, Mutex, OnceLock};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

/// Private unwind payload for infallible Rust parking APIs.
#[derive(Debug)]
struct Cancelled;

/// Stop an infallible parking call. Native and ABI boundaries must use
/// `catch_cancelled` before returning through foreign frames.
pub fn resume_cancelled() -> ! {
    std::panic::resume_unwind(Box::new(Cancelled))
}

/// Convert only a parking cancellation into a typed error. Preserve all other
/// panic payloads, including coroutine forced unwinds.
pub fn catch_cancelled<T>(operation: impl FnOnce() -> T) -> crate::ValueResult<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        Ok(value) => Ok(value),
        Err(payload) if payload.is::<Cancelled>() => Err(crate::ValueError::Cancelled),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

pub type BlockingResult = Box<dyn Any + Send>;
pub type BlockingJob = Box<dyn FnOnce() -> BlockingResult + Send>;

pub struct Hooks {
    pub current: fn() -> Option<u64>,
    pub park: fn(Pin<&mut dyn Future<Output = ()>>, Option<Duration>) -> bool,
    pub sleep: fn(Duration),
    pub blocking: fn(BlockingJob) -> BlockingResult,
}
static HOOKS: OnceLock<Hooks> = OnceLock::new();

pub fn install(hooks: Hooks) {
    let _ = HOOKS.set(hooks);
}
pub fn current() -> Option<u64> {
    HOOKS.get().and_then(|h| (h.current)())
}

/// Run Send-only native work off the isolate while a fiber waits. Conversion
/// from/to managed Values must happen on the caller's thread, outside `job`.
pub fn blocking<F, T>(job: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    if let Some(h) = HOOKS.get().filter(|_| current().is_some()) {
        *(h.blocking)(Box::new(move || Box::new(job())))
            .downcast::<T>()
            .unwrap()
    } else {
        job()
    }
}
pub fn sleep(duration: Duration) {
    if let Some(h) = HOOKS.get().filter(|_| current().is_some()) {
        (h.sleep)(duration);
    } else {
        std::thread::sleep(duration);
    }
}

/// Park a fiber on a borrowed future. Panics when called outside a fiber.
pub fn park<F: Future>(future: F) -> F::Output {
    let mut result = None;
    {
        let wait = pin!(async { result = Some(future.await) });
        assert!(current().is_some(), "parking outside a fiber");
        (HOOKS.get().unwrap().park)(wait, None);
    }
    result.unwrap()
}

#[derive(Debug, Default)]
struct State {
    generation: u64,
    next_id: u64,
    wakers: Vec<(u64, Waker)>,
}

/// Broadcast event with race-free registration and cancellation cleanup.
/// Create a listener BEFORE checking the protected condition, then release
/// the condition's lock before waiting. No runtime lock may cross a park.
#[derive(Debug, Default)]
pub struct Event {
    state: Mutex<State>,
    cond: Condvar,
}
impl Event {
    pub fn listen(&self) -> Listener<'_> {
        let mut state = self.state.lock().unwrap();
        let id = state.next_id;
        state.next_id = state.next_id.wrapping_add(1);
        Listener {
            event: self,
            generation: state.generation,
            id,
        }
    }
    pub fn notify_all(&self) {
        let wakers = {
            let mut state = self.state.lock().unwrap();
            state.generation = state.generation.wrapping_add(1);
            std::mem::take(&mut state.wakers)
        };
        self.cond.notify_all();
        for (_, waker) in wakers {
            waker.wake();
        }
    }
}
pub struct Listener<'a> {
    event: &'a Event,
    generation: u64,
    id: u64,
}
impl Listener<'_> {
    /// True on notification; false on timeout. Handles spurious wakeups.
    pub fn wait(mut self, timeout: Option<Duration>) -> bool {
        if let Some(h) = HOOKS.get().filter(|_| current().is_some()) {
            return (h.park)(
                Pin::new(&mut self) as Pin<&mut dyn Future<Output = ()>>,
                timeout,
            );
        }
        let deadline = timeout.map(|d| Instant::now() + d);
        let mut state = self.event.state.lock().unwrap();
        while state.generation == self.generation {
            if let Some(deadline) = deadline {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return false;
                }
                state = self.event.cond.wait_timeout(state, remaining).unwrap().0;
            } else {
                state = self.event.cond.wait(state).unwrap();
            }
        }
        true
    }
}
impl Future for Listener<'_> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut state = self.event.state.lock().unwrap();
        if state.generation != self.generation {
            return Poll::Ready(());
        }
        if let Some((_, w)) = state.wakers.iter_mut().find(|(id, _)| *id == self.id) {
            w.clone_from(cx.waker());
        } else {
            state.wakers.push((self.id, cx.waker().clone()));
        }
        Poll::Pending
    }
}
impl Drop for Listener<'_> {
    fn drop(&mut self) {
        self.event
            .state
            .lock()
            .unwrap()
            .wakers
            .retain(|(id, _)| *id != self.id);
    }
}

/// Drive a notification future synchronously, parking the current fiber when
/// present. The fallback needs no Tokio runtime; futures using a reactor must
/// supply that reactor themselves.
pub fn block_on<F: Future>(future: F) -> F::Output {
    if current().is_some() {
        return park(future);
    }
    struct ThreadWake(std::thread::Thread);
    impl std::task::Wake for ThreadWake {
        fn wake(self: std::sync::Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &std::sync::Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(std::sync::Arc::new(ThreadWake(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}
