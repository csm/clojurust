//! IR-level verification of direct function references.
//!
//! Passes that clone, rename or specialise functions (stage-4 region
//! promotion above all) can leave a reference pointing at a function that no
//! longer exists under the name it uses.  Cranelift never sees the IR
//! interpreter's input, so a dangling target there surfaces only as a runtime
//! `not found` error far from the pass that caused it.  [`verify_fn_refs`]
//! checks the tree independently, using the lookup rules the execution
//! engines actually apply:
//!
//! * **Definitions** — every named function in the tree is declared under its
//!   name by codegen (`declare_subfunctions`), so names must be unique across
//!   the whole tree.  This also makes the interpreter's first-match lookup
//!   among siblings unambiguous.
//! * **`CallWithRegion`** — the interpreter resolves the target among the
//!   *direct* subfunctions of the calling function.  The target must take the
//!   hidden region parameter (`RegionParam` in its entry block), its explicit
//!   arity must match the call's arguments, and the region operand must be a
//!   region handle (`RegionStart`/`RegionParam`) of the caller.
//! * **`AllocClosure` arity names** — resolved among the direct subfunctions
//!   of the allocating function.  Each target's parameter list is
//!   `captures ++ fixed params ++ [rest]`, and it must not expect a region.
//! * **`CallDirect`** — resolved by name at runtime (interpreter: a global
//!   lookup; codegen: any function in the unit or a previously compiled one),
//!   so a name not defined in the tree is a legitimate external reference.
//!   When the name *is* defined in the tree, the call must match its
//!   calling convention (arity, and no hidden region parameter).
//!
//! The verifier runs in debug builds at the boundaries of the optimisation
//! pipeline (see `lower::optimize::optimize_with_externals`) and is public so
//! tests and other passes can call it directly.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use crate::fnref::{FnRefKind, for_each_inst_fn_ref};
use crate::{BlockId, Inst, IrFunction, VarId};

/// One verification failure.
#[derive(Debug, Clone)]
pub struct FnRefError {
    /// Name of the function containing the offending site (`<anon>` if none).
    pub function: String,
    /// `(block, instruction index)` of the offending instruction, if any.
    pub location: Option<(BlockId, usize)>,
    /// Debug rendering of the offending instruction, if any.
    pub inst: Option<String>,
    /// What is wrong.
    pub problem: FnRefProblem,
}

/// The specific defect a [`FnRefError`] reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FnRefProblem {
    /// Two functions in the tree share a name.
    DuplicateDefinition { name: Arc<str> },
    /// A reference whose target is not reachable under the engine's lookup
    /// rule (direct subfunctions of the containing function).
    Unresolved { kind: FnRefKind, target: Arc<str> },
    /// A reference resolves but the target's calling convention does not
    /// match the site.
    Incompatible {
        kind: FnRefKind,
        target: Arc<str>,
        reason: String,
    },
    /// An `AllocClosure` template whose per-arity vectors disagree in length.
    MalformedTemplate { reason: String },
}

impl fmt::Display for FnRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "in fn `{}`", self.function)?;
        if let Some((block, idx)) = self.location {
            write!(f, " at {block}[{idx}]")?;
        }
        match &self.problem {
            FnRefProblem::DuplicateDefinition { name } => write!(
                f,
                ": function `{name}` is defined more than once in the tree"
            )?,
            FnRefProblem::Unresolved { kind, target } => write!(
                f,
                ": {kind} `{target}` does not resolve to a subfunction of this fn"
            )?,
            FnRefProblem::Incompatible {
                kind,
                target,
                reason,
            } => write!(f, ": {kind} `{target}` is incompatible: {reason}")?,
            FnRefProblem::MalformedTemplate { reason } => {
                write!(f, ": malformed closure template: {reason}")?
            }
        }
        if let Some(inst) = &self.inst {
            write!(f, "\n    inst: {inst}")?;
        }
        Ok(())
    }
}

/// `true` if `f` expects the hidden region parameter (a `RegionParam` in its
/// entry block), i.e. it is a region-parameterised variant.
pub fn takes_region(f: &IrFunction) -> bool {
    f.blocks
        .first()
        .is_some_and(|b| b.insts.iter().any(|i| matches!(i, Inst::RegionParam(_))))
}

