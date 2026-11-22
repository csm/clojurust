//! The one definition of where an IR tree names a function.
//!
//! Every IR transformation that clones, renames, specialises or mangles
//! functions must agree on which fields are *function references*.  Keeping a
//! separate, partial list in each pass is how `CallWithRegion` targets were
//! left stale when a specialised function was cloned again (issue #506).
//!
//! This module is that list.  [`for_each_inst_fn_ref`] and
//! [`for_each_inst_fn_ref_mut`] match **every** [`Inst`] variant explicitly —
//! there is no `_` arm — so adding an instruction forces its author to decide
//! here whether it carries a function reference.  Tree-level helpers
//! ([`for_each_fn_ref`], [`remap_fn_refs`]) build on them, and the verifier in
//! [`crate::verify`] checks the result.
//!
//! ## What counts as a function reference
//!
//! | Site | Kind |
//! |---|---|
//! | [`IrFunction::name`] of a subfunction | [`FnRefKind::Definition`] |
//! | [`ClosureTemplate::arity_fn_names`]`[i]` in [`Inst::AllocClosure`] | [`FnRefKind::ClosureArity`] |
//! | target of [`Inst::CallWithRegion`] | [`FnRefKind::CallWithRegion`] |
//! | target of [`Inst::CallDirect`] (unless `.method` interop) | [`FnRefKind::CallDirect`] |
//!
//! Not references: [`ClosureTemplate::name`] (the Clojure-level display name
//! of the fn value), `LoadGlobal`/`LoadVar`/`DefVar` names (namespace vars,
//! resolved at runtime), and `.method` `CallDirect` markers (interop method
//! names, not functions).

use std::collections::HashMap;
use std::sync::Arc;

use crate::{Inst, IrFunction};

/// Which kind of function reference a visited name is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FnRefKind {
    /// The `name` of a (sub)function — a definition, not a use.
    Definition,
    /// `arity_fn_names[index]` of an `AllocClosure` template.
    ClosureArity { index: usize },
    /// The target of a `CallWithRegion`.
    CallWithRegion,
    /// The target of a non-interop `CallDirect`.
    CallDirect,
}

impl std::fmt::Display for FnRefKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Definition => write!(f, "definition"),
            Self::ClosureArity { index } => write!(f, "closure arity #{index}"),
            Self::CallWithRegion => write!(f, "CallWithRegion target"),
            Self::CallDirect => write!(f, "CallDirect target"),
        }
    }
}

/// `true` for a `CallDirect` name that is a `.method` interop marker rather
/// than a function name.
pub fn is_interop_marker(name: &str) -> bool {
    name.len() > 1 && name.starts_with('.')
}

/// Visit every function-reference *use* in `inst`.
pub fn for_each_inst_fn_ref(inst: &Inst, mut f: impl FnMut(FnRefKind, &Arc<str>)) {
    match inst {
        Inst::AllocClosure(_, tmpl, _) => {
            for (index, n) in tmpl.arity_fn_names.iter().enumerate() {
                f(FnRefKind::ClosureArity { index }, n);
            }
        }
        Inst::CallWithRegion(_, target, _, _) => f(FnRefKind::CallWithRegion, target),
        Inst::CallDirect(_, target, _) => {
            if !is_interop_marker(target) {
                f(FnRefKind::CallDirect, target);
            }
        }
        // No function reference.  Listed exhaustively on purpose: a new
        // instruction must be classified here.
        Inst::Const(..)
        | Inst::LoadLocal(..)
        | Inst::LoadGlobal(..)
        | Inst::LoadVar(..)
        | Inst::AllocVector(..)
        | Inst::AllocMap(..)
        | Inst::AllocSet(..)
        | Inst::AllocList(..)
        | Inst::AllocCons(..)
        | Inst::CallKnown(..)
        | Inst::Call(..)
        | Inst::Deref(..)
        | Inst::DefVar(..)
        | Inst::SetBang(..)
        | Inst::Throw(..)
        | Inst::Phi(..)
        | Inst::Recur(..)
        | Inst::SourceLoc(..)
        | Inst::RegionStart(..)
        | Inst::RegionAlloc(..)
        | Inst::RegionEnd(..)
        | Inst::RegionParam(..)
        | Inst::Await { .. }
        | Inst::Spawn { .. }
        | Inst::ChanTake { .. }
        | Inst::ChanPut { .. }
        | Inst::StateStore { .. }
        | Inst::StateLoad { .. }
        | Inst::AsyncSuspend { .. }
        | Inst::AsyncResume { .. } => {}
    }
}

/// Mutable counterpart of [`for_each_inst_fn_ref`].
pub fn for_each_inst_fn_ref_mut(inst: &mut Inst, mut f: impl FnMut(FnRefKind, &mut Arc<str>)) {
    match inst {
        Inst::AllocClosure(_, tmpl, _) => {
            for (index, n) in tmpl.arity_fn_names.iter_mut().enumerate() {
                f(FnRefKind::ClosureArity { index }, n);
            }
        }
        Inst::CallWithRegion(_, target, _, _) => f(FnRefKind::CallWithRegion, target),
        Inst::CallDirect(_, target, _) => {
            if !is_interop_marker(target) {
                f(FnRefKind::CallDirect, target);
            }
        }
        Inst::Const(..)
        | Inst::LoadLocal(..)
        | Inst::LoadGlobal(..)
        | Inst::LoadVar(..)
        | Inst::AllocVector(..)
        | Inst::AllocMap(..)
        | Inst::AllocSet(..)
        | Inst::AllocList(..)
        | Inst::AllocCons(..)
        | Inst::CallKnown(..)
        | Inst::Call(..)
        | Inst::Deref(..)
        | Inst::DefVar(..)
        | Inst::SetBang(..)
        | Inst::Throw(..)
        | Inst::Phi(..)
        | Inst::Recur(..)
        | Inst::SourceLoc(..)
        | Inst::RegionStart(..)
        | Inst::RegionAlloc(..)
        | Inst::RegionEnd(..)
        | Inst::RegionParam(..)
        | Inst::Await { .. }
        | Inst::Spawn { .. }
        | Inst::ChanTake { .. }
        | Inst::ChanPut { .. }
        | Inst::StateStore { .. }
        | Inst::StateLoad { .. }
        | Inst::AsyncSuspend { .. }
        | Inst::AsyncResume { .. } => {}
    }
}

