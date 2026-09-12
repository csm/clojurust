//! How the AOT harness depends on a project's Rust extension crate.
//!
//! Two different names meet at that dependency line, and they are not the same
//! string:
//!
//! - the **crate identifier** — what generated Rust *source* calls the crate
//!   (`cljrs_base64`). It is a Rust identifier, so it is underscored, and
//!   `cljrs.edn`'s `:rust :init` is where we read it from.
//! - the **package name** — what Cargo *resolves* the dependency by
//!   (`cljrs-base64`), declared as `name` under `[package]` in the crate's own
//!   `Cargo.toml`. Package names are hyphenated by community convention, and
//!   every crate in this workspace follows it.
//!
//! Cargo reads a dependency KEY as the package name unless an explicit
//! `package = "..."` says otherwise. Emitting the identifier as the key
//! therefore makes a hyphenated package unresolvable:
//!
//! ```text
//! error: no matching package named `my_plugin` found
//! help: packages with similar names: my-plugin
//! ```
//!
//! `package = "..."` is exactly the knob for this: the key stays the Rust
//! identifier the generated source uses, while Cargo resolves the real
//! package. So we emit both, and the two names stop being conflated.
//!
//! The strata below are kept apart on purpose — the parsing and formatting are
//! pure and directly testable, and one small function does the file read.

use std::path::Path;

// ── Promote: pure, data -> data ─────────────────────────────────────────────

/// The `name` declared under `[package]` in `cargo_toml`.
///
/// Deliberately narrow: it reads the one key this crate needs rather than
/// pulling in a TOML parser for it. It understands section headers, full-line
/// comments, and both quote styles, and it refuses to be fooled by a `name`
/// key belonging to some *other* section (`[lib]`, `[[bin]]`,
/// `[dependencies.foo]`) — which is the only way a naive scan gets this wrong
/// in a real manifest.
///
/// Returns `None` for anything it does not understand, and the caller then
/// keeps the pre-existing behaviour rather than guessing.
pub fn package_name(cargo_toml: &str) -> Option<&str> {
    let mut in_package = false;
    for raw in cargo_toml.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // A section header ends the previous section; `[[bin]]` too, whose
        // inner `[bin]` is not `package` either way.
        if let Some(rest) = line.strip_prefix('[') {
            let header = rest.split(']').next().unwrap_or("").trim();
            in_package = header == "package";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "name" {
            continue;
        }
        return unquote(value.trim());
    }
    None
}

/// The contents of a leading quoted string, ignoring anything after it.
///
/// Stopping at the closing quote is what lets the scanner leave trailing
/// comments alone without having to track quote state across the whole line.
fn unquote(value: &str) -> Option<&str> {
    let quote = value.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &value[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(&rest[..end])
}

/// The `[dependencies]` line the harness's `Cargo.toml` needs for a project's
/// extension crate.
///
/// `package` is spelled out only when it actually differs from the identifier;
/// an underscored package name needs no redirect, and emitting one would be
/// noise in a generated file people read when a build goes wrong.
pub fn dep_line(ident: &str, crate_dir: &str, package: Option<&str>) -> String {
    match package {
        Some(pkg) if pkg != ident => {
            format!("{ident} = {{ path = \"{crate_dir}\", package = \"{pkg}\" }}\n")
        }
        _ => format!("{ident} = {{ path = \"{crate_dir}\" }}\n"),
    }
}

// ── Collect: the one effectful leaf ─────────────────────────────────────────

/// The package name declared by the crate rooted at `crate_dir`.
///
/// `None` when the manifest is missing, unreadable, or declares no
/// `[package] name` — every one of which leaves the caller emitting exactly
/// what it emitted before this function existed.
pub fn read_package_name(crate_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(crate_dir.join("Cargo.toml")).ok()?;
    package_name(&text).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_package_name() {
        let toml = "[package]\nname = \"cljrs-base64\"\nversion = \"0.1.0\"\n";
        assert_eq!(package_name(toml), Some("cljrs-base64"));
    }

    #[test]
    fn ignores_a_name_key_in_another_section() {
        // `[lib]` legitimately carries its own `name`, and it is the crate
        // identifier — precisely the string we must NOT report as the package.
        let toml = "\
[package]
version = \"0.1.0\"

[lib]
name = \"cljrs_base64\"
";
        assert_eq!(package_name(toml), None);
    }

    #[test]
    fn prefers_the_package_section_over_a_later_one() {
        let toml = "\
[package]
name = \"cljrs-blake3\"

[lib]
name = \"cljrs_blake3\"
";
        assert_eq!(package_name(toml), Some("cljrs-blake3"));
    }

    #[test]
    fn skips_comments_and_tolerates_odd_spacing() {
        let toml = "\
# the manifest
[package]
   name   =   'single-quoted'   # trailing note
";
        assert_eq!(package_name(toml), Some("single-quoted"));
    }

    #[test]
    fn a_double_bracket_section_is_not_package() {
        let toml = "[[bin]]\nname = \"a-binary\"\n";
        assert_eq!(package_name(toml), None);
    }

    #[test]
    fn no_package_section_reads_as_unknown() {
        assert_eq!(package_name("[dependencies]\nserde = \"1\"\n"), None);
        assert_eq!(package_name(""), None);
    }

    #[test]
    fn a_hyphenated_package_gets_an_explicit_redirect() {
        assert_eq!(
            dep_line("cljrs_base64", "/x/cljrs-base64", Some("cljrs-base64")),
            "cljrs_base64 = { path = \"/x/cljrs-base64\", package = \"cljrs-base64\" }\n"
        );
    }

    #[test]
    fn a_matching_name_needs_no_redirect() {
        assert_eq!(
            dep_line("my_plugin", "/x/my_plugin", Some("my_plugin")),
            "my_plugin = { path = \"/x/my_plugin\" }\n"
        );
    }

    #[test]
    fn an_unknown_package_keeps_the_original_shape() {
        assert_eq!(
            dep_line("my_plugin", "/x/dep", None),
            "my_plugin = { path = \"/x/dep\" }\n"
        );
    }
}
