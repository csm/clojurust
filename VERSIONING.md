# Versioned Function Resolution

This document tracks the design and implementation progress of two related
features: **versioned symbol resolution** and the **`cljrs.edn` dependency
configuration file**.

## Overview

Symbols in ClojuRust source can carry a git-commit suffix:

```clojure
my-fn@abc1234          ; unqualified versioned symbol
my.ns/my-fn@abc1234    ; namespace-qualified versioned symbol
```

Whole namespaces can also be required at a specific commit:

```clojure
(require '[my.lib@abc1234 :as lib-v1])
(require '[my.lib         :as lib   ])   ; HEAD

(lib/some-fn x)      ; current version
(lib-v1/some-fn x)   ; pinned version
```

This lets callers pin to a historical implementation without the defining
codebase needing to maintain the old code alongside the new.

---

## `cljrs.edn` — project configuration

Discovered by walking up the directory tree from the working directory.  Format
is valid ClojuRust EDN:

```clojure
{:paths ["src" "resources"]

 :deps
 {my.lib {:git/url "https://github.com/user/my-lib"
           :git/sha "abc1234ef"}
  vendor.utils {:local/root "../vendor/utils"}}

 :aliases
 {:dev  {:extra-paths ["dev"]}
  :test {:extra-paths ["test"]
         :extra-deps  {test-tools {:git/url "..." :git/sha "..."}}}}}
```

Git dependencies are cached locally in `~/.cljrs/cache/git/`.  No implicit
network access: the user must run `cljrs deps fetch` before git deps are
usable.

---

## Propagation semantics

When a function body is evaluated in a versioned context (commit `C`):

| Symbol form                        | Resolved at          |
|------------------------------------|----------------------|
| Unqualified / same-namespace, no `@` | Commit `C` (inherited) |
| Qualified self-reference (`my.ns/x` inside `my.ns@C`) | Commit `C` |
| Explicitly versioned `foo@D`       | Commit `D`           |
| External / cross-namespace, no `@` | HEAD (current)       |

This means a versioned call is a logical "snapshot": internal helpers are also
drawn from the same commit, but the standard library and cross-namespace
dependencies use their current values unless explicitly pinned.

### Whole-namespace snapshots

Resolving `ns/name@commit` — by any execution tier — loads the **whole
namespace** at that commit into an immutable namespace named `"ns@commit"`,
then performs a plain lookup in it.  Consequences:

- Historical definitions never intern into the live namespace (HEAD bindings
  are never clobbered by a pinned lookup).
- Top-level side effects of the pinned file run once, when the versioned
  namespace is first loaded (per session, cached thereafter).
- Same-namespace commit inheritance is structural: functions defined while
  loading `ns@commit` carry `defining_ns = "ns@commit"`, so their internal
  references resolve at the pinned commit on every tier without extra
  bookkeeping.

---

## Execution tiers

Versioned symbols work identically on all four execution paths; the shared
resolver lives in `cljrs_env::versioned`:

| Tier | Mechanism |
|------|-----------|
| Tree-walking interpreter | `eval_symbol` → `resolve_versioned_symbol` (thin shim) |
| IR interpreter (Tier 2) | `LoadGlobal` carries the `@sha` in its name string; `load_global_value` detects it |
| JIT-native (Tier 1) | codegen emits a **fill-once inline cache** per call site (`rt_load_global_versioned_ic`): versioned bindings are immutable, so the slot never needs invalidation; the cached value is permanently GC-rooted |
| AOT binaries | same codegen; pinned sources are **embedded at compile time** (below) |

### AOT: compile-time snapshot

`cljrs compile` resolves every versioned require and bare versioned symbol
during compilation (the `pin_versioned_references` pass), fetches each pinned
source from git, and embeds it in the binary under the versioned namespace
name.  The produced binary is **self-contained**:

- no git repository, source files, or `~/.cljrs` cache needed at runtime;
- the harness sets `versioned_offline`, so a versioned namespace that was not
  embedded fails with a clear error instead of attempting a fetch;
- a bad pin (missing commit, failed signature check) fails the *compile*;
- `--verify-commit-signatures` verifies signatures natively (against the
  `:trusted-signers` keys) at compile time — the binary trusts its embedded
  sources.

---

## Native (Rust-backed) functions

Native functions live in the running binary; there is no Clojure source to
re-evaluate at a commit.  Two modes:

### Default: verified HEAD binding

A pinned lookup that falls back to a native function resolves to the current
binary's implementation, **verified against the package's recorded
provenance** (the commit it was built from, registered via
`cljrs_interop::register_provenance!("ns", env!("CLJRS_PKG_COMMIT"))` or
`Registry::set_provenance`):

- provenance matches the pin (either may be abbreviated) → silent;
- mismatch or no recorded provenance → warning, once per pin;
- with `--enforce-native-versions` (or `:enforce-native-versions true` in
  `cljrs.edn`) → hard error.

### Opt-in: pinned native code (`:rust/load :dylib`) — experimental

```clojure
{:deps
 {my.native.lib {:git/url   "https://github.com/user/my-native-lib"
                 :git/sha   "abc1234ef"
                 :rust/init "my_native_lib::cljrs_init"
                 :rust/load :dylib}}}
```

When a pinned symbol resolves into such a dep's namespace, `cljrs-dylib`
fetches the repository at the pinned commit, generates a wrapper cdylib crate
(pinning the exact same `cljrs-interop` as the host), builds it with cargo
(cached under `~/.cljrs/cache/dylibs/<crate>@<commit>/`), `dlopen`s it, and
registers its exports into the immutable `"<ns>@<commit>"` namespace via a
versioned `Registry` view.

