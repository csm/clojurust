//! Walk the `cljrs-reader` Form tree produced from a `cljrs.edn` source
//! and construct a `DepsConfig`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cljrs_reader::{Form, FormKind, Parser};

use crate::{Alias, Dependency, DepsConfig, GitDep};

// ── Public entry point ────────────────────────────────────────────────────────

/// Parse `src` (the text of a `cljrs.edn` file located at `config_path`) into
/// a `DepsConfig`.  `config_path` is used only for resolving `:local/root`
/// paths relative to the config directory.
pub fn parse_config(src: &str, config_path: &Path) -> Result<DepsConfig, String> {
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new("."));

    let mut parser = Parser::new(src.to_owned(), config_path.display().to_string());
    let forms = parser.parse_all().map_err(|e| e.to_string())?;

    // The file must contain exactly one top-level form: a map.
    if forms.len() != 1 {
        return Err(format!(
            "cljrs.edn must contain exactly one top-level map; found {} forms",
            forms.len()
        ));
    }

    extract_config(&forms[0], config_dir)
}

// ── Top-level map ─────────────────────────────────────────────────────────────

fn extract_config(form: &Form, config_dir: &Path) -> Result<DepsConfig, String> {
    let pairs = require_map(form, "top-level cljrs.edn")?;
    let mut config = DepsConfig::default();

    let mut i = 0;
    while i + 1 < pairs.len() {
        let key = &pairs[i];
        let val = &pairs[i + 1];
        i += 2;

        match keyword_name(key) {
            Some("paths") => {
                config.paths = extract_path_vec(val, ":paths", config_dir)?;
            }
            Some("deps") => {
                config.deps = extract_deps_map(val, config_dir)?;
            }
            Some("aliases") => {
                config.aliases = extract_aliases_map(val, config_dir)?;
            }
            _ => {} // ignore unknown keys
        }
    }

    Ok(config)
}

// ── :paths ────────────────────────────────────────────────────────────────────

fn extract_path_vec(form: &Form, ctx: &str, base: &Path) -> Result<Vec<PathBuf>, String> {
    let items = require_vec(form, ctx)?;
    items
        .iter()
        .map(|f| {
            let s = require_str(f, ctx)?;
            Ok(base.join(s))
        })
        .collect()
}

// ── :deps ─────────────────────────────────────────────────────────────────────

fn extract_deps_map(form: &Form, config_dir: &Path) -> Result<Vec<(Arc<str>, Dependency)>, String> {
    let pairs = require_map(form, ":deps")?;
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < pairs.len() {
        let name = sym_or_kw_name(&pairs[i])
            .ok_or_else(|| format!(":deps key must be a symbol, got {:?}", pairs[i].kind))?;
        let dep = extract_dependency(&pairs[i + 1], config_dir, &name)?;
        out.push((Arc::from(name), dep));
        i += 2;
    }
    Ok(out)
}

fn extract_dependency(form: &Form, config_dir: &Path, name: &str) -> Result<Dependency, String> {
    let pairs = require_map(form, &format!("dep {name}"))?;
    let mut git_url: Option<Arc<str>> = None;
    let mut git_sha: Option<Arc<str>> = None;
    let mut local_root: Option<PathBuf> = None;

    let mut i = 0;
    while i + 1 < pairs.len() {
        match keyword_name(&pairs[i]) {
            Some("git/url") => {
                git_url = Some(Arc::from(require_str(&pairs[i + 1], "git/url")?));
            }
            Some("git/sha") => {
                git_sha = Some(Arc::from(require_str(&pairs[i + 1], "git/sha")?));
            }
            Some("local/root") => {
                let rel = require_str(&pairs[i + 1], "local/root")?;
                local_root = Some(config_dir.join(rel));
            }
            _ => {}
        }
        i += 2;
    }

    match (git_url, git_sha, local_root) {
        (Some(url), Some(sha), _) => Ok(Dependency::Git(GitDep { url, sha })),
        (_, _, Some(root)) => Ok(Dependency::Local { root }),
        _ => Err(format!(
            "dep {name}: must specify either :git/url + :git/sha or :local/root"
        )),
    }
}

// ── :aliases ──────────────────────────────────────────────────────────────────

fn extract_aliases_map(form: &Form, config_dir: &Path) -> Result<Vec<(Arc<str>, Alias)>, String> {
    let pairs = require_map(form, ":aliases")?;
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < pairs.len() {
        let name = keyword_name(&pairs[i])
            .ok_or_else(|| ":aliases key must be a keyword".to_string())?
            .to_owned();
        let alias = extract_alias(&pairs[i + 1], config_dir, &name)?;
        out.push((Arc::from(name), alias));
        i += 2;
    }
    Ok(out)
}

fn extract_alias(form: &Form, config_dir: &Path, name: &str) -> Result<Alias, String> {
    let pairs = require_map(form, &format!("alias :{name}"))?;
    let mut alias = Alias::default();
    let mut i = 0;
    while i + 1 < pairs.len() {
        match keyword_name(&pairs[i]) {
            Some("extra-paths") => {
                alias.extra_paths = extract_path_vec(&pairs[i + 1], ":extra-paths", config_dir)?;
            }
            Some("extra-deps") => {
                alias.extra_deps = extract_deps_map(&pairs[i + 1], config_dir)?;
            }
            _ => {}
        }
        i += 2;
    }
    Ok(alias)
}

// ── Form helpers ──────────────────────────────────────────────────────────────

fn require_map<'a>(form: &'a Form, ctx: &str) -> Result<&'a Vec<Form>, String> {
    match &form.kind {
        FormKind::Map(pairs) => Ok(pairs),
        _ => Err(format!("{ctx}: expected a map, got {:?}", form.kind)),
    }
}

fn require_vec<'a>(form: &'a Form, ctx: &str) -> Result<&'a Vec<Form>, String> {
    match &form.kind {
        FormKind::Vector(items) => Ok(items),
        _ => Err(format!("{ctx}: expected a vector, got {:?}", form.kind)),
    }
}

fn require_str<'a>(form: &'a Form, ctx: &str) -> Result<&'a str, String> {
    match &form.kind {
        FormKind::Str(s) => Ok(s.as_str()),
        _ => Err(format!("{ctx}: expected a string, got {:?}", form.kind)),
    }
}

/// Extract the name from a `:keyword` form (without the leading colon).
fn keyword_name(form: &Form) -> Option<&str> {
    match &form.kind {
        FormKind::Keyword(k) => Some(k.as_str()),
        _ => None,
    }
}

/// Extract the string content from either a `Symbol` or `Keyword` form.
fn sym_or_kw_name(form: &Form) -> Option<String> {
    match &form.kind {
        FormKind::Symbol(s) => Some(s.clone()),
        FormKind::Keyword(k) => Some(k.clone()),
        _ => None,
    }
}
