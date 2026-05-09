//! Pick a representative "blame" use for an allocation that didn't make
//! it into a region.  The escape analyzer joins all uses' contributions
//! into a single `EscapeState`; for visualization we want to point at one
//! concrete use that explains the verdict.
//!
//! Strategy: scan the use chain and return the first use whose
//! contribution matches the joined verdict.

use cljrs_ir::lower::{EscapeState, UseInfo, UseKind};

/// Classify what a single use contributes to the escape lattice.
fn use_contribution(kind: &UseKind) -> EscapeState {
    match kind {
        UseKind::Return => EscapeState::Returns,
        UseKind::DefVar
        | UseKind::SetBang
        | UseKind::ClosureCapture
        | UseKind::Throw
        | UseKind::StoredInHeap
        | UseKind::Recur => EscapeState::Escapes,
        UseKind::UnknownCallArg { .. } | UseKind::CallCallee => EscapeState::Escapes,
        UseKind::KnownCallArg { .. } => EscapeState::ArgEscape,
        // These don't escape on their own; they propagate via the analyzer
        // to other uses.
        UseKind::PhiInput | UseKind::BranchCond | UseKind::Deref => EscapeState::NoEscape,
    }
}

/// Find the first use whose contribution is `>=` the joined verdict
/// (so that we point at a use that actually justifies the verdict, rather
/// than a benign one).
pub fn blame_use(uses: &[UseInfo], verdict: EscapeState) -> Option<&UseInfo> {
    if verdict == EscapeState::NoEscape {
        return None;
    }
    uses.iter()
        .find(|u| rank(use_contribution(&u.kind)) >= rank(verdict))
}

fn rank(s: EscapeState) -> u8 {
    match s {
        EscapeState::NoEscape => 0,
        EscapeState::ArgEscape => 1,
        EscapeState::Returns => 2,
        EscapeState::Escapes => 3,
    }
}

/// Human-readable label for a use kind, used in the visualizer.
pub fn use_kind_label(kind: &UseKind) -> String {
    match kind {
        UseKind::Return => "return value".into(),
        UseKind::DefVar => "stored in def'd var".into(),
        UseKind::SetBang => "set!".into(),
        UseKind::ClosureCapture => "captured by closure".into(),
        UseKind::Throw => "thrown".into(),
        UseKind::StoredInHeap => "stored into heap object".into(),
        UseKind::Recur => "passed to recur".into(),
        UseKind::KnownCallArg { func, arg_index } => {
            format!("arg {arg_index} of known call {func:?}")
        }
        UseKind::UnknownCallArg { arg_index, .. } => {
            format!("arg {arg_index} of unknown call")
        }
        UseKind::CallCallee => "callee of unknown call".into(),
        UseKind::PhiInput => "phi input".into(),
        UseKind::BranchCond => "branch condition".into(),
        UseKind::Deref => "deref'd".into(),
    }
}

/// Short label for an escape state, used as a badge.
pub fn state_label(state: EscapeState) -> &'static str {
    match state {
        EscapeState::NoEscape => "no-escape",
        EscapeState::ArgEscape => "arg-escape",
        EscapeState::Returns => "returns",
        EscapeState::Escapes => "escapes",
    }
}