/// Visit every function reference in the tree rooted at `root`: each
/// function's own name ([`FnRefKind::Definition`], root included) and every
/// use inside its blocks.  `f` receives the function containing the site.
pub fn for_each_fn_ref(root: &IrFunction, f: &mut impl FnMut(&IrFunction, FnRefKind, &Arc<str>)) {
    if let Some(name) = &root.name {
        f(root, FnRefKind::Definition, name);
    }
    for block in &root.blocks {
        for inst in block.phis.iter().chain(block.insts.iter()) {
            for_each_inst_fn_ref(inst, |kind, name| f(root, kind, name));
        }
    }
    for sub in &root.subfunctions {
        for_each_fn_ref(sub, f);
    }
}

/// Rewrite every function reference in the tree rooted at `root` — the
/// definitions (each function's `name`, root included) **and** every use —
/// through `map`.  Names absent from `map` are left unchanged.
///
/// Renaming a definition without its uses (or vice versa) is exactly the
/// dangling-reference bug this function exists to prevent, so passes should
/// go through it rather than rewriting names by hand.
pub fn remap_fn_refs(root: &mut IrFunction, map: &HashMap<Arc<str>, Arc<str>>) {
    if map.is_empty() {
        return;
    }
    remap_fn_refs_with(root, &mut |_, name| map.get(name).cloned());
}

/// Like [`remap_fn_refs`], but driven by a callback: `f(kind, name)` returns
/// the replacement name, or `None` to leave it unchanged.
pub fn remap_fn_refs_with(
    root: &mut IrFunction,
    f: &mut impl FnMut(FnRefKind, &str) -> Option<Arc<str>>,
) {
    if let Some(name) = &mut root.name
        && let Some(new) = f(FnRefKind::Definition, name)
    {
        *name = new;
    }
    for block in &mut root.blocks {
        for inst in block.phis.iter_mut().chain(block.insts.iter_mut()) {
            for_each_inst_fn_ref_mut(inst, |kind, name| {
                if let Some(new) = f(kind, name) {
                    *name = new;
                }
            });
        }
    }
    for sub in &mut root.subfunctions {
        remap_fn_refs_with(sub, f);
    }
}

/// Collect the names of every *subfunction* (at any depth) of `root` — the
/// functions that are cloned along with it.  The root's own name is excluded.
pub fn subfunction_names(root: &IrFunction) -> Vec<Arc<str>> {
    fn walk(f: &IrFunction, out: &mut Vec<Arc<str>>) {
        for sub in &f.subfunctions {
            if let Some(n) = &sub.name {
                out.push(n.clone());
            }
            walk(sub, out);
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Block, BlockId, ClosureTemplate, Terminator, VarId};

    fn func(name: &str, insts: Vec<Inst>) -> IrFunction {
        let mut f = IrFunction::new(Some(Arc::from(name)), None);
        f.blocks.push(Block {
            id: BlockId(0),
            phis: Vec::new(),
            insts,
            terminator: Terminator::Return(VarId(0)),
        });
        f
    }

    #[test]
    fn remap_covers_definitions_and_every_use_kind() {
        let tmpl = ClosureTemplate {
            name: Some(Arc::from("g")),
            arity_fn_names: vec![Arc::from("inner")],
            param_counts: vec![0],
            is_variadic: vec![false],
            capture_names: vec![],
        };
        let mut root = func(
            "root",
            vec![
                Inst::AllocClosure(VarId(1), tmpl, vec![]),
                Inst::CallWithRegion(VarId(2), Arc::from("inner"), vec![], VarId(3)),
                Inst::CallDirect(VarId(4), Arc::from("inner"), vec![]),
                Inst::CallDirect(VarId(5), Arc::from(".inner"), vec![VarId(1)]),
            ],
        );
        root.subfunctions.push(func("inner", vec![]));

        let map: HashMap<Arc<str>, Arc<str>> = [(Arc::from("inner"), Arc::from("inner__x"))]
            .into_iter()
            .collect();
        remap_fn_refs(&mut root, &map);

        let mut seen = Vec::new();
        for_each_fn_ref(&root, &mut |_, kind, name| {
            seen.push((kind, name.to_string()))
        });
        assert!(seen.contains(&(FnRefKind::Definition, "inner__x".into())));
        assert!(seen.contains(&(FnRefKind::ClosureArity { index: 0 }, "inner__x".into())));
        assert!(seen.contains(&(FnRefKind::CallWithRegion, "inner__x".into())));
        assert!(seen.contains(&(FnRefKind::CallDirect, "inner__x".into())));
        assert!(!seen.iter().any(|(_, n)| n == "inner"));
        // Interop markers and the template's display name are not references.
        let Inst::CallDirect(_, m, _) = &root.blocks[0].insts[3] else {
            unreachable!()
        };
        assert_eq!(m.as_ref(), ".inner");
        let Inst::AllocClosure(_, t, _) = &root.blocks[0].insts[0] else {
            unreachable!()
        };
        assert_eq!(t.name.as_deref(), Some("g"));
    }
}
