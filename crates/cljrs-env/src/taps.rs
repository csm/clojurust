use crate::callback::{capture_eval_context, install_eval_context};
use crate::dynamics;
use crate::env::GlobalEnv;
use cljrs_value::Value;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

const TAP_QUEUE_CAPACITY: usize = 1024;

pub struct TapState {
    fns: Vec<Value>,
    queue: VecDeque<Value>,
    draining: bool,
}

pub static TAP: std::sync::LazyLock<(Mutex<TapState>, Condvar)> = std::sync::LazyLock::new(|| {
    (
        Mutex::new(TapState {
            fns: Vec::new(),
            queue: VecDeque::new(),
            draining: false,
        }),
        Condvar::new(),
    )
});

/// Drain loop: runs on a background thread, delivers queued values to tap fns.
fn tap_drain_loop() {
    loop {
        let (val, fns) = {
            let (lock, cvar) = &*TAP;
            let mut state = lock.lock().unwrap();
            while state.queue.is_empty() {
                if state.fns.is_empty() {
                    state.draining = false;
                    return;
                }
                state = cvar.wait(state).unwrap();
                if state.queue.is_empty() && state.fns.is_empty() {
                    state.draining = false;
                    return;
                }
            }
            let val = state.queue.pop_front().unwrap();
            let fns = state.fns.clone();
            (val, fns)
        };

        // Deliver outside the lock; errors are silently ignored (per Clojure spec).
        for f in &fns {
            let _ = crate::callback::invoke(f, vec![val.clone()]);
        }
    }
}

/// Spawn the drain thread with captured eval context and dynamic bindings.
fn spawn_drain_thread(globals: Arc<GlobalEnv>, ns: Arc<str>) {
    let bindings = dynamics::capture_current();
    thread::Builder::new()
        .name("tap-drain".into())
        .spawn(move || {
            install_eval_context(globals, ns);
            dynamics::install_frames(bindings);
            tap_drain_loop();
        })
        .expect("failed to spawn tap drain thread");
}

/// Ensure the drain thread is running (called under lock).
/// Returns whether a new thread needs to be spawned (caller spawns after releasing lock).
fn needs_drain(state: &TapState) -> bool {
    !state.draining && !state.queue.is_empty() && !state.fns.is_empty()
}

pub fn add_tap(f: Value) {
    let should_spawn = {
        let (lock, _) = &*TAP;
        let mut state = lock.lock().unwrap();
        if !state.fns.iter().any(|existing| existing == &f) {
            state.fns.push(f);
        }
        let spawn = needs_drain(&state);
        if spawn {
            state.draining = true;
        }
        spawn
    };
    if should_spawn {
        if let Some((globals, ns)) = capture_eval_context() {
            spawn_drain_thread(globals, ns);
        }
        let (_, cvar) = &*TAP;
        cvar.notify_one();
    }
}

pub fn remove_tap(f: &Value) {
    let (lock, cvar) = &*TAP;
    let mut state = lock.lock().unwrap();
    state.fns.retain(|existing| existing != f);
    cvar.notify_one();
}

/// Trace all GcPtr values in the tap system as GC roots.
pub fn trace_roots(visitor: &mut cljrs_gc::MarkVisitor) {
    use cljrs_gc::Trace;
    let (lock, _) = &*TAP;
    let state = lock.lock().unwrap();
    for val in &state.fns {
        val.trace(visitor);
    }
    for val in &state.queue {
        val.trace(visitor);
    }
}

pub fn send(val: Value) -> bool {
    let should_spawn = {
        let (lock, _) = &*TAP;
        let mut state = lock.lock().unwrap();
        if state.fns.is_empty() {
            return false;
        }
        if state.queue.len() >= TAP_QUEUE_CAPACITY {
            return false;
        }
        state.queue.push_back(val);
        let spawn = needs_drain(&state);
        if spawn {
            state.draining = true;
        }
        spawn
    };
    if should_spawn && let Some((globals, ns)) = capture_eval_context() {
        spawn_drain_thread(globals, ns);
    }
    let (_, cvar) = &*TAP;
    cvar.notify_one();
    true
}
