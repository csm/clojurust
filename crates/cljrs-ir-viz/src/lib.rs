//! HTML visualizer for clojurust IR.
//!
//! Generates a single self-contained HTML file (inline CSS + JS, no external
//! deps) that shows a function's IR alongside its source, color-coded by
//! the region-allocation optimizer's output:
//!
//! * Each `RegionStart`/`RegionEnd` pair gets a deterministic color; every
//!   instruction inside the region — and every source line that produced
//!   one of its `RegionAlloc` insts — is tinted with that color.
//! * Allocations that *didn't* get promoted to a region are flagged with
//!   their escape-analysis verdict and a "blamed" use, making it obvious
//!   why the optimizer left them on the GC heap.
//!
//! Hover any IR instruction to highlight its source line; hover any source
//! line to highlight related IR instructions.

mod blame;
mod region;
mod render;

pub use render::{RenderOptions, render_html};
