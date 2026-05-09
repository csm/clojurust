//! Single-file HTML rendering.

use std::collections::HashMap;
use std::fmt::Write as _;

use cljrs_ir::lower::{AnalysisResult, EscapeContext, EscapeState, analyze, make_analysis_context};
use cljrs_ir::{Block, BlockId, Const, Inst, IrFunction, Terminator, VarId};
use cljrs_types::span::Span;

use crate::blame::{blame_use, state_label, use_kind_label};
use crate::region::{Region, collect_regions, membership_map};

/// Options for HTML rendering.
#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    /// Page title.  Defaults to `"clojurust IR"`.
    pub title: Option<String>,
}

/// Render `ir` (and all its subfunctions) to a single self-contained HTML
/// document.  When `source` is provided, the source pane is shown alongside
/// the IR with hover-linked highlighting; when `None`, only the IR pane is
/// shown.
pub fn render_html(ir: &IrFunction, source: Option<&str>, opts: &RenderOptions) -> String {
    let title = opts.title.as_deref().unwrap_or("clojurust IR");
    let ctx = make_analysis_context(ir);

    let mut ir_html = String::new();
    let mut all_regions: Vec<RegionRef> = Vec::new();
    let mut next_region_idx: usize = 0;
    render_function_tree(
        ir,
        &ctx,
        &mut ir_html,
        &mut all_regions,
        &mut next_region_idx,
    );

    let source_html = source
        .map(|s| render_source_pane(s, &all_regions))
        .unwrap_or_default();

    let region_css = render_region_css(&all_regions);
    let layout_class = if source.is_some() {
        "two-pane"
    } else {
        "ir-only"
    };

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>{title}</title>
<style>
{base_css}
{region_css}
</style>
</head>
<body class="{layout_class}">
<header><h1>{title}</h1>
<p class="hint">Hover IR instructions to highlight their source line; hover source lines to highlight related IR. Region-allocated insts share a color with their source range; non-promoted allocations show their escape state.</p>
</header>
<main>
{source_block}<section class="ir-pane">
{ir_html}
</section>
</main>
<script>
{script}
</script>
</body>
</html>
"#,
        title = html_escape(title),
        layout_class = layout_class,
        base_css = BASE_CSS,
        region_css = region_css,
        source_block = if source.is_some() {
            format!("<section class=\"src-pane\">{source_html}</section>")
        } else {
            String::new()
        },
        ir_html = ir_html,
        script = SCRIPT,
    )
}

// ── Region color assignment ─────────────────────────────────────────────────

/// A region paired with the global index used for color assignment.
#[derive(Debug, Clone)]
struct RegionRef {
    /// Globally unique index across all functions in the IR tree.
    idx: usize,
    /// Source-line set for this region (collected from SourceLoc markers
    /// of region-allocated insts).
    source_lines: Vec<u32>,
}

fn region_bg(idx: usize) -> String {
    // Golden-angle hue spacing for distinct, deterministic colors.  Use a
    // very pale tint so it doesn't overwhelm the IR text.
    let hue = (idx as f64 * 137.508_f64) % 360.0;
    format!("hsl({hue:.1}, 70%, 94%)")
}

fn region_accent(idx: usize) -> String {
    let hue = (idx as f64 * 137.508_f64) % 360.0;
    format!("hsl({hue:.1}, 55%, 55%)")
}

fn region_strong_bg(idx: usize) -> String {
    let hue = (idx as f64 * 137.508_f64) % 360.0;
    format!("hsl({hue:.1}, 70%, 85%)")
}

fn render_region_css(regions: &[RegionRef]) -> String {
    let mut s = String::new();
    for r in regions {
        // `.rN`         — pale tint for in-region insts and source lines
        // `.rN-strong`  — accent border + denser tint for the actual
        //                 `RegionAlloc` / `RegionStart` / `RegionEnd` insts
        writeln!(
            s,
            ".r{idx} {{ background: {bg}; }}\n.r{idx}-strong {{ background: {sbg}; border-left: 3px solid {ac}; }}\n.src-line.r{idx} {{ box-shadow: inset 3px 0 0 {ac}; }}",
            idx = r.idx,
            bg = region_bg(r.idx),
            sbg = region_strong_bg(r.idx),
            ac = region_accent(r.idx),
        )
        .unwrap();
    }
    s
}