**ABI discipline:** the wrapper exports `cljrs_dylib_abi()` returning a
fingerprint (cljrs version + `rustc -V`, baked at the wrapper's build time);
the host refuses to call the Rust-ABI `cljrs_dylib_init(*mut Registry)`
unless the fingerprint equals its own exactly.  Feature-flag skew between
host and wrapper is *not* detected; a Rust toolchain is required at runtime.
A full C-ABI vtable is the safer long-term design and is deliberately
deferred, as is statically linking pinned native crates into AOT harnesses
(open problem: two statically linked versions of one crate submit `#[export]`
inventory entries under the same unversioned names).

---

## Architecture

### New crates

| Crate        | Responsibility |
|--------------|----------------|
| `cljrs-deps` | Parse `cljrs.edn`, `DepsConfig` / `Dependency` types, config discovery |
| `cljrs-vcs`  | Pure-Rust (gitoxide) git helpers: `find_repo_root`, `get_file_at_commit`, `fetch_remote`, commit-hash validation, cache layout, native PGP/SSH commit-signature verification |

### Modified crates

| Crate            | Changes |
|------------------|---------|
| `cljrs-value`    | `Symbol` gains `version: Option<Arc<str>>`; `Symbol::parse` splits on `@`; `Namespace` gains `source_file`, `git_repo_root`, `is_versioned` |
| `cljrs-reader`   | `lex_symbol` peeks for `@<hex>` suffix and embeds it in the symbol string |
| `cljrs-env`      | `RequireSpec` gains `version`; `GlobalEnv` gains `version_cache` and `deps_config`; `Env` gains `versioned_eval_commit` and `lookup_local_frames`; `loader.rs` gains `load_versioned_ns` |
| `cljrs-interp`   | `eval_symbol` dispatches versioned symbols; `resolve_versioned_symbol` function added |
| `cljrs-builtins` / `cljrs-interp special.rs` | `parse_require_spec_val` / `parse_require_spec_form` extract version from namespace symbol |
| `cljrs` (CLI)    | Load `cljrs.edn` at startup; `cljrs deps fetch/status` subcommands |

---

## Implementation phases and status

| # | Phase | Crate(s) touched | Status |
|---|-------|------------------|--------|
| 1 | `cljrs-deps` crate — config types and `cljrs.edn` parser | `cljrs-deps` (new) | ✅ Done |
| 2 | `cljrs-vcs` crate — pure-Rust (gitoxide) git helpers | `cljrs-vcs` (new) | ✅ Done |
| 3 | `Symbol.version`, `Namespace` git fields | `cljrs-value` | ✅ Done |
| 4 | Lexer `@hash` recognition | `cljrs-reader` | ✅ Done |
| 5 | `RequireSpec.version`, `GlobalEnv` version cache, `Env.versioned_eval_commit` | `cljrs-env` | ✅ Done |
| 6 | `eval_symbol` versioned dispatch, `resolve_versioned_symbol`, `load_versioned_ns` | `cljrs-interp` | ✅ Done |
| 7 | `parse_require_spec_*` version extraction | `cljrs-interp` special forms | ✅ Done |
| 8 | CLI: startup config load, `deps fetch/status` | `cljrs` | ✅ Done |

---

## Key data structures (as implemented)

### `Symbol` (cljrs-value)
```rust
pub struct Symbol {
    pub namespace: Option<Arc<str>>,
    pub name: Arc<str>,
    pub version: Option<Arc<str>>,   // e.g. "abc1234"
}
```
`Symbol::parse("ns/my-fn@abc1234")` → `{ namespace: Some("ns"), name: "my-fn", version: Some("abc1234") }`

### `Namespace` (cljrs-value)
```rust
pub struct Namespace {
    pub name: Arc<str>,
    pub interns: Mutex<HashMap<Arc<str>, GcPtr<Var>>>,
    pub refers:  Mutex<HashMap<Arc<str>, GcPtr<Var>>>,
    pub aliases: Mutex<HashMap<Arc<str>, Arc<str>>>,
    pub source_file:   Mutex<Option<Arc<str>>>,
    pub git_repo_root: Mutex<Option<Arc<str>>>,
    pub is_versioned:  bool,
}
```

### `RequireSpec` (cljrs-env)
```rust
pub struct RequireSpec {
    pub ns:      Arc<str>,
    pub version: Option<Arc<str>>,   // present for `my.ns@abc1234`
    pub alias:   Option<Arc<str>>,
    pub refer:   RequireRefer,
}
```

### `Env` (cljrs-env)
```rust
pub struct Env {
    pub frames:                Vec<Frame>,
    pub current_ns:            Arc<str>,
    pub globals:               Arc<GlobalEnv>,
    pub versioned_eval_commit: Option<Arc<str>>,
}
```

### `GlobalEnv` additions (cljrs-env)
```rust
pub version_cache: Mutex<HashMap<Arc<str>, Value>>,
// key: "<ns>/<name>@<commit>"
pub deps_config: RwLock<Option<Arc<DepsConfig>>>,
```

### `DepsConfig` (cljrs-deps)
```rust
pub struct DepsConfig {
    pub paths:   Vec<PathBuf>,
    pub deps:    Vec<(String, Dependency)>,
    pub aliases: Vec<(String, Alias)>,
}
pub enum Dependency {
    Git   { url: String, sha: String },
    Local { root: PathBuf },
}
```

---

## CLI commands (phase 8)

```
cljrs deps fetch              # clone/update all git deps
cljrs deps fetch <dep-name>   # fetch one dep by name
cljrs deps status             # show cached vs missing deps
```

Network access is always opt-in.  If a versioned symbol or namespace requires a
git dep that is not cached, the runtime returns a clear error:

```
error: dependency 'my.lib' is not cached locally.
       run `cljrs deps fetch` to download it.
```
