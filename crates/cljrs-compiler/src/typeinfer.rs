//! Scalar representation inference for [`IrFunction`]s — Phase 10.6.
//!
//! Assigns every IR `VarId` a [`Repr`]: either an unboxed machine
//! representation (`i64` long, `f64` double, `i8` boolean) or the default
//! boxed `*const Value`.  Codegen (`codegen.rs`) keeps unboxed values in
//! registers — arithmetic compiles to `iadd`/`fadd`/`icmp` instead of
//! `rt_add`/`rt_lt` bridge calls (each of which allocates or interns a boxed
//! result on the GC heap) — and boxes a value only where it flows into a
//! boxed context (a call argument, collection element, return, phi that
//! joins with a boxed value, …).
//!
//! ## Soundness
//!
//! A var is given an unboxed repr only when the *exact* runtime semantics of
//! the boxed bridge are expressible on the raw representation:
//!
//! - `rt_add`/`rt_sub`/`rt_mul` on `(Long, Long)` use `wrapping_*` — identical
//!   to Cranelift `iadd`/`isub`/`imul`.  Mixed `Long`/`Double` operands
//!   promote to `f64`, identical to `fcvt_from_sint` + `fadd`/….
//! - `rt_lt`/`rt_gt`/`rt_lte`/`rt_gte` compare `(Long, Long)` as `i64` and
//!   promote mixed operands to `f64` — identical to `icmp`/`fcmp`.
//! - `rt_eq` on `(Long, Long)` is `i64` equality.  Other operand types keep
//!   the boxed call (`Value` equality has cross-variant rules).
//! - `rt_div` truncates `(Long, Long)` and yields nil for division by zero —
//!   not expressible unboxed, so `Div`/`Rem` results stay boxed except the
//!   all-`Double` case which is exactly `fdiv`.
//! - Truthiness of an unboxed long/double is constant `true` (every number is
//!   truthy); of an unboxed bool it is the raw `i8`.
//!
//! Parameters are seeded from the caller-supplied specialization `specs`
//! (derived from Tier-1 type profiles; see `cljrs-eval/src/jit_state.rs`).
//! The compiled prologue *guards* each specialized parameter's runtime type
//! and deoptimizes to Tier 1 on mismatch, so inference may trust the seeds.

use std::collections::HashMap;

use crate::ir::{Const, Inst, IrFunction, KnownFn, Terminator, VarId};

/// Machine representation of an IR variable in compiled code.
///
/// Defined in `cljrs-ir` (so `IrFunction` can carry static `seed_reprs` from
/// `^long`/`^double` type hints) and re-exported here for backwards
/// compatibility — existing `cljrs_compiler::typeinfer::Repr` paths still work.
pub use crate::ir::Repr;

/// Lattice element during inference: `None` = ⊥ (not yet computed).
type Lat = Option<Repr>;

fn meet(a: Lat, b: Lat) -> Lat {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(x), Some(y)) if x == y => Some(x),
        _ => Some(Repr::Boxed),
    }
}

/// Result repr of an arithmetic [`KnownFn`] (`Add`/`Sub`/`Mul`) given operand
/// reprs.  `None` (⊥) propagates; any boxed/non-numeric operand forces Boxed.
fn arith_result(a: Lat, b: Lat) -> Lat {
    match (a?, b?) {
        (Repr::Long, Repr::Long) => Some(Repr::Long),
        (Repr::Long | Repr::Double, Repr::Long | Repr::Double) => Some(Repr::Double),
        _ => Some(Repr::Boxed),
    }
}

/// Result repr of an ordered comparison (`Lt`/`Gt`/`Lte`/`Gte`).
fn cmp_result(a: Lat, b: Lat) -> Lat {
    match (a?, b?) {
        (Repr::Long | Repr::Double, Repr::Long | Repr::Double) => Some(Repr::Bool),
        _ => Some(Repr::Boxed),
    }
}

