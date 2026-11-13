//! `cljrs trust` / `cljrs untrust` — manage which projects may load native
//! code (see [`crate::native::check`] and [`cljrs_project::trust`]).

use std::path::PathBuf;

use cljrs_project::trust::TrustStore;
use miette::IntoDiagnostic as _;

#[derive(clap::Args)]
pub struct Args {
    /// The project root to trust. Defaults to the directory of the nearest
    /// `cljrs.edn` in or above the current directory.
    pub dir: Option<PathBuf>,

    /// List the trusted project roots instead of adding one.
    #[arg(long, conflicts_with = "dir")]
    pub list: bool,
}

#[derive(clap::Args)]
pub struct UntrustArgs {
    /// The project root to stop trusting. Defaults to the directory of the
    /// nearest `cljrs.edn` in or above the current directory.
    pub dir: Option<PathBuf>,
}

/// The project root a command applies to: `dir` if given, otherwise the
/// directory of the nearest `cljrs.edn`.
///
/// This locates the config without applying the ownership check — trusting a
/// project is exactly how that check is lifted.
fn resolve_root(dir: Option<PathBuf>) -> miette::Result<PathBuf> {
    if let Some(dir) = dir {
        return Ok(dir);
    }
    let cwd = std::env::current_dir().into_diagnostic()?;
    cljrs_project::config::find_config_file(&cwd)
        .and_then(|config| config.parent().map(PathBuf::from))
        .ok_or_else(|| {
            miette::miette!(
                "no cljrs.edn found in or above the current directory; \
                 name the project root explicitly: `cljrs trust <DIR>`"
            )
        })
}

/// `cljrs trust [DIR]` / `cljrs trust --list`.
pub fn run(args: Args) -> miette::Result<i32> {
    let mut store = TrustStore::open().into_diagnostic()?;
    if args.list {
        for root in store.roots() {
            println!("{}", root.display());
        }
        return Ok(0);
    }

    let root = resolve_root(args.dir)?;
    let (canonical, added) = store
        .add(&root)
        .map_err(|e| miette::miette!("cannot trust {}: {e}", root.display()))?;
    if !added {
        eprintln!("cljrs: {} is already trusted", canonical.display());
        return Ok(0);
    }
    store.save().into_diagnostic()?;
    if !canonical.join("cljrs.edn").exists() {
        eprintln!(
            "cljrs: note: {} has no cljrs.edn; trust applies to the directory holding one",
            canonical.display()
        );
    }
    eprintln!(
        "cljrs: trusted {} — native code its cljrs.edn declares will be built and loaded",
        canonical.display()
    );
    Ok(0)
}

/// `cljrs untrust [DIR]`.
pub fn run_untrust(args: UntrustArgs) -> miette::Result<i32> {
    let mut store = TrustStore::open().into_diagnostic()?;
    let root = resolve_root(args.dir)?;
    if store.remove(&root) {
        store.save().into_diagnostic()?;
        eprintln!("cljrs: no longer trusting {}", root.display());
    } else {
        eprintln!("cljrs: {} was not trusted", root.display());
    }
    Ok(0)
}
