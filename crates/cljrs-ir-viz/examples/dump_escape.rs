//! Debug helper: lower a sample, run escape analysis, and dump the per-alloc
//! verdict + the full use chain for each allocation, recursively across
//! every (sub)function in the IR tree.

use cljrs_compiler::aot::lower_file_to_ir;
use cljrs_ir::lower::{
    AnalysisResult, EscapeContext, EscapeState, UseKind, analyze, make_analysis_context,
};
use cljrs_ir::{Inst, IrFunction, KnownFn};

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump-escape <file>");
    let (_src, ir) = lower_file_to_ir(std::path::Path::new(&path), &[], true).unwrap();
    let ctx = make_analysis_context(&ir);
    walk(&ir, "", &ctx);
}

fn walk(ir: &IrFunction, parent: &str, ctx: &EscapeContext) {
    let path = if parent.is_empty() {
        ir.name.as_deref().unwrap_or("<anon>").to_string()
    } else {
        format!("{parent} → {}", ir.name.as_deref().unwrap_or("<anon>"))
    };
    let analysis = analyze(ir, Some(ctx));
    println!("\n══ {path} ══");
    for block in &ir.blocks {
        for inst in &block.insts {
            if let Some(dst) = inst.dst()
                && let Some(state) = analysis.states.get(&dst).copied()
            {
                println!("  {dst} = {} → {:?}", short_inst(inst), state);
                if state != EscapeState::NoEscape {
                    print_use_chain(dst, &analysis, ir, &mut Default::default(), 4);
                }
            }
        }
    }
    for sub in &ir.subfunctions {
        walk(sub, &path, ctx);
    }
}

fn short_inst(inst: &Inst) -> String {
    match inst {
        Inst::AllocVector(_, e) => format!("alloc-vec [{}]", e.len()),
        Inst::AllocMap(_, p) => format!("alloc-map [{} pairs]", p.len()),
        Inst::AllocSet(_, e) => format!("alloc-set [{}]", e.len()),
        Inst::AllocList(_, e) => format!("alloc-list [{}]", e.len()),
        Inst::AllocCons(_, _, _) => "alloc-cons".into(),
        Inst::AllocClosure(_, t, c) => {
            format!("closure {:?} captures={}", t.name, c.len())
        }
        _ => format!("{inst}"),
    }
}

fn print_use_chain(
    var: cljrs_ir::VarId,
    analysis: &AnalysisResult,
    ir: &IrFunction,
    seen: &mut std::collections::HashSet<cljrs_ir::VarId>,
    indent: usize,
) {
    if !seen.insert(var) {
        return;
    }
    let pad = " ".repeat(indent);
    let Some(uses) = analysis.uses.get(&var) else {
        return;
    };
    for u in uses {
        println!(
            "{pad}↪ {var} used as {} in {}",
            describe_use(&u.kind),
            u.block
        );
        // For KnownCallArg-with-escape, follow the call result one level
        if let UseKind::KnownCallArg { func, .. } = &u.kind
            && known_arg_escapes(func)
            && let Some(call_dst) = find_call_result(var, func, ir, u.block)
        {
            println!("{pad}    → call result is {call_dst}");
            print_use_chain(call_dst, analysis, ir, seen, indent + 4);
        }
    }
}

fn describe_use(kind: &UseKind) -> String {
    match kind {
        UseKind::Return => "Return".into(),
        UseKind::DefVar => "DefVar".into(),
        UseKind::SetBang => "SetBang".into(),
        UseKind::ClosureCapture => "ClosureCapture".into(),
        UseKind::Throw => "Throw".into(),
        UseKind::StoredInHeap => "StoredInHeap".into(),
        UseKind::Recur => "Recur".into(),
        UseKind::KnownCallArg { func, arg_index } => {
            format!("KnownCall({func:?}, arg {arg_index})")
        }
        UseKind::UnknownCallArg { arg_index, .. } => format!("UnknownCall(arg {arg_index})"),
        UseKind::CallCallee => "CallCallee".into(),
        UseKind::PhiInput => "PhiInput".into(),
        UseKind::BranchCond => "BranchCond".into(),
        UseKind::Deref => "Deref".into(),
    }
}

fn known_arg_escapes(_func: &KnownFn) -> bool {
    // Rough indicator — the real table lives in cljrs_ir::lower::escape and
    // is private.  Matching the default-true behaviour is enough here since
    // we use this only to decide whether to follow the call result into the
    // dump's recursion.
    true
}

fn find_call_result(
    used: cljrs_ir::VarId,
    func: &KnownFn,
    ir: &IrFunction,
    block_id: cljrs_ir::BlockId,
) -> Option<cljrs_ir::VarId> {
    let block = ir.blocks.iter().find(|b| b.id == block_id)?;
    for inst in &block.insts {
        if let Inst::CallKnown(dst, f, args) = inst
            && f == func
            && args.contains(&used)
        {
            return Some(*dst);
        }
    }
    None
}