// ── Source pane ─────────────────────────────────────────────────────────────

fn render_source_pane(source: &str, regions: &[RegionRef]) -> String {
    let mut by_line: HashMap<u32, Vec<usize>> = HashMap::new();
    for r in regions {
        for &line in &r.source_lines {
            by_line.entry(line).or_default().push(r.idx);
        }
    }

    let mut html = String::from("<pre class=\"src\">");
    for (i, line) in source.lines().enumerate() {
        let line_no = (i + 1) as u32;
        let active = by_line.get(&line_no);
        let region_class = active
            .and_then(|v| v.last())
            .map(|idx| format!("r{idx}"))
            .unwrap_or_default();
        let region_data = active
            .map(|v| {
                v.iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        writeln!(
            html,
            "<span class=\"src-line {cls}\" data-line=\"{n}\" data-regions=\"{regions}\"><span class=\"ln\">{n:>4}</span> {body}</span>",
            cls = region_class,
            n = line_no,
            regions = region_data,
            body = html_escape(line)
        )
        .unwrap();
    }
    html.push_str("</pre>");
    html
}

// ── IR pane ─────────────────────────────────────────────────────────────────

fn render_function_tree(
    ir: &IrFunction,
    ctx: &EscapeContext,
    out: &mut String,
    regions_out: &mut Vec<RegionRef>,
    next_region_idx: &mut usize,
) {
    render_one_function(ir, ctx, "", out, regions_out, next_region_idx);
    for sub in &ir.subfunctions {
        render_one_function(
            sub,
            ctx,
            ir.name.as_deref().unwrap_or("<anon>"),
            out,
            regions_out,
            next_region_idx,
        );
    }
}

fn render_one_function(
    ir: &IrFunction,
    ctx: &EscapeContext,
    parent_path: &str,
    out: &mut String,
    regions_out: &mut Vec<RegionRef>,
    next_region_idx: &mut usize,
) {
    let analysis = analyze(ir, Some(ctx));
    let regions = collect_regions(ir);
    let memb = membership_map(ir, &regions);

    // Assign each region a global index and collect source lines per region.
    let mut handle_to_global: HashMap<VarId, usize> = HashMap::new();
    let mut handle_to_lines: HashMap<VarId, Vec<u32>> = HashMap::new();

    let fn_label = ir.name.as_deref().unwrap_or("<anon>");
    let fn_path = if parent_path.is_empty() {
        fn_label.to_string()
    } else {
        format!("{parent_path} → {fn_label}")
    };

    for r in &regions {
        let idx = *next_region_idx;
        *next_region_idx += 1;
        handle_to_global.insert(r.handle, idx);
        // Find source lines for region-allocated insts inside this region.
        let lines = collect_region_alloc_lines(ir, r);
        handle_to_lines.insert(r.handle, lines.clone());
        regions_out.push(RegionRef {
            idx,
            source_lines: lines,
        });
    }

    let params = ir
        .params
        .iter()
        .map(|(n, v)| format!("{n}: {v}"))
        .collect::<Vec<_>>()
        .join(", ");

    let span_text = ir
        .span
        .as_ref()
        .map(|s| format!(" <small>{}:{}:{}</small>", s.file, s.line, s.col))
        .unwrap_or_default();

    writeln!(
        out,
        "<article class=\"fn\"><h2><span class=\"fn-name\">{name}</span>({params}){span}</h2>",
        name = html_escape(&fn_path),
        params = html_escape(&params),
        span = span_text,
    )
    .unwrap();

    let stats = compute_alloc_stats(ir);
    writeln!(
        out,
        "<p class=\"stats\"><span class=\"badge ok\">region-allocated: {region}</span> <span class=\"badge bad\">heap: {heap}</span> <span class=\"badge\">closures: {closure}</span></p>",
        region = stats.region,
        heap = stats.heap,
        closure = stats.closures,
    )
    .unwrap();

    for block in &ir.blocks {
        render_block(block, ir, &analysis, &memb, &handle_to_global, out);
    }

    out.push_str("</article>\n");
}

fn render_block(
    block: &Block,
    ir: &IrFunction,
    analysis: &AnalysisResult,
    memb: &HashMap<(BlockId, usize), Vec<&Region>>,
    handle_to_global: &HashMap<VarId, usize>,
    out: &mut String,
) {
    writeln!(
        out,
        "<section class=\"block\"><h3>{bid}:</h3><ol class=\"insts\">",
        bid = block.id
    )
    .unwrap();

    // Phis
    for phi in &block.phis {
        writeln!(
            out,
            "<li class=\"inst phi\">{}</li>",
            html_escape(&format!("{phi}"))
        )
        .unwrap();
    }

    // Track current source line for tying insts back to source.
    let mut current_line: Option<u32> = None;

    for (idx, inst) in block.insts.iter().enumerate() {
        if let Inst::SourceLoc(span) = inst {
            current_line = Some(span.line);
            // Render SourceLoc as a subtle marker but linked to source.
            writeln!(
                out,
                "<li class=\"inst loc\" data-line=\"{line}\"><span class=\"ix\">{ix}</span> # {file}:{line}:{col}</li>",
                line = span.line,
                col = span.col,
                file = html_escape(&span.file),
                ix = idx,
            )
            .unwrap();
            continue;
        }

        let line_attr = current_line
            .map(|l| format!(" data-line=\"{l}\""))
            .unwrap_or_default();

        // Region class — innermost wins.
        let region_idx_opt = memb
            .get(&(block.id, idx))
            .and_then(|stack| stack.first().and_then(|r| handle_to_global.get(&r.handle)));
        // Strong tint only for the actual region-alloc / region-start /
        // region-end markers — the rest of the block stays subtly tinted
        // so the IR text remains readable.
        let is_marker = matches!(
            inst,
            Inst::RegionAlloc(..) | Inst::RegionStart(..) | Inst::RegionEnd(..)
        );
        let region_class = region_idx_opt
            .map(|i| {
                if is_marker {
                    format!(" r{i}-strong")
                } else {
                    format!(" r{i}")
                }
            })
            .unwrap_or_default();
        let region_strong = String::new();

        let inst_text = render_inst(inst, ir);
        let escape_badge = format_escape_badge(inst, analysis);

        let kind_class = inst_kind_class(inst);

        writeln!(
            out,
            "<li class=\"inst {kind}{region}{strong}\"{attr}><span class=\"ix\">{ix}</span> {body}{badge}</li>",
            kind = kind_class,
            region = region_class,
            strong = region_strong,
            attr = line_attr,
            ix = idx,
            body = html_escape(&inst_text),
            badge = escape_badge,
        )
        .unwrap();
    }

    // Terminator
    writeln!(
        out,
        "<li class=\"inst term\">→ {}</li>",
        html_escape(&format!("{}", block.terminator))
    )
    .unwrap();

    // If this block is an end of any region, mark the wrap-up.
    if matches!(block.terminator, Terminator::Return(_)) {
        // nothing extra
    }
    let _ = ir; // unused for now but reserved
    let _ = analysis;
    let _ = memb;
    let _ = handle_to_global;
    out.push_str("</ol></section>\n");
}

fn inst_kind_class(inst: &Inst) -> &'static str {
    match inst {
        Inst::AllocVector(..)
        | Inst::AllocMap(..)
        | Inst::AllocSet(..)
        | Inst::AllocList(..)
        | Inst::AllocCons(..)
        | Inst::AllocClosure(..) => "alloc",
        Inst::RegionAlloc(..) => "ralloc",
        Inst::RegionStart(..) => "rstart",
        Inst::RegionEnd(..) => "rend",
        Inst::Call(..) | Inst::CallKnown(..) | Inst::CallDirect(..) => "call",
        Inst::DefVar(..) | Inst::SetBang(..) => "store",
        _ => "other",
    }
}

fn render_inst(inst: &Inst, _ir: &IrFunction) -> String {
    match inst {
        Inst::Const(dst, c) => format!("{dst} = const {}", render_const(c)),
        Inst::AllocVector(dst, elems) => format!("{dst} = alloc-vec {}", fmt_vars(elems)),
        Inst::AllocMap(dst, pairs) => {
            let body = pairs
                .iter()
                .map(|(k, v)| format!("{k} {v}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{dst} = alloc-map {{{body}}}")
        }
        Inst::AllocSet(dst, elems) => format!("{dst} = alloc-set {}", fmt_vars(elems)),
        Inst::AllocList(dst, elems) => format!("{dst} = alloc-list {}", fmt_vars(elems)),
        Inst::AllocCons(dst, h, t) => format!("{dst} = cons {h} {t}"),
        Inst::AllocClosure(dst, tmpl, captures) => format!(
            "{dst} = closure {:?} captures={}",
            tmpl.name,
            fmt_vars(captures),
        ),
        Inst::RegionAlloc(dst, region, kind, operands) => {
            format!(
                "{dst} = region-alloc {region} {kind} {}",
                fmt_vars(operands)
            )
        }
        Inst::RegionStart(handle) => format!("{handle} = region-start"),
        Inst::RegionEnd(handle) => format!("region-end {handle}"),
        _ => format!("{inst}"),
    }
}

fn fmt_vars(vs: &[VarId]) -> String {
    let s: Vec<String> = vs.iter().map(|v| v.to_string()).collect();
    format!("[{}]", s.join(", "))
}

fn render_const(c: &Const) -> String {
    match c {
        Const::Nil => "nil".into(),
        Const::Bool(b) => format!("{b}"),
        Const::Long(n) => format!("{n}"),
        Const::Double(f) => format!("{f}"),
        Const::Str(s) => format!("\"{s}\""),
        Const::Keyword(s) => format!(":{s}"),
        Const::Symbol(s) => format!("'{s}"),
        Const::Char(c) => format!("\\{c}"),
    }
}

fn format_escape_badge(inst: &Inst, analysis: &AnalysisResult) -> String {
    let dst = match inst {
        Inst::AllocVector(d, _)
        | Inst::AllocMap(d, _)
        | Inst::AllocSet(d, _)
        | Inst::AllocList(d, _)
        | Inst::AllocCons(d, _, _)
        | Inst::AllocClosure(d, _, _) => *d,
        _ => return String::new(),
    };

    let Some(state) = analysis.states.get(&dst).copied() else {
        return String::new();
    };

    let css = match state {
        EscapeState::NoEscape => "ok",
        EscapeState::ArgEscape => "warn",
        EscapeState::Returns | EscapeState::Escapes => "bad",
    };
    let label = state_label(state);

    let blame = analysis
        .uses
        .get(&dst)
        .and_then(|us| blame_use(us, state))
        .map(|u| format!(" ({} in {})", use_kind_label(&u.kind), u.block))
        .unwrap_or_default();

    format!(
        " <span class=\"badge {css}\">{label}{blame}</span>",
        css = css,
        label = label,
        blame = html_escape(&blame),
    )
}

// ── Source-line collection for regions ──────────────────────────────────────

fn collect_region_alloc_lines(ir: &IrFunction, region: &Region) -> Vec<u32> {
    let mut lines: Vec<u32> = Vec::new();
    let mut current_line: Option<u32> = None;

    for block in &ir.blocks {
        if !region.blocks.contains(&block.id) {
            continue;
        }
        let in_start_block = block.id == region.start_block;
        let in_end_block = block.id == region.end_block;

        for (idx, inst) in block.insts.iter().enumerate() {
            // Adjust current_line as we walk.
            if let Inst::SourceLoc(s) = inst {
                current_line = Some(s.line);
                continue;
            }

            // Is this inst within the region's window in this block?
            let within = if region.start_block == region.end_block {
                idx >= region.start_inst_idx && idx <= region.end_inst_idx
            } else if in_start_block {
                idx >= region.start_inst_idx
            } else if in_end_block {
                idx <= region.end_inst_idx
            } else {
                true
            };
            if !within {
                continue;
            }

            // Only count `RegionAlloc` insts whose handle matches *this*
            // region — otherwise nested regions would steal each other's
            // source lines.
            if let Inst::RegionAlloc(_, handle, _, _) = inst
                && *handle == region.handle
                && let Some(line) = current_line
                && !lines.contains(&line)
            {
                lines.push(line);
            }
        }
    }
    lines
}

// ── Stats ───────────────────────────────────────────────────────────────────

#[derive(Default)]
struct AllocStats {
    region: usize,
    heap: usize,
    closures: usize,
}

fn compute_alloc_stats(ir: &IrFunction) -> AllocStats {
    let mut s = AllocStats::default();
    for block in &ir.blocks {
        for inst in &block.insts {
            match inst {
                Inst::RegionAlloc(..) => s.region += 1,
                Inst::AllocVector(..)
                | Inst::AllocMap(..)
                | Inst::AllocSet(..)
                | Inst::AllocList(..)
                | Inst::AllocCons(..) => s.heap += 1,
                Inst::AllocClosure(..) => s.closures += 1,
                _ => {}
            }
        }
    }
    s
}

// ── Span helper (currently unused but kept for future inline-source view) ──

#[allow(dead_code)]
fn span_byte_range(span: &Span) -> (usize, usize) {
    (span.start, span.end)
}

// ── Escape ──────────────────────────────────────────────────────────────────

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

// ── Static assets ───────────────────────────────────────────────────────────

const BASE_CSS: &str = r#"
* { box-sizing: border-box; }
body { margin: 0; font-family: -apple-system, system-ui, sans-serif; color: #222; background: #fafafa; }
header { padding: 1rem 1.5rem; border-bottom: 1px solid #ddd; background: #fff; }
header h1 { margin: 0 0 0.25rem 0; font-size: 1.1rem; }
.hint { margin: 0; font-size: 0.85rem; color: #555; }
main { display: grid; gap: 0; }
body.two-pane main { grid-template-columns: minmax(0, 1fr) minmax(0, 1.4fr); height: calc(100vh - 70px); }
body.ir-only main { grid-template-columns: minmax(0, 1fr); }
.src-pane, .ir-pane { overflow: auto; padding: 0.5rem 0; }
.src-pane { border-right: 1px solid #ddd; background: #fdfdfd; }
.ir-pane { background: #fff; }
pre.src { margin: 0; font: 12px ui-monospace, Menlo, Consolas, monospace; }
.src-line { display: block; padding: 0 0.75rem; transition: background 60ms; }
.src-line .ln { display: inline-block; width: 3rem; text-align: right; color: #999; user-select: none; padding-right: 0.75rem; border-right: 1px solid #eee; margin-right: 0.5rem; }
.src-line.hl { background: #fff7c2 !important; outline: 1px solid #f1c40f; }
.fn { padding: 0.75rem 1rem; border-bottom: 1px solid #eee; }
.fn h2 { margin: 0 0 0.25rem 0; font-size: 0.95rem; font-family: ui-monospace, Menlo, monospace; }
.fn h2 small { color: #888; font-weight: normal; }
.stats { margin: 0 0 0.5rem 0; font-size: 0.75rem; }
.badge { display: inline-block; padding: 1px 6px; border-radius: 3px; background: #eee; color: #333; font-size: 0.7rem; margin-right: 0.25rem; vertical-align: middle; }
.badge.ok { background: #d6f5dc; color: #1d6033; }
.badge.warn { background: #fdebc8; color: #8a5a00; }
.badge.bad { background: #f8d2d0; color: #8a1f1a; }
.block { margin: 0.5rem 0; }
.block h3 { margin: 0; font: bold 0.8rem ui-monospace, Menlo, monospace; color: #444; }
ol.insts { margin: 0; padding: 0; list-style: none; font: 12px ui-monospace, Menlo, monospace; }
.inst { padding: 1px 0.5rem 1px 2.5rem; position: relative; transition: background 60ms; }
.inst .ix { position: absolute; left: 0.5rem; color: #aaa; user-select: none; font-size: 10px; width: 1.5rem; text-align: right; }
.inst.hl { background: #fff7c2 !important; outline: 1px solid #f1c40f; }
.inst.term { color: #555; font-style: italic; padding-left: 1rem; }
.inst.alloc { color: #8a3f00; }
.inst.ralloc { color: #1d6033; font-weight: 500; }
.inst.rstart, .inst.rend { color: #555; font-style: italic; font-size: 11px; }
.inst.call { color: #2a5a8a; }
.inst.store { color: #6a2a8a; }
.inst.loc { color: #aaa; font-size: 10px; padding-top: 0; padding-bottom: 0; }
"#;

const SCRIPT: &str = r#"
(function () {
  function highlight(line) {
    document.querySelectorAll('[data-line="' + line + '"]').forEach(function (el) {
      el.classList.add('hl');
    });
  }
  function clear() {
    document.querySelectorAll('.hl').forEach(function (el) { el.classList.remove('hl'); });
  }
  document.querySelectorAll('[data-line]').forEach(function (el) {
    el.addEventListener('mouseenter', function () { highlight(el.dataset.line); });
    el.addEventListener('mouseleave', clear);
  });
})();
"#;