/// Infer a representation for every variable in `func`.
///
/// `specs` seeds the function parameters (positionally; missing entries are
/// `Boxed`).  Any variable not present in the returned map is `Boxed`.
pub fn infer(func: &IrFunction, specs: &[Repr]) -> HashMap<VarId, Repr> {
    let mut lat: HashMap<VarId, Lat> = HashMap::new();

    for (i, (_name, var)) in func.params.iter().enumerate() {
        let r = specs.get(i).copied().unwrap_or(Repr::Boxed);
        lat.insert(*var, Some(r));
    }

    // Seed `let`/`loop`-bound locals from static type hints.  These are folded
    // through the same monotonic `meet` as everything else, so a hint can only
    // confirm an unboxed repr the body's dataflow agrees with — it never
    // unsoundly forces a boxed-producing binding into an unboxed register.
    for (var, r) in &func.local_seed_reprs {
        let merged = meet(lat.get(var).copied().flatten(), Some(*r));
        lat.insert(*var, merged);
    }

    let get = |lat: &HashMap<VarId, Lat>, v: VarId| -> Lat { lat.get(&v).copied().flatten() };

    // Fixpoint: reprs move monotonically ⊥ → unboxed → Boxed, so this
    // terminates in a few passes even with loop-carried phis.
    loop {
        let mut changed = false;
        let update = |lat: &mut HashMap<VarId, Lat>, dst: VarId, new: Lat| {
            let old = lat.get(&dst).copied().flatten();
            let merged = meet(old, new);
            if merged != old {
                lat.insert(dst, merged);
                true
            } else {
                false
            }
        };

        for block in &func.blocks {
            for inst in block.phis.iter().chain(block.insts.iter()) {
                let new: Option<(VarId, Lat)> = match inst {
                    Inst::Const(dst, c) => {
                        let r = match c {
                            Const::Long(_) => Repr::Long,
                            Const::Double(_) => Repr::Double,
                            Const::Bool(_) => Repr::Bool,
                            _ => Repr::Boxed,
                        };
                        Some((*dst, Some(r)))
                    }
                    Inst::Phi(dst, entries) => {
                        let mut m: Lat = None;
                        for (_, v) in entries {
                            m = meet(m, get(&lat, *v));
                        }
                        Some((*dst, m))
                    }
                    Inst::CallKnown(dst, kf, args) if args.len() == 2 => {
                        let a = get(&lat, args[0]);
                        let b = get(&lat, args[1]);
                        let r = match kf {
                            KnownFn::Add | KnownFn::Sub | KnownFn::Mul => arith_result(a, b),
                            // Long/Long division truncates and nil-guards; only
                            // the all-double case is expressible unboxed.
                            KnownFn::Div => match (a, b) {
                                (Some(Repr::Double), Some(Repr::Double)) => Some(Repr::Double),
                                (None, _) | (_, None) => None,
                                _ => Some(Repr::Boxed),
                            },
                            KnownFn::Lt | KnownFn::Gt | KnownFn::Lte | KnownFn::Gte => {
                                cmp_result(a, b)
                            }
                            KnownFn::Eq => match (a, b) {
                                (Some(Repr::Long), Some(Repr::Long)) => Some(Repr::Bool),
                                (None, _) | (_, None) => None,
                                _ => Some(Repr::Boxed),
                            },
                            _ => Some(Repr::Boxed),
                        };
                        Some((*dst, r))
                    }
                    other => other.dst().map(|d| (d, Some(Repr::Boxed))),
                };
                if let Some((dst, r)) = new {
                    changed |= update(&mut lat, dst, r);
                }
            }
            // Recur back-edges feed loop-header phis positionally; fold the
            // jump argument reprs into the target block's phi lattice values.
            if let Terminator::RecurJump { target, args } = &block.terminator
                && let Some(tb) = func.blocks.iter().find(|b| b.id == *target)
            {
                for (phi, arg) in tb.phis.iter().zip(args.iter()) {
                    if let Inst::Phi(dst, _) = phi {
                        let m = get(&lat, *arg);
                        changed |= update(&mut lat, *dst, m);
                    }
                }
            }
        }

        if !changed {
            break;
        }
    }

    lat.into_iter()
        .filter_map(|(v, r)| r.map(|r| (v, r)))
        .filter(|(_, r)| *r != Repr::Boxed)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Block, BlockId};
    use std::sync::Arc;

    /// (fn [n] (loop [i 0 acc 0] (if (< i n) (recur (+ i 1) (+ acc i)) acc)))
    /// with `n` specialized to Long: everything must come out unboxed.
    #[test]
    fn loop_counter_infers_long_when_param_specialized() {
        let mut f = IrFunction::new(Some(Arc::from("sum")), None);
        let n = f.fresh_var();
        f.params = vec![(Arc::from("n"), n)];

        let entry = f.fresh_block();
        let header = f.fresh_block();
        let body = f.fresh_block();
        let exit = f.fresh_block();

        let zero1 = f.fresh_var();
        let zero2 = f.fresh_var();
        let i = f.fresh_var();
        let acc = f.fresh_var();
        let cond = f.fresh_var();
        let one = f.fresh_var();
        let i2 = f.fresh_var();
        let acc2 = f.fresh_var();

        f.blocks.push(Block {
            id: entry,
            phis: vec![],
            insts: vec![
                Inst::Const(zero1, Const::Long(0)),
                Inst::Const(zero2, Const::Long(0)),
            ],
            terminator: Terminator::RecurJump {
                target: header,
                args: vec![zero1, zero2],
            },
        });
        f.blocks.push(Block {
            id: header,
            phis: vec![
                Inst::Phi(i, vec![(entry, zero1), (body, i2)]),
                Inst::Phi(acc, vec![(entry, zero2), (body, acc2)]),
            ],
            insts: vec![Inst::CallKnown(cond, KnownFn::Lt, vec![i, n])],
            terminator: Terminator::Branch {
                cond,
                then_block: body,
                else_block: exit,
            },
        });
        f.blocks.push(Block {
            id: body,
            phis: vec![],
            insts: vec![
                Inst::Const(one, Const::Long(1)),
                Inst::CallKnown(i2, KnownFn::Add, vec![i, one]),
                Inst::CallKnown(acc2, KnownFn::Add, vec![acc, i]),
            ],
            terminator: Terminator::RecurJump {
                target: header,
                args: vec![i2, acc2],
            },
        });
        f.blocks.push(Block {
            id: exit,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Return(acc),
        });

        let reprs = infer(&f, &[Repr::Long]);
        assert_eq!(reprs.get(&n), Some(&Repr::Long));
        assert_eq!(reprs.get(&i), Some(&Repr::Long));
        assert_eq!(reprs.get(&acc), Some(&Repr::Long));
        assert_eq!(reprs.get(&i2), Some(&Repr::Long));
        assert_eq!(reprs.get(&acc2), Some(&Repr::Long));
        assert_eq!(reprs.get(&cond), Some(&Repr::Bool));

        // Without the specialization seed, `(< i n)` is mixed and the loop
        // phis are still unboxed Long, but the comparison must stay boxed.
        let reprs = infer(&f, &[]);
        assert_eq!(reprs.get(&n), None, "unspecialized param must be boxed");
        assert_eq!(reprs.get(&cond), None, "cmp with boxed operand stays boxed");
        assert_eq!(reprs.get(&i), Some(&Repr::Long));
    }

    /// A `let`-bound local seeded with a scalar hint is inferred unboxed when
    /// the dataflow agrees, and the hint is folded soundly through `meet`.
    #[test]
    fn local_seed_repr_is_honored() {
        let mut f = IrFunction::new(None, None);
        let x = f.fresh_var();
        let one = f.fresh_var();
        let y = f.fresh_var();
        let entry = f.fresh_block();
        f.blocks.push(Block {
            id: entry,
            phis: vec![],
            insts: vec![
                Inst::Const(x, Const::Long(5)),
                Inst::Const(one, Const::Long(1)),
                Inst::CallKnown(y, KnownFn::Add, vec![x, one]),
            ],
            terminator: Terminator::Return(y),
        });
        // Seed `x` as Long (as a `^long` let-binding would).
        f.local_seed_reprs = vec![(x, Repr::Long)];
        let reprs = infer(&f, &[]);
        assert_eq!(reprs.get(&x), Some(&Repr::Long));
        assert_eq!(reprs.get(&y), Some(&Repr::Long));
    }

    /// A phi joining a Long with a Boxed value must come out Boxed.
    #[test]
    fn mixed_phi_falls_back_to_boxed() {
        let mut f = IrFunction::new(None, None);
        let p = f.fresh_var();
        f.params = vec![(Arc::from("x"), p)];

        let entry = f.fresh_block();
        let a = f.fresh_block();
        let b = f.fresh_block();
        let join = f.fresh_block();

        let c = f.fresh_var();
        let long_v = f.fresh_var();
        let phi = f.fresh_var();

        f.blocks.push(Block {
            id: entry,
            phis: vec![],
            insts: vec![Inst::CallKnown(c, KnownFn::IsNil, vec![p])],
            terminator: Terminator::Branch {
                cond: c,
                then_block: a,
                else_block: b,
            },
        });
        f.blocks.push(Block {
            id: a,
            phis: vec![],
            insts: vec![Inst::Const(long_v, Const::Long(7))],
            terminator: Terminator::Jump(join),
        });
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Jump(join),
        });
        f.blocks.push(Block {
            id: join,
            phis: vec![Inst::Phi(phi, vec![(a, long_v), (b, p)])],
            insts: vec![],
            terminator: Terminator::Return(phi),
        });

        let reprs = infer(&f, &[]);
        assert_eq!(reprs.get(&long_v), Some(&Repr::Long));
        assert_eq!(reprs.get(&phi), None, "phi joining Long with Boxed");
    }
}
