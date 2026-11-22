//! Differential replay of *execution histories* across tier configurations
//! (issue #510).
//!
//! The same program can run through the tree walker, the IR interpreter and
//! native code at different points in its life, so whether it is correct
//! depends on the order things happened in — which function got warm first,
//! whether a var was rebound before or after a worker snapshotted it, whether
//! an old function value was kept and called later — not only on whether each
//! construct works in isolation.  A test here is therefore a short [`History`]
//! of [`Op`]s, not one expression, and the harness replays the whole history
//! under every [`Config`] it is given.
//!
//! Two checks apply to each replay:
//!
//! - **Explicit expectations.**  [`Op::Expect`] states the result the
//!   language specifies.  Agreement with the tree walker alone is not a
//!   correctness oracle: all tiers can share a bug.
//! - **Differential agreement.**  Every [`Op::Observe`] and [`Op::Expect`]
//!   records an [`Outcome`]; the transcripts of all configurations must be
//!   identical, so a tier that changes a result the history did not pin down
//!   still fails.
//!
//! ## Determinism
//!
//! Nothing waits on a sleep.  [`Op::Settle`] blocks until the background
//! workers report idle (`tiered::sync::wait_idle`), and the pause ops park the
//! lowering worker at a named `tiered::sync::Phase` so a history can rebind a
//! var *between* two worker steps.  Every wait is bounded; a timeout panics
//! with the step, the op and the last worker phase.
//!
//! [`Op::AssertPromoted`] guards against vacuous passes: if a history claims
//! to exercise IR (or native code) and the function never got there, the
//! history fails rather than silently comparing the tree walker to itself.
//!
//! ## Threads and isolation
//!
//! Runtime values are `!Send` and the tiering workers are process-wide, so
//! all replays in a test binary run on one dedicated thread that owns one
//! runtime per configuration; test threads send it a history and wait for the
//! verdict.  That also serializes histories, which pausing a process-wide
//! worker requires.  Each replay gets a namespace no other replay used.

#![allow(dead_code)] // each test binary uses a different subset

use std::collections::HashMap;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_runtime::tiered::sync::{self, Phase};
use cljrs_runtime::{ExecutionMode, Runtime};
use cljrs_value::Value;

/// How long any single synchronization step may take.
const STEP_TIMEOUT: Duration = Duration::from_secs(60);

/// Calls per stage of [`Op::Promote`].  Each stage crosses one tier boundary
/// at the thresholds [`init_thresholds`] sets; the count is the same in every
/// configuration so call-counting side effects stay comparable.
pub const PROMOTE_CALLS: usize = 6;

/// The tier warm-up thresholds every history binary runs under.
pub fn init_thresholds() {
    cljrs_runtime::tiered::set_ir_threshold(2);
    cljrs_runtime::tiered::set_jit_threshold(2);
}

/// The furthest tier a configuration promises to promote a function to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Everything is tree-walked.
    TreeWalk,
    /// Promoted functions have IR in the cache.
    Ir,
    /// Promoted functions have native code published.
    Native,
}

/// One execution policy a history is replayed under.
#[derive(Clone, Copy)]
pub struct Config {
    pub name: &'static str,
    pub mode: ExecutionMode,
    /// Whether functions are lowered by the background worker (so the pause
    /// ops have something to pause).  False for tree-walk and eager lowering.
    pub background: bool,
    pub reach: Reach,
    /// Extra set-up on the freshly built runtime, e.g. attaching a JIT.
    pub setup: fn(&Runtime),
}

fn no_setup(_: &Runtime) {}

/// Tree walk only: the baseline.
pub const TREE_WALK: Config = Config {
    name: "tree-walk",
    mode: ExecutionMode::TreeWalk,
    background: false,
    reach: Reach::TreeWalk,
    setup: no_setup,
};

/// Background lowering to the IR interpreter; native JIT suppressed.
pub const IR_BACKGROUND: Config = Config {
    name: "ir-background",
    mode: ExecutionMode::TieredNoJit,
    background: true,
    reach: Reach::Ir,
    setup: no_setup,
};

