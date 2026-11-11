//! End-to-end checks of the native-code trust gate (issue #476), driving the
//! real `cljrs` binary.
//!
//! The project under test is the issue's reproduction: a `cljrs.edn` whose
//! `:rust` section names a shared library with a constructor that announces
//! itself on stderr, and an init symbol that does not exist. If the library is
//! ever opened the constructor runs, whatever happens afterwards.
//!
//! `cargo` is replaced on `PATH` by a script that records every invocation,
//! so the tests can also assert that no toolchain process is started in an
//! untrusted project's directory.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BANNER: &str = "NATIVE CODE EXECUTED";

/// A hostile project, an isolated home directory, and a recording `cargo`.
struct Fixture {
    _tmp: tempfile::TempDir,
    /// The project root (holds `cljrs.edn`).
    project: PathBuf,
    /// A subdirectory of the project, used as the working directory so that
    /// the config is found by the ancestor walk.
    subdir: PathBuf,
    home: PathBuf,
    fake_bin: PathBuf,
    cargo_log: PathBuf,
}

impl Fixture {
    /// Build the fixture, or `None` when there is no C compiler to produce
    /// the probe library with.
    fn new() -> Option<Fixture> {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let project = base.join("project");
        let subdir = project.join("subdir");
        let home = base.join("home");
        let fake_bin = base.join("bin");
        let cargo_log = base.join("cargo.log");
        let lib_dir = project.join("target").join("debug");
        for d in [&subdir, &home, &fake_bin, &lib_dir] {
            std::fs::create_dir_all(d).unwrap();
        }

        std::fs::write(
            project.join("cljrs.edn"),
            r#"{:rust {:crate "." :init "probe::missing_symbol"}}"#,
        )
        .unwrap();

        let src = base.join("probe.c");
        std::fs::write(
            &src,
            format!(
                "#include <stdio.h>\n\
                 __attribute__((constructor)) static void loaded(void) {{\n\
                 fputs(\"{BANNER}\\n\", stderr);\n}}\n"
            ),
        )
        .unwrap();
        let lib_name = if cfg!(target_os = "macos") {
            "libprobe.dylib"
        } else {
            "libprobe.so"
        };
        let built = Command::new("cc")
            .args(["-shared", "-fPIC"])
            .arg(&src)
            .arg("-o")
            .arg(lib_dir.join(lib_name))
            .status();
        if !built.is_ok_and(|s| s.success()) {
            eprintln!("skipped: no working C compiler (`cc`)");
            return None;
        }

        // A stand-in `cargo`: logs its arguments, succeeds for `build` (so
        // `cljrs build-native` completes) and fails for everything else (so
        // `cargo metadata` falls back to `<crate>/target`).
        let cargo = fake_bin.join("cargo");
        std::fs::write(
            &cargo,
            format!(
                "#!/bin/sh\necho \"$@\" >> '{}'\n[ \"$1\" = build ] && exit 0\nexit 1\n",
                cargo_log.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();

        Some(Fixture {
            _tmp: tmp,
            project,
            subdir,
            home,
            fake_bin,
            cargo_log,
        })
    }

    /// A `cljrs` invocation from the project's subdirectory, with the
    /// isolated home, the recording `cargo`, and no inherited trust settings.
    fn cljrs(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cljrs"));
        let path = std::env::join_paths(
            std::iter::once(self.fake_bin.clone())
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        cmd.current_dir(&self.subdir)
            .env("HOME", &self.home)
            .env("PATH", path)
            .env_remove("CLJRS_NATIVE")
            .env_remove("CLJRS_NO_NATIVE")
            .env_remove("CLJRS_NO_CONFIG")
            .env_remove("CLJRS_TRUSTED_PROJECTS");
        cmd
    }

    fn eval(&self, pre_args: &[&str], expr: &str) -> Run {
        Run::of(self.cljrs().args(pre_args).args(["eval", expr]))
    }

    fn cargo_was_run(&self) -> bool {
        self.cargo_log.exists()
    }
}

/// The captured result of one `cljrs` invocation.
struct Run {
    stdout: String,
    stderr: String,
    success: bool,
}

impl Run {
    fn of(cmd: &mut Command) -> Run {
        let Output {
            status,
            stdout,
            stderr,
        } = cmd.output().expect("failed to run cljrs");
        Run {
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
            success: status.success(),
        }
    }

    fn loaded_native(&self) -> bool {
        self.stderr.contains(BANNER)
    }
}

fn trust_file(home: &Path) -> PathBuf {
    home.join(".cljrs").join("trusted-projects")
}

/// The issue as filed: a harmless `eval` beneath a hostile config must not
/// open the library, must not start `cargo`, must still evaluate, and must
/// say why the native code was skipped.
#[test]
fn an_untrusted_project_loads_no_native_code() {
    let Some(fx) = Fixture::new() else { return };

    let run = fx.eval(&[], "(+ 1 2)");
    assert!(run.success, "{}", run.stderr);
    assert_eq!(run.stdout.trim(), "3");
    assert!(!run.loaded_native(), "{}", run.stderr);
    assert!(
        !fx.cargo_was_run(),
        "cargo was started in an untrusted project"
    );
    assert!(run.stderr.contains("not trusted"), "{}", run.stderr);
    assert!(run.stderr.contains("cljrs trust"), "{}", run.stderr);

    // The load used to happen before the expression was even parsed.
    let run = fx.eval(&[], "(");
    assert!(!run.success);
    assert!(!run.loaded_native(), "{}", run.stderr);
    assert!(!fx.cargo_was_run());
}

/// `--native` and `CLJRS_NATIVE` allow it for one run, and leave no trust
/// behind.
#[test]
fn the_native_flag_allows_a_single_run() {
    let Some(fx) = Fixture::new() else { return };

    let run = fx.eval(&["--native"], "(+ 1 2)");
    assert_eq!(run.stdout.trim(), "3");
    assert!(run.loaded_native(), "{}", run.stderr);
    // The recording cargo is what `native_lib_path` consulted, which shows
    // the negative assertion in the untrusted case is a real one.
    assert!(fx.cargo_was_run());

    let run = Run::of(fx.cljrs().env("CLJRS_NATIVE", "1").args(["eval", "1"]));
    assert!(run.loaded_native(), "{}", run.stderr);

    assert!(!trust_file(&fx.home).exists());
    assert!(!fx.eval(&[], "1").loaded_native());
}

/// `cljrs trust` (run from a subdirectory) allows it durably; `--no-native`
/// and `CLJRS_NO_NATIVE` still switch it off; `cljrs untrust` revokes it.
#[test]
fn trust_is_recorded_per_project_and_can_be_overridden_and_revoked() {
    let Some(fx) = Fixture::new() else { return };

    let run = Run::of(fx.cljrs().arg("trust"));
    assert!(run.success, "{}", run.stderr);
    let listed = Run::of(fx.cljrs().args(["trust", "--list"]));
    assert_eq!(listed.stdout.trim(), fx.project.display().to_string());

    let run = fx.eval(&[], "(+ 1 2)");
    assert_eq!(run.stdout.trim(), "3");
    assert!(run.loaded_native(), "{}", run.stderr);

    let run = fx.eval(&["--no-native"], "(+ 1 2)");
    assert_eq!(run.stdout.trim(), "3");
    assert!(!run.loaded_native(), "{}", run.stderr);
    // The user asked for this, so there is nothing to explain.
    assert!(!run.stderr.contains("not trusted"), "{}", run.stderr);

    // The environment off switch beats the environment on switch.
    let run = Run::of(
        fx.cljrs()
            .env("CLJRS_NO_NATIVE", "1")
            .env("CLJRS_NATIVE", "1")
            .args(["eval", "1"]),
    );
    assert!(!run.loaded_native(), "{}", run.stderr);

    let run = Run::of(fx.cljrs().arg("untrust"));
    assert!(run.success, "{}", run.stderr);
    assert!(!fx.eval(&[], "1").loaded_native());
    assert!(
        Run::of(fx.cljrs().args(["trust", "--list"]))
            .stdout
            .trim()
            .is_empty()
    );
}

/// `CLJRS_TRUSTED_PROJECTS` grants trust without a store, and only to the
/// roots it lists.
#[test]
fn the_environment_list_grants_trust() {
    let Some(fx) = Fixture::new() else { return };

    let run = Run::of(
        fx.cljrs()
            .env("CLJRS_TRUSTED_PROJECTS", &fx.project)
            .args(["eval", "1"]),
    );
    assert!(run.loaded_native(), "{}", run.stderr);

    // Trusting the subdirectory (or any other directory) is not trusting
    // the project.
    let run = Run::of(
        fx.cljrs()
            .env("CLJRS_TRUSTED_PROJECTS", &fx.subdir)
            .args(["eval", "1"]),
    );
    assert!(!run.loaded_native(), "{}", run.stderr);
}

/// A successful `cljrs build-native` records the project as trusted and says
/// so.
#[test]
fn build_native_grants_trust() {
    let Some(fx) = Fixture::new() else { return };
    assert!(!fx.eval(&[], "1").loaded_native());

    let run = Run::of(fx.cljrs().arg("build-native"));
    assert!(run.success, "{}", run.stderr);
    assert!(run.stderr.contains("trusted"), "{}", run.stderr);
    assert!(run.stderr.contains("cljrs untrust"), "{}", run.stderr);

    assert!(fx.eval(&[], "1").loaded_native());
}

/// `--no-config` / `CLJRS_NO_CONFIG` skip discovery entirely: the config is
/// not read (a broken one draws no warning) and nothing native is loaded even
/// for a trusted project.
#[test]
fn no_config_skips_discovery() {
    let Some(fx) = Fixture::new() else { return };
    assert!(Run::of(fx.cljrs().arg("trust")).success);

    let run = fx.eval(&["--no-config"], "(+ 1 2)");
    assert_eq!(run.stdout.trim(), "3");
    assert!(!run.loaded_native(), "{}", run.stderr);

    std::fs::write(fx.project.join("cljrs.edn"), "{:paths").unwrap();
    let run = fx.eval(&[], "(+ 1 2)");
    assert!(run.stderr.contains("cljrs.edn"), "{}", run.stderr);
    let run = Run::of(
        fx.cljrs()
            .env("CLJRS_NO_CONFIG", "1")
            .args(["eval", "(+ 1 2)"]),
    );
    assert_eq!(run.stdout.trim(), "3");
    assert!(!run.stderr.contains("cljrs.edn"), "{}", run.stderr);
}

/// A config owned by another user is ignored with a warning, even with
/// `--native`; trusting its project lifts that. Handing a file to another uid
/// needs root, so the test is skipped otherwise.
#[test]
fn a_config_owned_by_another_user_is_ignored() {
    // `id -u` rather than a libc call: this test crate has no libc dependency.
    let uid = Command::new("id").arg("-u").output().unwrap().stdout;
    if String::from_utf8_lossy(&uid).trim() != "0" {
        eprintln!("skipped: needs root to chown");
        return;
    }
    let Some(fx) = Fixture::new() else { return };
    std::os::unix::fs::chown(fx.project.join("cljrs.edn"), Some(65534), None).unwrap();

    let run = fx.eval(&["--native"], "(+ 1 2)");
    assert_eq!(run.stdout.trim(), "3");
    assert!(!run.loaded_native(), "{}", run.stderr);
    assert!(
        run.stderr.contains("owned by another user"),
        "{}",
        run.stderr
    );

    assert!(Run::of(fx.cljrs().arg("trust")).success);
    let run = fx.eval(&[], "(+ 1 2)");
    assert!(run.loaded_native(), "{}", run.stderr);
    assert!(
        !run.stderr.contains("owned by another user"),
        "{}",
        run.stderr
    );
}

/// The same gate covers `:rust/load :dylib` dependencies: requiring a
/// namespace such a dependency provides starts no build in an untrusted
/// project, and fails with the trust message rather than "not found".
#[test]
fn an_untrusted_project_builds_no_native_dependency() {
    let Some(fx) = Fixture::new() else { return };
    std::fs::create_dir_all(fx.project.join("dep")).unwrap();
    std::fs::write(
        fx.project.join("cljrs.edn"),
        r#"{:deps {pinlib {:local/root "dep" :rust/load :dylib :rust/init "pinlib::init"}}}"#,
    )
    .unwrap();

    let run = fx.eval(&[], "(require 'pinlib)");
    assert!(!run.success);
    assert!(run.stderr.contains("not trusted"), "{}", run.stderr);
    assert!(
        !fx.cargo_was_run(),
        "cargo was started in an untrusted project"
    );

    // Off switch: refused as disabled, still without a build.
    let run = fx.eval(&["--no-native"], "(require 'pinlib)");
    assert!(!run.success);
    assert!(run.stderr.contains("disabled"), "{}", run.stderr);
    assert!(!fx.cargo_was_run());
}
