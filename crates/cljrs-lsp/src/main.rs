//! Standalone `cljrs-lsp` binary: runs the language server over stdio.

#[tokio::main]
async fn main() {
    cljrs_lsp::run_stdio().await;
}