/// The default tiered path, with no JIT backend linked: behaves as
/// [`IR_BACKGROUND`] but through `ExecutionMode::Tiered`'s dispatch.
pub const TIERED_DEFAULT: Config = Config {
    name: "tiered-default",
    mode: ExecutionMode::Tiered,
    background: true,
    reach: Reach::Ir,
    setup: no_setup,
};

/// Eager lowering at definition time.  The caller must have called
/// `cljrs_runtime::tiered::force_eager_lowering()`, which is process-wide —
/// hence its own test binary.
pub const IR_EAGER: Config = Config {
    name: "ir-eager",
    mode: ExecutionMode::TieredNoJit,
    background: false,
    reach: Reach::Ir,
    setup: no_setup,
};

/// What a replay expects an expression to produce.
#[derive(Debug, Clone, Copy)]
pub enum Want {
    /// `pr-str` of the value.
    Value(&'static str),
    /// An error whose message contains this text.
    Error(&'static str),
}

/// One step of a history.  Source text is Clojure, evaluated in the
/// history's own namespace.
#[derive(Debug, Clone, Copy)]
pub enum Op {
    /// Evaluate for effect: define, rebind, capture (`(def old f)`), …
    Eval(&'static str),
    /// Evaluate a rebind that must invalidate IR specialized against the old
    /// binding (checked in every configuration that lowers).
    Rebind(&'static str),
    /// Evaluate `form` this many times without waiting for the workers —
    /// enough to enqueue work, which a following pause op can intercept.
    Call(&'static str, usize),
    /// Drive `form` across every tier boundary: [`PROMOTE_CALLS`] calls,
    /// settle, [`PROMOTE_CALLS`] more, settle.
    Promote(&'static str),
    /// Wait until no background lowering or compilation is pending.
    Settle,
    /// Arm `phase`: the next lowering request to reach it parks.
    PauseAt(Phase),
    /// Wait until the worker is parked at `phase`.
    AwaitPaused(Phase),
    /// Let a parked worker continue.
    Release,
    /// Record the outcome of `form` for cross-configuration comparison.
    Observe(&'static str),
    /// Record the outcome of `form` and check it against the specification.
    Expect(&'static str, Want),
    /// `form` evaluates to a function: every arity must have reached this
    /// configuration's [`Reach`].
    AssertPromoted(&'static str),
}

/// A named sequence of operations.
#[derive(Debug, Clone, Copy)]
pub struct History {
    pub name: &'static str,
    pub ops: &'static [Op],
}

/// What an observed expression did.  Errors compare equal regardless of
/// message — tiers phrase the same failure differently — so only
/// [`Want::Error`] inspects the text.
#[derive(Debug, Clone)]
pub enum Outcome {
    Value(String),
    Error(String),
}

impl PartialEq for Outcome {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Outcome::Value(a), Outcome::Value(b)) => a == b,
            (Outcome::Error(_), Outcome::Error(_)) => true,
            _ => false,
        }
    }
}

/// `(step, form, outcome)` for every observing op, in order.
type Transcript = Vec<(usize, &'static str, Outcome)>;

// ── The replay thread ────────────────────────────────────────────────────────

type Job = (History, Vec<Config>, Sender<Result<(), String>>);

fn replay_thread() -> &'static Mutex<Sender<Job>> {
    static TX: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("history-replay".into())
            // Tree walking a deep bootstrap wants more than the default.
            .stack_size(64 << 20)
            .spawn(move || {
                let _mutator = cljrs_gc::register_mutator();
                let mut runtimes: HashMap<&'static str, Arc<GlobalEnv>> = HashMap::new();
                let mut next_ns = 0u64;
                for (history, configs, reply) in rx {
                    let verdict = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        check(&history, &configs, &mut runtimes, &mut next_ns)
                    }))
                    .unwrap_or_else(|p| {
                        // Never leave the worker parked for the next history.
                        sync::release();
                        Err(panic_text(&p))
                    });
                    let _ = reply.send(verdict);
                }
            })
            .expect("spawn replay thread");
        Mutex::new(tx)
    })
}

fn panic_text(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "non-string panic".into())
}

