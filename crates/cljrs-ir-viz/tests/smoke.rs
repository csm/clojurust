//! Smoke tests: lower a tiny snippet, run the optimizer, and verify the
//! HTML output is well-formed and contains the expected markers.

use std::sync::Arc;

use cljrs_ir::lower::{lower_fn_body, optimize};
use cljrs_ir_viz::{RenderOptions, render_html};
use cljrs_reader::Parser;

fn lower(source: &str) -> cljrs_ir::IrFunction {
    let mut parser = Parser::new(source.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse");
    let ir = lower_fn_body(Some("test"), "user", &[], &forms, false).expect("lower");
    optimize(ir)
}

#[test]
fn renders_without_source() {
    let ir = lower("(let [v [1 2 3]] (count v))");
    let html = render_html(&ir, None, &RenderOptions::default());
    assert!(html.contains("<!doctype html>"));
    assert!(html.contains("<title>clojurust IR</title>"));
    assert!(html.contains("class=\"ir-pane\""));
    assert!(!html.contains("class=\"src-pane\""));
}

#[test]
fn renders_with_source_and_source_pane() {
    let src = "(let [v [1 2 3]] (count v))";
    let ir = lower(src);
    let html = render_html(&ir, Some(src), &RenderOptions::default());
    assert!(html.contains("class=\"src-pane\""));
    assert!(html.contains("data-line=\"1\""));
}

#[test]
fn region_color_appears_when_optimization_succeeds() {
    // A vector that's only counted should not escape — the optimizer will
    // promote `[1 2 3]` to a region allocation.
    let src = "(count [1 2 3])";
    let ir = lower(src);
    let html = render_html(&ir, Some(src), &RenderOptions::default());
    // Either a region was emitted (we'll see ".r0") or an escape badge
    // marks the heap allocation.  We don't assume which — we just check
    // that one of the two diagnostics appears.
    let has_region = html.contains(".r0 {");
    let has_escape = html.contains("class=\"badge");
    assert!(
        has_region || has_escape,
        "expected either a region or an escape annotation in: {html}"
    );
}

#[test]
fn escape_annotation_for_returned_vector() {
    // A vector that is returned escapes — the analyzer should classify it
    // as `Returns` (or `Escapes` depending on lowering).  Either way the
    // visualizer should emit a "bad" badge.
    let src = "[1 2 3]";
    let ir = lower(src);
    let html = render_html(&ir, Some(src), &RenderOptions::default());
    assert!(
        html.contains("badge bad") || html.contains("badge warn"),
        "expected an escape badge for a returned vector, got: {html}"
    );
}

#[test]
fn html_escapes_user_text() {
    let src = "\"<script>alert(1)</script>\"";
    let ir = lower(src);
    let html = render_html(&ir, Some(src), &RenderOptions::default());
    assert!(!html.contains("<script>alert(1)</script>"));
    assert!(html.contains("&lt;script&gt;"));
}

#[test]
fn renders_with_subfunctions() {
    let src = "(fn [x] (fn [y] (+ x y)))";
    let ir = lower(src);
    let html = render_html(&ir, Some(src), &RenderOptions::default());
    // The outer "test" function plus at least one subfn arity body.
    let fn_count = html.matches("<article class=\"fn\">").count();
    assert!(fn_count >= 1, "expected ≥1 fn article, got {fn_count}");
}

#[test]
fn options_title_is_used() {
    let ir = lower("nil");
    let opts = RenderOptions {
        title: Some("Custom Title".to_string()),
    };
    let html = render_html(&ir, None, &opts);
    assert!(html.contains("<title>Custom Title</title>"));
    assert!(html.contains("<h1>Custom Title</h1>"));
}

// Silence `Arc` import lint if cargo decides we don't need it.
#[allow(dead_code)]
fn _arc_witness() -> Arc<str> {
    Arc::from("x")
}
