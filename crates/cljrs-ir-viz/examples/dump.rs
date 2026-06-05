//! Tiny example: lower a snippet, optimize, and write the visualizer HTML
//! to stdout.  Run with:
//!   cargo run -p cljrs-ir-viz --example dump > /tmp/ir.html

use cljrs_ir::lower::{lower_fn_body, optimize};
use cljrs_ir_viz::{RenderOptions, render_html};
use cljrs_reader::Parser;

fn main() {
    let src = r#"
(defn maybe-tag [x]
  (let [v [1 2 x]
        m {:tag :ok :v v}]
    (count v)))

(let [pure [1 2 3]
      esc  [4 5 6]]
  (println (count pure))
  esc)
"#;
    let mut parser = Parser::new(src.to_string(), "<example>".to_string());
    let forms = parser.parse_all().unwrap();
    let ir = lower_fn_body(Some("__example"), "user", &[], &forms, false).unwrap();
    let ir = optimize(ir);
    let html = render_html(
        &ir,
        Some(src),
        &RenderOptions {
            title: Some("dump example".into()),
        },
    );
    print!("{html}");
}
