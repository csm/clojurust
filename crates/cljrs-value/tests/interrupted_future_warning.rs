//! Discarding an unread interrupted future is silent by default and warns
//! when `CLJRS_WARN_INTERRUPTED_FUTURES` opts in.

use cljrs_value::{CljxFuture, FutureState};

const CHILD: &str = "CLJRS_INTERRUPTED_WARNING_CHILD";
const WARNING: &str = "an interrupted future was discarded";

fn child_stderr(setting: Option<&str>) -> String {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "interrupted_futures_warn_only_when_enabled",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env_remove("CLJRS_WARN_INTERRUPTED_FUTURES");
    if let Some(value) = setting {
        command.env("CLJRS_WARN_INTERRUPTED_FUTURES", value);
    }
    let output = command.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{stderr}");
    stderr
}

#[test]
fn interrupted_futures_warn_only_when_enabled() {
    if std::env::var_os(CHILD).is_some() {
        let future = CljxFuture::new();
        *future.state.lock().unwrap() = FutureState::Interrupted;
        drop(future);
        return;
    }
    for (setting, warns) in [
        (None, false),
        (Some("0"), false),
        (Some("1"), true),
        (Some("true"), true),
    ] {
        let stderr = child_stderr(setting);
        assert_eq!(stderr.contains(WARNING), warns, "{setting:?}: {stderr}");
        // Never reported as an ordinary failed future.
        assert!(
            !stderr.contains("a failed future was discarded"),
            "{stderr}"
        );
    }
}
