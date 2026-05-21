# cljrs-deps

## Purpose

Parses `cljrs.edn` project configuration files and provides the `DepsConfig`
type used throughout the runtime to locate git-hosted and local dependencies,
and to discover the optional embedded Rust crate configuration.

## Status

Phase 1 (implemented).  Config parsing is complete.  Integration with the CLI
(`cljrs deps fetch/status`) is Phase 8 (todo).

## File layout

| File | Description |
|------|-------------|
| `src/lib.rs` | Public types (`DepsConfig`, `RustConfig`, `Dependency`, `Alias`, `GitDep`), `find_config_file`, `load_config` |
| `src/parse.rs` | Walk the `cljrs-reader` Form tree from a `cljrs.edn` source into `DepsConfig` |

## Public API

```rust
/// Find the nearest `cljrs.edn` by walking up from `start`.
pub fn find_config_file(start: &Path) -> Option<PathBuf>

/// Load and parse the nearest `cljrs.edn`, returning None if absent.
pub fn load_config(start: &Path) -> DepsResult<Option<DepsConfig>>

/// Parse `cljrs.edn` source text directly (used in tests / CLI).
pub fn parse_config(src: &str, config_path: &Path) -> Result<DepsConfig, String>

pub struct DepsConfig {
    pub paths:                    Vec<PathBuf>,
    pub deps:                     Vec<(Arc<str>, Dependency)>,
    pub aliases:                  Vec<(Arc<str>, Alias)>,
    pub verify_commit_signatures: bool,
    pub rust:                     Option<RustConfig>,
}

/// Rust-crate configuration for mixed Rust/Clojure projects.
/// Parsed from the `:rust` key in `cljrs.edn`.
pub struct RustConfig {
    /// Directory containing the user's Cargo.toml (resolved from cljrs.edn dir).
    pub crate_dir: PathBuf,
    /// Fully-qualified init fn, e.g. "my_project::cljrs_init". Optional.
    pub init_fn:   Option<Arc<str>>,
}

pub enum Dependency {
    Git(GitDep),
    Local { root: PathBuf },
}

pub struct GitDep { pub url: Arc<str>, pub sha: Arc<str> }

pub struct Alias {
    pub extra_paths: Vec<PathBuf>,
    pub extra_deps:  Vec<(Arc<str>, Dependency)>,
}
```

## cljrs.edn format

```edn
{:paths ["src"]

 :deps
 {my.lib {:git/url "https://github.com/user/my-lib" :git/sha "abc1234ef"}}

 ;; Optional: embed a Rust crate in this project.
 ;; :crate is the path (relative to cljrs.edn) to the directory holding Cargo.toml.
 ;; :init  is the fully-qualified Rust path to a fn(registry: &mut Registry) called
 ;;        at startup before any Clojure source is loaded.
 :rust {:crate "."
        :init  "my_project::cljrs_init"}

 :aliases
 {:dev  {:extra-paths ["dev"]}
  :test {:extra-paths ["test"]
         :extra-deps  {test-tools {:git/url "..." :git/sha "..."}}}}}
```
