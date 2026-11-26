//! Observable, controllable state for the background tiering workers.
//!
//! Promotion is asynchronous: a warm function is shipped to the
//! `cljrs-ir-lower` worker, and a hot one to a JIT backend's compile worker.
//! A test that wants to say "after the IR is published, rebind the var" or
//! "rebind while the worker holds a snapshot of the externals" cannot express
//! that with `Thread/sleep` — a sleep is a guess about the scheduler, and a
//! loaded CI machine eventually guesses wrong (issue #510).
//!
//! This module gives those orderings names:
//!
//! - **Idle tracking.**  Every request a worker accepts is counted in
//!   ([`note_enqueued`]) and out ([`note_finished`]); [`wait_idle`] blocks
//!   until nothing is queued or running on any worker.
//! - **Checkpoints.**  The lowering worker announces each [`Phase`] it reaches
//!   with [`checkpoint`].  A test arms one phase with [`pause_at`]; the next
//!   request to reach it parks there until [`release`], and [`wait_paused`]
//!   tells the test it has arrived.  The mutator is free to run (and rebind)
//!   while the worker is parked — the worker is not a GC mutator, so parking
//!   it never blocks a collection.
//! - **Invalidations.**  Each var rebind that dropped dependent IR is counted
//!   ([`invalidations`]), so a test can assert a rebind was actually seen.
//!
//! Every wait is bounded and fails with a [`SyncTimeout`] naming the last
//! phase the worker reached, so a hang reports *where* it hung.
//!
//! The state is process-wide because the workers are.  Tests that pause a
//! worker must therefore not overlap with other tests in the same process
//! that promote functions; the history harness serializes on a lock.
//!
//! In production nobody arms a pause, and the cost is one uncontended mutex
//! acquisition per request per phase — negligible beside lowering itself.

use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// A point the background lowering worker passes on the way to publishing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// The worker has taken a request off the queue and not yet lowered it.
    Dequeued,
    /// One arity has been lowered and optimized: the externals it
    /// specialized against are snapshotted, but its IR is not yet in the
    /// cache.
    Lowered,
    /// One arity's IR is in the cache and survived the rebind re-check; the
    /// function has not yet been registered for cross-defn optimization.
    Published,
}

/// A background worker whose queue [`wait_idle`] drains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Worker {
    /// The `cljrs-ir-lower` thread.
    Lower,
    /// A JIT backend's compile thread.
    Jit,
}

/// A bounded wait gave up.  Formats as a one-line report of what was being
/// waited for and how far the workers got.
#[derive(Debug, Clone)]
pub struct SyncTimeout {
    /// What the caller was waiting for.
    pub waiting_for: String,
    /// The last checkpoint any worker reached, with the function name the
    /// request carried.
    pub last_phase: Option<(Phase, Option<String>)>,
    /// Lowering requests accepted and not yet finished.
    pub pending_lower: usize,
    /// JIT compile requests accepted and not yet finished.
    pub pending_jit: usize,
}

impl std::fmt::Display for SyncTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "timed out waiting for {}; last worker phase {:?}; pending lower={} jit={}",
            self.waiting_for, self.last_phase, self.pending_lower, self.pending_jit
        )
    }
}

impl std::error::Error for SyncTimeout {}

#[derive(Default)]
struct State {
    pending_lower: usize,
    pending_jit: usize,
    last_phase: Option<(Phase, Option<String>)>,
    /// The armed phase, if any.
    pause: Option<Phase>,
    /// Set while a worker is parked at `pause`.
    parked: bool,
    invalidations: u64,
}

static STATE: Mutex<State> = Mutex::new(State {
    pending_lower: 0,
    pending_jit: 0,
    last_phase: None,
    pause: None,
    parked: false,
    invalidations: 0,
});
static CHANGED: Condvar = Condvar::new();

fn state() -> MutexGuard<'static, State> {
    // A panicking test thread must not wedge every later test.
    STATE.lock().unwrap_or_else(|p| p.into_inner())
}

fn timeout(st: &State, waiting_for: impl Into<String>) -> SyncTimeout {
    SyncTimeout {
        waiting_for: waiting_for.into(),
        last_phase: st.last_phase.clone(),
        pending_lower: st.pending_lower,
        pending_jit: st.pending_jit,
    }
}

