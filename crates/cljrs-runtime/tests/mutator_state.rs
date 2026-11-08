//! `MutatorState` save/restore between two live evaluations.
//!
//! Virtual threads (docs/virtual-threads-plan.md, Phase 0) run several
//! computations on one thread by swapping their per-mutator thread-locals at
//! a park point.  This suite plays the scheduler by hand: evaluation A calls
//! a native `park!` that detaches A's state, runs a whole second evaluation B
//! on the same thread, and reinstalls A's state before returning.  Neither
//! evaluation may observe the other's dynamic bindings, output capture or
//! eval context.
//!
//! The values crossing the switch are keywords, strings held by A's capture
//! and small longs, so no collection is needed (or safe) while A's roots are
//! detached — tracing suspended state is Phase 1.

mod common;

use std::cell::RefCell;
use std::sync::Arc;

use cljrs_gc::mutator::MutatorState;
use cljrs_runtime::env::callback::capture_eval_context;
use cljrs_runtime::env::env::Env;
use cljrs_value::{Arity, NativeFn, Value};

use common::{eval_in, fresh_env};

thread_local! {
    /// What evaluation B saw, recorded by `park!`.
    static SEEN_BY_B: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn install_park(globals: &Arc<cljrs_runtime::env::env::GlobalEnv>, ns: &str) {
    let b_globals = globals.clone();
    let b_ns = ns.to_string();
    let park = NativeFn::with_closure("park!", Arity::Fixed(0), move |_| {
        let a_state = MutatorState::save();

        // Detached: no eval context, no bindings of A's are visible.
        assert!(capture_eval_context().is_none());

        let mut b_env = Env::new(b_globals.clone(), &b_ns);
        let seen = eval_in(
            &mut b_env,
            r#"[(pr-str *who*)
                (binding [*who* :b] (with-out-str (print *who*)))
                (pr-str *who*)]"#,
        )
        .expect("evaluation B");
        SEEN_BY_B.with(|s| s.borrow_mut().push(format!("{seen}")));

        a_state.restore();
        Ok(Value::Nil)
    });
    globals.intern(
        ns,
        Arc::from("park!"),
        Value::NativeFunction(cljrs_gc::GcPtr::new(park)),
    );
}

#[test]
fn evaluations_do_not_share_dynamic_state_across_a_switch() {
    let (globals, mut env) = fresh_env();
    let ns = env.current_ns.to_string();
    install_park(&globals, &ns);
    eval_in(&mut env, "(def ^:dynamic *who* :root)").unwrap();

    let result = eval_in(
        &mut env,
        r#"(binding [*who* :a]
             (with-out-str
               (print *who*)
               (park!)
               (print *who*)))"#,
    )
    .expect("evaluation A");

    // A's output capture and binding survived the switch; nothing B printed
    // leaked into it.
    assert_eq!(result, Value::string(":a:a"));

    // B saw only the root binding and its own frame and capture.
    let seen = SEEN_BY_B.with(|s| s.borrow().clone());
    assert_eq!(seen, vec![r#"[":root" ":b" ":root"]"#.to_string()]);

    // A's frames are all popped once it finishes.
    assert_eq!(who_str(&mut env), ":root");
}

fn who_str(env: &mut Env) -> String {
    match eval_in(env, "(pr-str *who*)").unwrap() {
        Value::Str(s) => s.get().to_string(),
        other => panic!("expected a string, got {other:?}"),
    }
}

#[test]
fn swap_with_current_alternates_between_two_computations() {
    let (_globals, mut env) = fresh_env();
    eval_in(&mut env, "(def ^:dynamic *who* :root)").unwrap();
    let who = env
        .globals
        .lookup_var(&env.current_ns.clone(), "*who*")
        .expect("*who* var");

    // Computation A: one binding frame, installed by hand.
    let mut frame = std::collections::HashMap::new();
    frame.insert(
        cljrs_runtime::env::dynamics::var_key_of(&who),
        eval_in(&mut env, ":a").unwrap(),
    );
    let guard = cljrs_runtime::env::dynamics::push_frame(frame);

    // Computation B starts fresh.
    let mut other = MutatorState::fresh();
    other.swap_with_current();
    assert_eq!(who_str(&mut env), ":root");

    // Back to A.
    other.swap_with_current();
    assert_eq!(who_str(&mut env), ":a");

    drop(guard);
    assert_eq!(who_str(&mut env), ":root");
}
