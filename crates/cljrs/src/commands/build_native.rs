//! `cljrs build-native` — build the project's own `:rust` crate as a shared
//! library, the one `cljrs run` and `cljrs repl` load at startup — and record
//! the project as trusted to load it.

use miette::IntoDiagnostic as _;

use crate::native;

#[derive(clap::Args)]
pub struct Args {
    /// Build in release mode instead of debug.
    #[arg(long)]
    pub release: bool,
}

/// Build the native Rust crate declared in `cljrs.edn` as a shared library.
pub fn run(args: Args) -> miette::Result<i32> {
    let release = args.release;
    let cwd = std::env::current_dir().into_diagnostic()?;
    let config = cljrs_project::config::load_config(&cwd)
        .into_diagnostic()?
        .ok_or_else(|| miette::miette!("no cljrs.edn found in or above the current directory"))?;

    let rust_config = config
        .rust
        .as_ref()
        .ok_or_else(|| miette::miette!("no :rust key found in cljrs.edn"))?;

    let crate_name = rust_config
        .crate_name()
        .ok_or_else(|| miette::miette!(":rust has no :init function; cannot derive crate name"))?;

    eprintln!(
        "[build-native] building {} in {}",
        crate_name,
        rust_config.crate_dir.display()
    );

    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("build");
    if release {
        cmd.arg("--release");
    }
    cmd.current_dir(&rust_config.crate_dir);

    let status = cmd.status().into_diagnostic()?;
    if !status.success() {
        return Err(miette::miette!("cargo build failed"));
    }

    let lib_path = native::native_lib_path(&rust_config.crate_dir, crate_name, release);
    eprintln!("[build-native] built {}", lib_path.display());
    println!("{}", lib_path.display());

    // Building a project's native crate is an explicit decision to run its
    // native code, so it also records the project as trusted to load it.
    // This is the only place a build grants trust; the gate itself
    // (`native::check`) knows nothing about it.
    if let Some(root) = config.root.as_deref() {
        trust_built_project(root);
    }

    Ok(0)
}

/// Record `root` in the trust store after a successful build, saying so when
/// it is newly added.  A failure to record is a warning, not a build failure:
/// the library is built, and `cljrs trust` or `--native` still work.
fn trust_built_project(root: &std::path::Path) {
    let recorded = cljrs_project::trust::TrustStore::open().and_then(|mut store| {
        let (canonical, added) = store.add(root)?;
        if added {
            store.save()?;
        }
        Ok((canonical, added))
    });
    match recorded {
        Ok((canonical, true)) => eprintln!(
            "[build-native] trusted {} to load native code (undo with `cljrs untrust`)",
            canonical.display()
        ),
        Ok((_, false)) => {}
        Err(e) => eprintln!(
            "[build-native] warning: could not record {} as trusted ({e}); \
             run `cljrs trust` or pass --native to load the library",
            root.display()
        ),
    }
}
