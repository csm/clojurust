//! `cljrs run` — interpret a source file, then call its `-main`.

use std::path::PathBuf;

use crate::session::{self, VersioningFlags};

#[derive(clap::Args)]
pub struct Args {
    /// Path to the source file.
    pub file: PathBuf,
    /// Source directories to search when resolving `require`.
    #[arg(long = "src-path", value_name = "DIR")]
    pub src_paths: Vec<PathBuf>,
    /// GC soft memory limit in MB (each isolate heap collects when it exceeds this).
    #[arg(long)]
    pub gc_soft_limit_mb: Option<usize>,
    /// Process managed-memory hard limit in MB (enforced only with CLJRS_MEMORY_STRICT=1).
    #[arg(long)]
    pub gc_hard_limit_mb: Option<usize>,
    /// Arguments forwarded to -main (everything after `--`).
    #[arg(last = true, value_name = "ARGS")]
    pub args: Vec<String>,
}

pub fn run(args: Args, versioning: VersioningFlags) -> miette::Result<i32> {
    let src = std::fs::read_to_string(&args.file)
        .map_err(|e| miette::miette!("{}: {}", args.file.display(), e))?;
    let filename = args.file.display().to_string();
    let gc_config = session::build_gc_config(args.gc_soft_limit_mb, args.gc_hard_limit_mb)?;
    let globals = session::setup_globals(args.src_paths, gc_config, versioning);
    session::run_source(&src, &filename, globals, &args.args)?;
    Ok(0)
}