/// Verify every function reference in the tree rooted at `root`.
///
/// Returns all failures found (not just the first), so a diagnostic can show
/// the full extent of a broken rewrite.
pub fn verify_fn_refs(root: &IrFunction) -> Result<(), Vec<FnRefError>> {
    let mut errors = Vec::new();

    // Tree-wide definitions.
    let mut defs: HashMap<Arc<str>, &IrFunction> = HashMap::new();
    collect_defs(root, &mut defs, &mut errors);

    verify_fn(root, &defs, &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Format a verifier result as one multi-line message (empty on success).
pub fn format_errors(errors: &[FnRefError]) -> String {
    errors
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Debug-build checkpoint for a sequence of passes.
///
/// [`PassChecker::new`] records whether the *input* tree verifies; each
/// [`PassChecker::after`] then re-verifies and panics naming the pass that
/// first broke a reference.  When the input already fails, the checker
/// panics at construction (naming `source`), so a defect is always
/// attributed to the stage that introduced it.
pub struct PassChecker;

impl PassChecker {
    /// Verify the pipeline input; `source` names where it came from.
    pub fn new(source: &str, input: &IrFunction) -> Self {
        Self::check(source, "input from", input);
        PassChecker
    }

    /// Verify the tree after `pass` ran.
    pub fn after(&self, pass: &str, f: &IrFunction) {
        Self::check(pass, "after pass", f);
    }

    fn check(stage: &str, what: &str, f: &IrFunction) {
        if let Err(errors) = verify_fn_refs(f) {
            panic!(
                "IR function-reference verification failed {what} `{stage}` (root fn `{}`):\n{}",
                fn_label(f),
                format_errors(&errors)
            );
        }
    }
}

fn fn_label(f: &IrFunction) -> String {
    f.name.as_deref().unwrap_or("<anon>").to_string()
}

fn collect_defs<'a>(
    f: &'a IrFunction,
    defs: &mut HashMap<Arc<str>, &'a IrFunction>,
    errors: &mut Vec<FnRefError>,
) {
    for sub in &f.subfunctions {
        if let Some(name) = &sub.name
            && defs.insert(name.clone(), sub).is_some()
        {
            errors.push(FnRefError {
                function: fn_label(f),
                location: None,
                inst: None,
                problem: FnRefProblem::DuplicateDefinition { name: name.clone() },
            });
        }
        collect_defs(sub, defs, errors);
    }
}

fn verify_fn(f: &IrFunction, defs: &HashMap<Arc<str>, &IrFunction>, errors: &mut Vec<FnRefError>) {
    // Direct-subfunction lookup scope (first match, as the interpreter does).
    let mut local: HashMap<&str, &IrFunction> = HashMap::new();
    for sub in &f.subfunctions {
        if let Some(n) = sub.name.as_deref() {
            local.entry(n).or_insert(sub);
        }
    }
    // Region handles defined in this function.
    let region_handles: std::collections::HashSet<VarId> = f
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter_map(|i| match i {
            Inst::RegionStart(v) | Inst::RegionParam(v) => Some(*v),
            _ => None,
        })
        .collect();

    for block in &f.blocks {
        for (idx, inst) in block.phis.iter().chain(block.insts.iter()).enumerate() {
            let mut site_errors: Vec<FnRefProblem> = Vec::new();

            if let Inst::AllocClosure(_, tmpl, _) = inst {
                let n = tmpl.arity_fn_names.len();
                if n != 0 && (tmpl.param_counts.len() != n || tmpl.is_variadic.len() != n) {
                    site_errors.push(FnRefProblem::MalformedTemplate {
                        reason: format!(
                            "{n} arity names, {} param counts, {} variadic flags",
                            tmpl.param_counts.len(),
                            tmpl.is_variadic.len()
                        ),
                    });
                }
            }

            for_each_inst_fn_ref(inst, |kind, target| {
                if let Some(p) = check_ref(inst, kind, target, &local, defs, &region_handles) {
                    site_errors.push(p);
                }
            });

            for problem in site_errors {
                errors.push(FnRefError {
                    function: fn_label(f),
                    location: Some((block.id, idx)),
                    inst: Some(format!("{inst:?}")),
                    problem,
                });
            }
        }
    }

    for sub in &f.subfunctions {
        verify_fn(sub, defs, errors);
    }
}

fn check_ref(
    inst: &Inst,
    kind: FnRefKind,
    target: &Arc<str>,
    local: &HashMap<&str, &IrFunction>,
    defs: &HashMap<Arc<str>, &IrFunction>,
    region_handles: &std::collections::HashSet<VarId>,
) -> Option<FnRefProblem> {
    let incompatible = |reason: String| {
        Some(FnRefProblem::Incompatible {
            kind,
            target: target.clone(),
            reason,
        })
    };
    match (kind, inst) {
        (FnRefKind::CallWithRegion, Inst::CallWithRegion(_, _, args, region)) => {
            let Some(callee) = local.get(target.as_ref()) else {
                return Some(FnRefProblem::Unresolved {
                    kind,
                    target: target.clone(),
                });
            };
            if !takes_region(callee) {
                return incompatible("target has no hidden region parameter".into());
            }
            if callee.params.len() != args.len() {
                return incompatible(format!(
                    "call passes {} args, target takes {} params",
                    args.len(),
                    callee.params.len()
                ));
            }
            if !region_handles.contains(region) {
                return incompatible(format!(
                    "region operand {region} is not a RegionStart/RegionParam of the caller"
                ));
            }
            None
        }
        (FnRefKind::ClosureArity { index }, Inst::AllocClosure(_, tmpl, captures)) => {
            let Some(callee) = local.get(target.as_ref()) else {
                return Some(FnRefProblem::Unresolved {
                    kind,
                    target: target.clone(),
                });
            };
            if takes_region(callee) {
                return incompatible("closure arity expects a hidden region parameter".into());
            }
            let (Some(&fixed), Some(&variadic)) =
                (tmpl.param_counts.get(index), tmpl.is_variadic.get(index))
            else {
                // Reported as MalformedTemplate.
                return None;
            };
            let expected = captures.len() + fixed + usize::from(variadic);
            if callee.params.len() != expected {
                return incompatible(format!(
                    "template expects {expected} params ({} captures + {fixed} fixed{}), target takes {}",
                    captures.len(),
                    if variadic { " + rest" } else { "" },
                    callee.params.len()
                ));
            }
            None
        }
        (FnRefKind::CallDirect, Inst::CallDirect(_, _, args)) => {
            // Undefined in this tree ⇒ external (another unit / a global).
            let callee = local
                .get(target.as_ref())
                .copied()
                .or_else(|| defs.get(target).copied())?;
            if takes_region(callee) {
                return incompatible(
                    "target expects a hidden region parameter; use CallWithRegion".into(),
                );
            }
            if callee.params.len() != args.len() {
                return incompatible(format!(
                    "call passes {} args, target takes {} params",
                    args.len(),
                    callee.params.len()
                ));
            }
            None
        }
        // `for_each_inst_fn_ref` only yields the kinds above for these insts.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Block, ClosureTemplate, Terminator};

    fn func(name: &str, params: usize, insts: Vec<Inst>) -> IrFunction {
        let mut f = IrFunction::new(Some(Arc::from(name)), None);
        f.params = (0..params)
            .map(|i| (Arc::from(format!("p{i}").as_str()), VarId(i as u32)))
            .collect();
        f.blocks.push(Block {
            id: BlockId(0),
            phis: Vec::new(),
            insts,
            terminator: Terminator::Return(VarId(0)),
        });
        f.next_var = 50;
        f.next_block = 1;
        f
    }

    fn region_variant(name: &str, params: usize) -> IrFunction {
        func(name, params, vec![Inst::RegionParam(VarId(40))])
    }

    fn caller_with_region_call(target: &str) -> IrFunction {
        func(
            "caller",
            1,
            vec![
                Inst::RegionStart(VarId(10)),
                Inst::CallWithRegion(VarId(11), Arc::from(target), vec![VarId(0)], VarId(10)),
                Inst::RegionEnd(VarId(10)),
            ],
        )
    }

    #[test]
    fn valid_region_call_passes() {
        let mut caller = caller_with_region_call("leaf__rg1");
        caller.subfunctions.push(region_variant("leaf__rg1", 1));
        verify_fn_refs(&caller).expect("valid");
    }

    #[test]
    fn dangling_call_with_region_is_rejected() {
        let mut caller = caller_with_region_call("leaf__rg1");
        caller
            .subfunctions
            .push(region_variant("leaf__rg1__rg2", 1));
        let errs = verify_fn_refs(&caller).unwrap_err();
        assert_eq!(errs.len(), 1);
        let e = &errs[0];
        assert_eq!(e.function, "caller");
        assert_eq!(e.location, Some((BlockId(0), 1)));
        assert_eq!(
            e.problem,
            FnRefProblem::Unresolved {
                kind: FnRefKind::CallWithRegion,
                target: Arc::from("leaf__rg1"),
            }
        );
        let msg = e.to_string();
        assert!(msg.contains("caller") && msg.contains("leaf__rg1"), "{msg}");
        assert!(msg.contains("CallWithRegion"), "{msg}");
    }

    #[test]
    fn region_call_target_must_be_a_direct_subfunction() {
        // Defined in the tree, but nested one level too deep for the
        // interpreter's lookup.
        let mut caller = caller_with_region_call("leaf__rg1");
        let mut holder = func("holder", 0, vec![]);
        holder.subfunctions.push(region_variant("leaf__rg1", 1));
        caller.subfunctions.push(holder);
        assert!(verify_fn_refs(&caller).is_err());
    }

    #[test]
    fn region_call_convention_is_checked() {
        // Target without RegionParam.
        let mut caller = caller_with_region_call("leaf");
        caller.subfunctions.push(func("leaf", 1, vec![]));
        let errs = verify_fn_refs(&caller).unwrap_err();
        assert!(matches!(errs[0].problem, FnRefProblem::Incompatible { .. }));

        // Arity mismatch.
        let mut caller = caller_with_region_call("leaf__rg1");
        caller.subfunctions.push(region_variant("leaf__rg1", 2));
        let errs = verify_fn_refs(&caller).unwrap_err();
        assert!(errs[0].to_string().contains("1 args"), "{}", errs[0]);
    }

    #[test]
    fn duplicate_definitions_are_rejected() {
        let mut root = func("root", 0, vec![]);
        let mut a = func("a", 0, vec![]);
        a.subfunctions.push(func("dup", 0, vec![]));
        root.subfunctions.push(a);
        root.subfunctions.push(func("dup", 0, vec![]));
        let errs = verify_fn_refs(&root).unwrap_err();
        assert!(matches!(
            &errs[0].problem,
            FnRefProblem::DuplicateDefinition { name } if name.as_ref() == "dup"
        ));
    }

    fn closure(names: &[&str], fixed: usize, captures: Vec<VarId>) -> Inst {
        Inst::AllocClosure(
            VarId(20),
            ClosureTemplate {
                name: None,
                arity_fn_names: names.iter().map(|n| Arc::from(*n)).collect(),
                param_counts: vec![fixed; names.len()],
                is_variadic: vec![false; names.len()],
                capture_names: vec![],
            },
            captures,
        )
    }

    #[test]
    fn closure_arity_refs_are_checked() {
        // captures (1) + fixed (2) = 3 params.
        let mut root = func("root", 1, vec![closure(&["arity"], 2, vec![VarId(0)])]);
        root.subfunctions.push(func("arity", 3, vec![]));
        verify_fn_refs(&root).expect("valid");

        let mut root = func("root", 1, vec![closure(&["missing"], 2, vec![VarId(0)])]);
        root.subfunctions.push(func("arity", 3, vec![]));
        let errs = verify_fn_refs(&root).unwrap_err();
        assert!(matches!(
            errs[0].problem,
            FnRefProblem::Unresolved {
                kind: FnRefKind::ClosureArity { index: 0 },
                ..
            }
        ));

        let mut root = func("root", 1, vec![closure(&["arity"], 2, vec![])]);
        root.subfunctions.push(func("arity", 3, vec![]));
        assert!(verify_fn_refs(&root).is_err());
    }

    #[test]
    fn call_direct_externals_are_allowed_but_local_targets_checked() {
        let root = func(
            "root",
            1,
            vec![
                Inst::CallDirect(VarId(1), Arc::from("other_unit_fn"), vec![VarId(0)]),
                Inst::CallDirect(VarId(2), Arc::from(".method"), vec![VarId(0)]),
            ],
        );
        verify_fn_refs(&root).expect("externals and interop markers are fine");

        let mut root = func(
            "root",
            1,
            vec![Inst::CallDirect(
                VarId(1),
                Arc::from("v__rg1"),
                vec![VarId(0)],
            )],
        );
        root.subfunctions.push(region_variant("v__rg1", 1));
        assert!(verify_fn_refs(&root).is_err());
    }
}
