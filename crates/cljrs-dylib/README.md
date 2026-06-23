# cljrs-dylib

## Purpose

Pinned native packages: build a dependency's Rust crate at a pinned git
commit as a cdylib and load it, so versioned symbols (`my.lib/f@<sha>`) can
resolve to **truly pinned** native code instead of the default verified HEAD
binding (`:rust/load :dylib` in `cljrs.edn`).  The same machinery also makes a
`:rust/load :dylib` dependency loadable by a **plain `require`** of its
namespace, registering the package's exports into the live (unversioned)
namespace.

## Status

Versioned-namespaces plan, Phase 5 (see `docs/versioned-namespaces-plan.md`).
Implemented and tested end-to-end, but **experimental**: the init call
crosses a Rust-ABI boundary guarded only by the fingerprint handshake
(feature-flag skew between host and wrapper is not detected), and a Rust
toolchain is required at runtime.  Statically linking pinned native crates
into AOT harnesses is deferred (open problem: `#[export]` inventory
collisions between two versions of one crate).

## File layout

```
src/lib.rs  — install (both loader hooks), wrapper crate generation, cargo
              build + cache, dlopen + ABI handshake, versioned/unversioned
              Registry init
build.rs    — captures `rustc -V` for the host side of the ABI fingerprint
tests/
  pinned_dylib_e2e.rs — gated end-to-end test (CLJRS_DYLIB_E2E=1): two-commit
              native crate fixture; pinned (versioned-symbol) resolution loads
              the v1 dylib while HEAD stays untouched, and a plain `require`
              loads the v1 dylib into the unversioned namespace
```

## Public API

```rust
/// Install both native loader hooks on the environment (idempotent): the
/// pinned-native loader (versioned-symbol resolution) and the native-require
/// loader (plain `require` of a `:rust/load :dylib` dep).  Called by the
/// cljrs CLI during setup_globals.
pub fn install(globals: &Arc<GlobalEnv>);

/// The host's ABI fingerprint: "cljrs <version>; <rustc -V>; <debug|release>".
/// A wrapper dylib is loaded only when its baked fingerprint equals this.
pub fn abi_fingerprint() -> String;

pub const ABI_SYMBOL: &[u8];   // b"cljrs_dylib_abi\0"
pub const INIT_SYMBOL: &[u8];  // b"cljrs_dylib_init\0"
```

## How it works

1. The versioned resolver (`cljrs_env::versioned`) calls the installed
   `PinnedNativeLoader` when a pinned lookup is about to fall back to a
   native function.
2. The loader finds a `:rust/load :dylib` git dep covering the namespace
   (exact or dotted-prefix match) with a `:rust/init` function.
3. `cljrs_vcs::fetch_remote` + a gitoxide worktree checkout of the pinned
   commit's tree (`~/.cljrs/cache/dylibs/checkouts/<crate>@<commit>`, no
   `.git`; a `.cljrs-checkout-complete` sentinel marks a finished checkout).
4. A wrapper cdylib crate is generated
   (`~/.cljrs/cache/dylibs/<crate>@<commit>/fp-<hash>/`), pinning the same
   `cljrs-interop` as the host (local checkout path when found —
   `CLJRS_WORKSPACE_ROOT` override honored — else the published `=version`),
   and built with cargo **in the host's profile** (debug/release —
   `cljrs-gc` object headers differ between profiles).
5. dlopen → `cljrs_dylib_abi()` fingerprint must equal
   `abi_fingerprint()` exactly, else refuse → `cljrs_dylib_init(*mut
   Registry)` registers the package's exports through
   `Registry::versioned(commit)`, landing every definition in the immutable
   `"<ns>@<commit>"` namespace.
6. The namespace is marked loaded; subsequent pinned lookups are plain
   namespace hits.

### Plain `require` of a native dep

When `(require '[my.native.lib :as l])` finds no Clojure source for the
namespace, `cljrs_env`'s unversioned loader consults the installed
`NativeRequireLoader`.  It runs the same fetch/checkout/wrapper-build pipeline
(steps 2–4 above), keyed on the dep's pinned `:git/sha`, then runs
`cljrs_dylib_init` through `Registry::for_require(...)` — an **unversioned**
view — so the exports land in the live `my.native.lib` namespace.  The loader
returns and the unversioned loader marks the namespace loaded, so `l/encode`
resolves like any other namespace.