/// Replay `history` under each of `configs`, panicking with a full report on
/// any expectation failure, synchronization timeout or disagreement.
pub fn run(history: History, configs: &[Config]) {
    let (reply_tx, reply_rx) = channel();
    replay_thread()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .send((history, configs.to_vec(), reply_tx))
        .expect("replay thread alive");
    if let Err(report) = reply_rx.recv().expect("replay thread replied") {
        panic!("history `{}`:\n{report}", history.name);
    }
}

fn check(
    history: &History,
    configs: &[Config],
    runtimes: &mut HashMap<&'static str, Arc<GlobalEnv>>,
    next_ns: &mut u64,
) -> Result<(), String> {
    let mut transcripts: Vec<(&'static str, Transcript)> = Vec::new();
    for config in configs {
        let globals = runtimes
            .entry(config.name)
            .or_insert_with(|| {
                let runtime = Runtime::builder()
                    .execution_mode(config.mode)
                    .build()
                    .expect("runtime");
                (config.setup)(&runtime);
                runtime.into_globals()
            })
            .clone();
        *next_ns += 1;
        let ns = format!("history-{}-{}", history.name, *next_ns);
        globals.get_or_create_ns(&ns);
        globals.refer_core(&ns);
        let mut env = Env::new(globals.clone(), &ns);
        let transcript = replay(history, config, &globals, &mut env)
            .map_err(|e| format!("[{}] {e}", config.name))?;
        transcripts.push((config.name, transcript));
    }

    let Some(((base_name, base), rest)) = transcripts.split_first() else {
        return Ok(());
    };
    for (name, transcript) in rest {
        if transcript != base {
            return Err(format!(
                "configurations disagree\n  {base_name}: {}\n  {name}: {}",
                show(base),
                show(transcript)
            ));
        }
    }
    Ok(())
}

fn show(t: &Transcript) -> String {
    t.iter()
        .map(|(step, form, outcome)| format!("\n    #{step} {form} => {outcome:?}"))
        .collect()
}

fn eval_src(env: &mut Env, src: &str) -> Result<Value, String> {
    let forms = Parser::new(src.to_string(), "<history>".to_string())
        .parse_all()
        .map_err(|e| format!("parse {src:?}: {e:?}"))?;
    let mut last = Value::Nil;
    for form in &forms {
        last = cljrs_runtime::tiered::eval(form, env).map_err(|e| format!("{e}"))?;
    }
    Ok(last)
}

fn outcome(env: &mut Env, form: &str) -> Outcome {
    match eval_src(env, &format!("(pr-str {form})")) {
        Ok(Value::Str(s)) => Outcome::Value(s.get().clone()),
        Ok(other) => Outcome::Error(format!("pr-str returned non-string {other:?}")),
        Err(e) => Outcome::Error(e),
    }
}

fn replay(
    history: &History,
    config: &Config,
    globals: &Arc<GlobalEnv>,
    env: &mut Env,
) -> Result<Transcript, String> {
    let mut transcript = Transcript::new();
    for (step, op) in history.ops.iter().enumerate() {
        let fail = |what: String| format!("step #{step} {op:?}: {what}{}", show(&transcript));
        match *op {
            Op::Eval(src) => {
                eval_src(env, src).map_err(fail)?;
            }
            Op::Rebind(src) => {
                let before = sync::invalidations();
                eval_src(env, src).map_err(fail)?;
                if config.reach != Reach::TreeWalk && sync::invalidations() == before {
                    return Err(fail("no optimized dependent was invalidated".into()));
                }
            }
            Op::Call(form, n) => {
                for _ in 0..n {
                    eval_src(env, form).map_err(fail)?;
                }
            }
            Op::Promote(form) => {
                for _ in 0..2 {
                    for _ in 0..PROMOTE_CALLS {
                        eval_src(env, form).map_err(fail)?;
                    }
                    sync::wait_idle(STEP_TIMEOUT).map_err(|e| fail(e.to_string()))?;
                }
            }
            Op::Settle => sync::wait_idle(STEP_TIMEOUT).map_err(|e| fail(e.to_string()))?,
            Op::PauseAt(phase) if config.background => sync::pause_at(phase),
            Op::AwaitPaused(phase) if config.background => {
                sync::wait_paused(phase, STEP_TIMEOUT).map_err(|e| fail(e.to_string()))?
            }
            Op::Release if config.background => sync::release(),
            // Nothing lowers in the background here: the pause would never
            // be reached, and the history's other ops still run in order.
            Op::PauseAt(_) | Op::AwaitPaused(_) | Op::Release => {}
            Op::Observe(form) => transcript.push((step, form, outcome(env, form))),
            Op::Expect(form, want) => {
                let got = outcome(env, form);
                let ok = match (&want, &got) {
                    (Want::Value(w), Outcome::Value(g)) => w == g,
                    (Want::Error(w), Outcome::Error(g)) => g.contains(w),
                    _ => false,
                };
                if !ok {
                    let msg = format!("expected {want:?}, got {got:?}");
                    return Err(fail(msg));
                }
                transcript.push((step, form, got));
            }
            Op::AssertPromoted(form) => {
                let Value::Fn(f) = eval_src(env, form).map_err(fail)? else {
                    return Err(fail(format!("{form} is not a function")));
                };
                for arity in &f.get().arities {
                    let id = arity.ir_arity_id;
                    let reached = match config.reach {
                        Reach::TreeWalk => true,
                        Reach::Ir => globals.ir_cache().get(id).is_some(),
                        Reach::Native => globals.jit().get_native_fn(id).is_some(),
                    };
                    if !reached {
                        return Err(fail(format!(
                            "arity {} of {form} never reached {:?}",
                            arity.params.len(),
                            config.reach
                        )));
                    }
                }
            }
        }
    }
    sync::release();
    sync::wait_idle(STEP_TIMEOUT).map_err(|e| format!("after the last step: {e}"))?;
    Ok(transcript)
}

// ── The deterministic matrix ────────────────────────────────────────────────

use Op::*;
use Want::Value as V;

/// Issue #505: a retained old function is warmed after its var was rebound;
/// a consumer of the var must keep seeing the *new* binding once lowered.
pub const RETAINED_OLD_WARMED_AFTER_REBIND: History = History {
    name: "retained-old-warmed-after-rebind",
    ops: &[
        Eval("(defn leaf [x] [x])"),
        Eval("(def old-leaf leaf)"),
        Eval("(defn leaf [x] [x x])"),
        Promote("(leaf 7)"),
        Promote("(old-leaf 7)"),
        AssertPromoted("old-leaf"),
        Eval("(defn consumer [x] (count (leaf x)))"),
        Expect("(consumer 7)", V("2")),
        Promote("(consumer 7)"),
        AssertPromoted("consumer"),
        Expect("(consumer 7)", V("2")),
        Expect("(old-leaf 7)", V("[7]")),
    ],
};

/// The reverse ordering: the consumer is optimized first, *then* the
/// retained old implementation gets hot.  Neither may disturb the other.
pub const RETAINED_OLD_WARMED_AFTER_CONSUMER: History = History {
    name: "retained-old-warmed-after-consumer",
    ops: &[
        Eval("(defn leaf [x] [x])"),
        Eval("(def old-leaf leaf)"),
        Eval("(defn leaf [x] [x x])"),
        Eval("(defn consumer [x] (count (leaf x)))"),
        Promote("(leaf 7)"),
        Promote("(consumer 7)"),
        AssertPromoted("consumer"),
        Promote("(old-leaf 7)"),
        Expect("(consumer 7)", V("2")),
        Expect("(old-leaf 7)", V("[7]")),
        // A retained *consumer* is a closure over the var, not the value:
        // it sees later rebinds too.
        Eval("(def old-consumer consumer)"),
        Eval("(defn leaf [x] [x x x])"),
        Expect("(old-consumer 7)", V("3")),
        Promote("(old-consumer 7)"),
        Expect("(old-consumer 7)", V("3")),
        Expect("(consumer 7)", V("3")),
    ],
};

/// The worker has snapshotted `leaf` while lowering `consumer` but not yet
/// published; `leaf` is rebound in that window.  The published IR must not
/// use the stale callee.
pub const REBIND_BETWEEN_SNAPSHOT_AND_PUBLISH: History = History {
    name: "rebind-between-snapshot-and-publish",
    ops: &[
        Eval("(defn leaf [x] [x])"),
        Promote("(leaf 7)"),
        AssertPromoted("leaf"),
        Eval("(defn consumer [x] (count (leaf x)))"),
        PauseAt(Phase::Lowered),
        Call("(consumer 7)", PROMOTE_CALLS),
        AwaitPaused(Phase::Lowered),
        Eval("(defn leaf [x] [x x x])"),
        Release,
        Settle,
        Expect("(consumer 7)", V("3")),
        Promote("(consumer 7)"),
        AssertPromoted("consumer"),
        Expect("(consumer 7)", V("3")),
    ],
};

/// `leaf`'s own lowering request is queued when `leaf` is rebound: the old
/// body's IR may be published for the old value, but must never be
/// registered as the implementation a consumer specializes against.
pub const REBIND_WHILE_QUEUED: History = History {
    name: "rebind-while-queued",
    ops: &[
        Eval("(defn leaf [x] [x])"),
        PauseAt(Phase::Dequeued),
        Call("(leaf 7)", PROMOTE_CALLS),
        AwaitPaused(Phase::Dequeued),
        Eval("(defn leaf [x] [x x])"),
        Release,
        Settle,
        Eval("(defn consumer [x] (count (leaf x)))"),
        Promote("(leaf 7)"),
        Promote("(consumer 7)"),
        AssertPromoted("consumer"),
        Expect("(consumer 7)", V("2")),
    ],
};

/// A rebind lands after the worker published `consumer`'s IR but before it
/// registered `consumer` for cross-defn use.
pub const REBIND_BETWEEN_PUBLISH_AND_REGISTER: History = History {
    name: "rebind-between-publish-and-register",
    ops: &[
        Eval("(defn leaf [x] [x])"),
        Promote("(leaf 7)"),
        Eval("(defn consumer [x] (count (leaf x)))"),
        Eval("(defn outer [x] (inc (consumer x)))"),
        PauseAt(Phase::Published),
        Call("(consumer 7)", PROMOTE_CALLS),
        AwaitPaused(Phase::Published),
        Eval("(defn leaf [x] [x x x x])"),
        Release,
        Settle,
        Expect("(consumer 7)", V("4")),
        Promote("(consumer 7)"),
        Promote("(outer 7)"),
        AssertPromoted("outer"),
        Expect("(outer 7)", V("5")),
    ],
};

/// Issue #506: region specialization composed with an already-specialized
/// callee, promoted bottom-up so each layer is optimized against an
/// optimized body.
pub const NESTED_REGION_BOTTOM_UP: History = History {
    name: "nested-region-bottom-up",
    ops: &[
        Eval("(defn leaf [x] [x])"),
        Eval("(defn middle [x] [(count (leaf x))])"),
        Eval("(defn outer [x] (count (middle x)))"),
        Promote("(leaf 7)"),
        Promote("(middle 7)"),
        AssertPromoted("middle"),
        Promote("(outer 7)"),
        AssertPromoted("outer"),
        Expect("(outer 7)", V("1")),
        Expect("(middle 7)", V("[1]")),
    ],
};

/// The same chain promoted top-down: `outer` is lowered while its callees
/// are still cold, then the callees get hot underneath it.
pub const NESTED_REGION_TOP_DOWN: History = History {
    name: "nested-region-top-down",
    ops: &[
        Eval("(defn leaf [x] [x (inc x)])"),
        Eval("(defn middle [x] [(count (leaf x)) (leaf x)])"),
        Eval("(defn outer [x] (let [[n v] (middle x)] (+ n (count v))))"),
        Promote("(outer 7)"),
        AssertPromoted("outer"),
        Promote("(middle 7)"),
        Promote("(leaf 7)"),
        Expect("(outer 7)", V("4")),
        // Re-lower `outer` against the now-optimized callees.
        Eval("(defn outer [x] (let [[n v] (middle x)] (* n (count v))))"),
        Promote("(outer 7)"),
        AssertPromoted("outer"),
        Expect("(outer 7)", V("4")),
        Expect("(middle 7)", V("[2 [7 8]]")),
    ],
};

/// Rebinding the innermost function of an optimized chain invalidates every
/// layer that specialized against it, immediately and after re-promotion.
pub const REBIND_LEAF_OF_OPTIMIZED_CHAIN: History = History {
    name: "rebind-leaf-of-optimized-chain",
    ops: &[
        Eval("(defn leaf [x] [x])"),
        Eval("(defn middle [x] [(count (leaf x))])"),
        Eval("(defn outer [x] (first (middle x)))"),
        Promote("(leaf 7)"),
        Promote("(middle 7)"),
        Promote("(outer 7)"),
        AssertPromoted("outer"),
        Expect("(outer 7)", V("1")),
        Rebind("(defn leaf [x] [x x x])"),
        Expect("(outer 7)", V("3")),
        Promote("(outer 7)"),
        Expect("(outer 7)", V("3")),
        Promote("(leaf 7)"),
        Promote("(middle 7)"),
        Promote("(outer 7)"),
        Expect("(outer 7)", V("3")),
    ],
};

/// Every call of an effectful callee must happen exactly once per call of
/// the consumer, whichever tier runs it — inlining or region promotion must
/// neither duplicate nor drop the effect.
pub const EFFECTS_COUNTED_ACROSS_TIERS: History = History {
    name: "effects-counted-across-tiers",
    ops: &[
        Eval("(def calls (atom 0))"),
        Eval("(defn leaf [x] (swap! calls inc) [x])"),
        Eval("(defn consumer [x] (+ (count (leaf x)) (count (leaf x))))"),
        Promote("(leaf 1)"),
        Promote("(consumer 1)"),
        AssertPromoted("consumer"),
        Expect("(consumer 1)", V("2")),
        Expect("@calls", V("38")),
        Rebind("(defn leaf [x] (swap! calls + 10) [x])"),
        Expect("(consumer 1)", V("2")),
        Expect("@calls", V("58")),
    ],
};

/// A rebind that changes a callee's shape — a different arity, a non-fn
/// value — turns an optimized call into the error the tree walker reports,
/// and back again.
pub const REBIND_CHANGES_CALLEE_SHAPE: History = History {
    name: "rebind-changes-callee-shape",
    ops: &[
        Eval("(defn leaf [x] [x])"),
        Eval("(defn consumer [x] (count (leaf x)))"),
        Promote("(leaf 7)"),
        Promote("(consumer 7)"),
        AssertPromoted("consumer"),
        Eval("(defn leaf [x y] [x y])"),
        Expect("(consumer 7)", Want::Error("")),
        Eval("(def leaf 5)"),
        Expect("(consumer 7)", Want::Error("")),
        Eval("(defn leaf [x] (throw (ex-info \"boom\" {:x x})))"),
        Expect("(consumer 7)", Want::Error("boom")),
        Eval("(defn leaf [x] (list x x))"),
        Expect("(consumer 7)", V("2")),
        Promote("(consumer 7)"),
        Expect("(consumer 7)", V("2")),
    ],
};

/// A function value retained *inside a data structure* and called through
/// it, after the var it came from is rebound and the consumer is hot.
pub const RETAINED_IN_COLLECTION: History = History {
    name: "retained-in-collection",
    ops: &[
        Eval("(defn leaf [x] [x])"),
        Eval("(def table {:old leaf})"),
        Eval("(defn leaf [x] [x x])"),
        Eval("(defn via-table [x] (count ((:old table) x)))"),
        Eval("(defn via-var [x] (count (leaf x)))"),
        Promote("((:old table) 7)"),
        Promote("(via-table 7)"),
        Promote("(via-var 7)"),
        AssertPromoted("via-var"),
        Expect("(via-table 7)", V("1")),
        Expect("(via-var 7)", V("2")),
    ],
};

/// Every history in the deterministic matrix.
pub const ALL: &[History] = &[
    RETAINED_OLD_WARMED_AFTER_REBIND,
    RETAINED_OLD_WARMED_AFTER_CONSUMER,
    REBIND_BETWEEN_SNAPSHOT_AND_PUBLISH,
    REBIND_WHILE_QUEUED,
    REBIND_BETWEEN_PUBLISH_AND_REGISTER,
    NESTED_REGION_BOTTOM_UP,
    NESTED_REGION_TOP_DOWN,
    REBIND_LEAF_OF_OPTIMIZED_CHAIN,
    EFFECTS_COUNTED_ACROSS_TIERS,
    REBIND_CHANGES_CALLEE_SHAPE,
    RETAINED_IN_COLLECTION,
];
