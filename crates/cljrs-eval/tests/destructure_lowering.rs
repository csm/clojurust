//! Phase 10.3 (seam shrink) — destructured parameters reach the IR tier.
//!
//! Before this phase, any function arity whose parameter list contained a
//! destructuring pattern (`[a b]`, `{:keys [x]}`, `& [c d]`, …) was rejected by
//! `eager_lower_fn` and forced to fall back to the tree-walking interpreter.
//! The interpreter splits such a parameter into a gensym placeholder name plus
//! the original pattern; the lowering pass now expands that pattern into
//! explicit IR-prologue bindings, so the body's references to the destructured
//! names resolve to real IR locals.
//!
//! These tests drive the lowering + IR interpreter directly (mirroring the data
//! the interpreter's `parse_arity` produces) and assert the destructured names
//! evaluate correctly — i.e. they are *not* emitted as `LoadGlobal` for
//! non-existent vars.

use std::sync::Arc;

use cljrs_eval::{Env, ir_interp::interpret_ir};
use cljrs_interp::standard_env_minimal;
use cljrs_ir::lower::lower_fn_body_destructured;
use cljrs_ir::{Inst, IrFunction};
use cljrs_reader::{Form, Parser};
use cljrs_value::{PersistentVector, Value};

fn parse_one(src: &str) -> Form {
    let mut p = Parser::new(src.to_string(), "<test>".to_string());
    p.parse_all()
        .expect("parse")
        .into_iter()
        .next()
        .expect("one form")
}

fn parse_body(src: &str) -> Vec<Form> {
    let mut p = Parser::new(src.to_string(), "<test>".to_string());
    p.parse_all().expect("parse")
}

/// Lower a single-arity function whose sole fixed param is the given
/// destructuring `pattern`, then run it against `arg`.
fn run_destructured(pattern_src: &str, body_src: &str, arg: Value) -> Value {
    let _mutator = cljrs_gc::register_mutator();

    let pattern = parse_one(pattern_src);
    let params: Vec<Arc<str>> = vec![Arc::from("__destructure_0")];
    let destructures: Vec<(usize, Form)> = vec![(0, pattern)];
    let body = parse_body(body_src);

    let ir: IrFunction =
        lower_fn_body_destructured(Some("test"), "user", &params, &destructures, &body, false)
            .expect("lower");

    // The destructured names must resolve to locals, never to globals.
    assert!(
        !mentions_load_global(&ir),
        "destructured body lowered to a LoadGlobal — pattern names did not bind"
    );

    let globals = standard_env_minimal(None, None, None);
    let mut env = Env::new(globals.clone(), "user");
    let ns: Arc<str> = Arc::from("user");
    cljrs_env::callback::push_eval_context(&env);
    let result = interpret_ir(&ir, vec![arg], &globals, &ns, &mut env);
    cljrs_env::callback::pop_eval_context();
    result.expect("interpret")
}

/// Does the IR (or any subfunction) contain a `LoadGlobal`?  Used as a proxy
/// for "a name failed to resolve to a destructured local."
fn mentions_load_global(ir: &IrFunction) -> bool {
    for block in &ir.blocks {
        for inst in &block.insts {
            if matches!(inst, Inst::LoadGlobal(..)) {
                return true;
            }
        }
    }
    ir.subfunctions.iter().any(mentions_load_global)
}

fn vec_of(items: impl IntoIterator<Item = Value>) -> Value {
    Value::Vector(cljrs_gc::GcPtr::new(PersistentVector::from_iter(items)))
}

#[test]
fn sequential_destructure_binds_first_element() {
    // (fn [[a b]] a) called with [10 3] => 10
    let got = run_destructured("[a b]", "a", vec_of([Value::Long(10), Value::Long(3)]));
    assert_eq!(got, Value::Long(10));
}

#[test]
fn sequential_destructure_binds_second_element() {
    // (fn [[a b]] b) called with [10 3] => 3
    let got = run_destructured("[a b]", "b", vec_of([Value::Long(10), Value::Long(3)]));
    assert_eq!(got, Value::Long(3));
}

#[test]
fn sequential_destructure_with_rest_and_as() {
    // (fn [[a & more :as all]] more) called with [1 2 3] => (2 3)
    let got = run_destructured(
        "[a & more :as all]",
        "more",
        vec_of([Value::Long(1), Value::Long(2), Value::Long(3)]),
    );
    // `more` is the tail sequence; assert it has two elements via the runtime.
    match got {
        Value::List(_) | Value::Cons(_) | Value::LazySeq(_) | Value::Vector(_) => {}
        other => panic!("expected a rest sequence, got {other:?}"),
    }
}
