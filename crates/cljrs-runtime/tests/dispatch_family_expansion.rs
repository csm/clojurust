//! The datatype/protocol/multimethod family is named once, in
//! `cljrs_ir::lower::dispatch_family`. These tests pin the two halves of that
//! single definition against each other: what the bootstrap macros actually
//! expand to, and what the list claims a member is.
//!
//! Both passes that read the list — ANF lowering and the AOT interpreted
//! preamble — run on the EXPANDED form. A surface macro that starts expanding
//! through a primitive the list does not name is a silent miscompile: the form
//! lowers as a generic call and the program returns nil with exit 0. Nothing
//! else in the tree fails when that happens, which is why it is asserted here.

use std::collections::HashSet;
use std::sync::Arc;

use cljrs_ir::lower::{DISPATCH_FAMILY, in_dispatch_family};
use cljrs_reader::Parser;
use cljrs_reader::form::{Form, FormKind};
use cljrs_runtime::env::env::{Env, GlobalEnv};

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

/// Every symbol in head position anywhere in `src`'s full expansion.
fn expansion_heads(src: &str) -> HashSet<String> {
    let (_globals, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut heads = HashSet::new();
    for form in forms {
        let expanded =
            cljrs_runtime::interp::macros::macroexpand_all(&form, &mut env).expect("expand error");
        collect_heads(&expanded, &mut heads);
    }
    heads
}

fn collect_heads(form: &Form, out: &mut HashSet<String>) {
    match &form.kind {
        FormKind::List(parts) => {
            if let Some(FormKind::Symbol(s)) = parts.first().map(|f| &f.kind) {
                out.insert(s.to_string());
            }
            for p in parts {
                collect_heads(p, out);
            }
        }
        FormKind::Vector(elems) | FormKind::Set(elems) | FormKind::Map(elems) => {
            for e in elems {
                collect_heads(e, out);
            }
        }
        _ => {}
    }
}

#[test]
fn each_surface_form_expands_through_a_primitive_the_family_names() {
    for (src, primitive) in [
        ("(deftype T [x])", "deftype*"),
        ("(defrecord R [x])", "deftype*"),
        ("(reify)", "deftype*"),
        ("(defprotocol P (m [_]))", "protocol*"),
    ] {
        let heads = expansion_heads(src);
        assert!(
            heads.contains(primitive),
            "{src} no longer expands through `{primitive}` — \
             whatever it expands through now must be added to DISPATCH_FAMILY"
        );
        assert!(
            in_dispatch_family(primitive),
            "`{primitive}` is reached by expansion but is not a DISPATCH_FAMILY member"
        );
    }
}

#[test]
fn every_head_reached_by_expanding_a_surface_form_is_a_member_or_lowerable() {
    // The general form of the invariant: expanding a family member may only
    // introduce heads the lowerer already understands, or further members.
    // `let*`/`fn*`/`loop*` are the ordinary lowerable primitives; anything
    // else ending in `*` is a datatype primitive and must be a member.
    const LOWERABLE_PRIMITIVES: &[&str] = &["let*", "fn*", "loop*"];

    for src in [
        "(deftype T [x])",
        "(defrecord R [x])",
        "(reify)",
        "(defprotocol P (m [_]))",
        "(defmulti mm :kind)",
    ] {
        for head in expansion_heads(src) {
            if !head.ends_with('*') || LOWERABLE_PRIMITIVES.contains(&head.as_str()) {
                continue;
            }
            assert!(
                in_dispatch_family(&head),
                "expanding {src} reaches the primitive `{head}`, \
                 which is not a DISPATCH_FAMILY member"
            );
        }
    }
}

#[test]
fn the_family_names_both_spellings_of_every_macro_backed_form() {
    // A surface name without its primitive (or the reverse) is the shape of
    // the hole: one pass reads the form before expansion, the other after.
    for (surface, primitive) in [
        ("deftype", "deftype*"),
        ("defrecord", "deftype*"),
        ("reify", "deftype*"),
        ("defprotocol", "protocol*"),
    ] {
        assert!(
            DISPATCH_FAMILY.contains(&surface),
            "surface name `{surface}` is missing from DISPATCH_FAMILY"
        );
        assert!(
            DISPATCH_FAMILY.contains(&primitive),
            "primitive `{primitive}` is missing from DISPATCH_FAMILY"
        );
    }
}