/// Block until `done(state)` holds or `limit` elapses.
fn wait_for(
    limit: Duration,
    waiting_for: impl FnOnce() -> String,
    mut done: impl FnMut(&State) -> bool,
) -> Result<(), SyncTimeout> {
    let deadline = Instant::now() + limit;
    let mut st = state();
    while !done(&st) {
        let now = Instant::now();
        if now >= deadline {
            return Err(timeout(&st, waiting_for()));
        }
        st = CHANGED
            .wait_timeout(st, deadline - now)
            .unwrap_or_else(|p| p.into_inner())
            .0;
    }
    Ok(())
}

/// Count a request `worker` is about to accept.  Call *before* handing the
/// request over, so the worker can never finish it first; undo with
/// [`note_finished`] if the hand-off fails.
pub fn note_enqueued(worker: Worker) {
    let mut st = state();
    match worker {
        Worker::Lower => st.pending_lower += 1,
        Worker::Jit => st.pending_jit += 1,
    }
}

/// Requests `worker` has accepted and not yet finished.
pub fn pending(worker: Worker) -> usize {
    let st = state();
    match worker {
        Worker::Lower => st.pending_lower,
        Worker::Jit => st.pending_jit,
    }
}

/// Count a request `worker` has finished (or failed to accept).
pub fn note_finished(worker: Worker) {
    let mut st = state();
    let slot = match worker {
        Worker::Lower => &mut st.pending_lower,
        Worker::Jit => &mut st.pending_jit,
    };
    *slot = slot.saturating_sub(1);
    CHANGED.notify_all();
}

/// Announce that a worker reached `phase` while processing `name`, parking
/// here if a test armed this phase with [`pause_at`].
pub fn checkpoint(phase: Phase, name: Option<&str>) {
    let mut st = state();
    st.last_phase = Some((phase, name.map(str::to_owned)));
    if st.pause == Some(phase) {
        st.parked = true;
        CHANGED.notify_all();
        while st.pause == Some(phase) {
            st = CHANGED.wait(st).unwrap_or_else(|p| p.into_inner());
        }
        st.parked = false;
    }
    CHANGED.notify_all();
}

/// Count a rebind that invalidated dependent IR.
pub fn note_invalidated() {
    state().invalidations += 1;
}

/// Rebinds counted by [`note_invalidated`] so far.
pub fn invalidations() -> u64 {
    state().invalidations
}

/// Arm `phase`: the next worker to reach it parks until [`release`].
/// Replaces any earlier pause (and so releases a worker parked at it).
pub fn pause_at(phase: Phase) {
    let mut st = state();
    st.pause = Some(phase);
    CHANGED.notify_all();
}

/// Disarm the pause and let a parked worker continue.
pub fn release() {
    let mut st = state();
    st.pause = None;
    CHANGED.notify_all();
}

/// Block until a worker is parked at the armed `phase`.
pub fn wait_paused(phase: Phase, limit: Duration) -> Result<(), SyncTimeout> {
    wait_for(
        limit,
        || format!("a worker to park at {phase:?}"),
        |st| st.parked && st.pause == Some(phase),
    )
}

/// Block until no lowering or compile request is queued or running.
///
/// A worker parked at a pause is still running its request, so this times
/// out rather than returning while a pause is held.
pub fn wait_idle(limit: Duration) -> Result<(), SyncTimeout> {
    wait_for(
        limit,
        || "background workers to go idle".to_string(),
        |st| st.pending_lower == 0 && st.pending_jit == 0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // The state is process-wide, and other unit tests in this crate drive the
    // lowering worker's checkpoints; take the lock that serializes them.
    fn guard() -> std::sync::MutexGuard<'static, ()> {
        crate::tiered::tiers::SWEEP_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn pause_parks_until_release() {
        let _g = guard();
        pause_at(Phase::Lowered);
        let worker = std::thread::spawn(|| {
            note_enqueued(Worker::Lower);
            checkpoint(Phase::Dequeued, Some("f"));
            checkpoint(Phase::Lowered, Some("f"));
            note_finished(Worker::Lower);
        });
        wait_paused(Phase::Lowered, Duration::from_secs(10)).expect("parks");
        let err = wait_idle(Duration::from_millis(20)).expect_err("still running");
        assert_eq!(err.last_phase, Some((Phase::Lowered, Some("f".into()))));
        assert!(err.to_string().contains("Lowered"), "{err}");
        release();
        worker.join().unwrap();
        wait_idle(Duration::from_secs(10)).expect("idle after release");
    }

    #[test]
    fn unarmed_checkpoints_do_not_block() {
        let _g = guard();
        release();
        checkpoint(Phase::Published, None);
        let err = wait_paused(Phase::Published, Duration::from_millis(10)).expect_err("none");
        assert!(err.waiting_for.contains("Published"));
    }
}
