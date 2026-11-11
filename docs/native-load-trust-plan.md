# Native-Load Trust — Options for Issue #476

## Why this document exists

Issue #476 reports that `cljrs eval '(+ 1 2)'` run anywhere beneath a directory holding a
`cljrs.edn` with a `:rust` section will `dlopen` a shared library named by that file — before the
expression is even parsed, with no flag and no trust decision. A library constructor runs on
`dlopen`, so the later "could not find symbol" check is too late to matter.

This document confirms the report against the current tree, widens it to the neighbouring paths
that share the same flaw, lays out the options for fixing it, and recommends one. **Nothing here is
implemented yet**; the open decisions at the end need a maintainer's call first.

---

## What actually happens today

Every session-creating subcommand (`eval`, `run`, `repl`, `test`, `nrepl`) goes through
`session::setup_globals` → `apply_deps_config` (`crates/cljrs/src/session.rs`):

1. `cljrs_project::config::find_config_file` walks from the cwd to the filesystem root and takes
   the first `cljrs.edn` it finds. There is no stop at `$HOME`, a VCS root, or a directory owned
   by someone else.
2. If the config has `:rust {… :init …}`, `native::load_project_lib` runs unconditionally.
3. `load_project_lib` calls `native_lib_path`, which **spawns `cargo metadata` with the cwd set to
   the config's `:crate` directory**, then `dlopen`s `<target>/debug/lib<crate>.so`, then looks up
   the init symbol.

So there are two execution points, not one:

| # | Vector | Needs a planted `.so`? | Status |
|---|---|---|---|
| 1 | `dlopen` of the project library (constructor runs) | yes | Reported in #476 |
| 2 | `cargo metadata` in the attacker's directory. Where `cargo` is the rustup proxy, a `rust-toolchain.toml` with `[toolchain] path = …` makes the proxy exec an arbitrary binary named `cargo` | **no** | Confirmed locally while writing this plan; not in the issue |

Vector 2 means "check the library before loading it" style fixes are insufficient on their own:
the subprocess has to be behind the same gate as the `dlopen`.

### Neighbouring paths with the same root cause

These are not triggered by a bare `eval '(+ 1 2)'`, but they are driven by the same untrusted,
ancestor-discovered config and should sit behind whatever gate is chosen:

- **`:rust/load :dylib` dependencies** (`native/pinned.rs`). A `require` of a namespace covered by
  such a dep runs `cargo build` (hence the dep's `build.rs`) and `dlopen`s the result. The doc
  comments call this "opt-in", but the opt-in lives in the very file whose trust is in question.
- **`:paths` and dependency source roots.** An ancestor config can append source directories, so a
  `require` in the user's own script can resolve to attacker-supplied Clojure. Not native code and
  arguably the ordinary meaning of "project config", but it is the reason restricting *discovery*
  matters independently of native loading.
- **`:verify-commit-signatures` / `:trusted-signers`.** The config under suspicion also supplies
  the key set that would vouch for it, which is why `--verify-commit-signatures` does not help
  here (as the issue notes).

`cljrs build-native` and `cljrs compile` also read `:rust`, but they are explicit build commands;
running the project's build is what the user asked for, as with `cargo build`. `cljrs lsp` does not
go through `setup_globals`.

---

## Design goals

1. No native code — library load *or* toolchain subprocess — runs from discovered config unless the
   user has said so through a channel the config cannot write to.
2. A way to switch config discovery and native loading off entirely.
3. Existing mixed Rust/Clojure projects keep a workflow that is one step, once, not a flag on
   every command.
4. Works non-interactively (CI, editors launching `cljrs nrepl`).

---

## Options

### Option A — Per-invocation flag (`--native` / `CLJRS_NATIVE=1`)

Native loading is off unless the flag or env var is present.

- **For:** smallest change; trivially auditable; no state on disk.
- **Against:** every `run`/`repl`/`test`/`nrepl` in a mixed project needs the flag, so people will
  export `CLJRS_NATIVE=1` in their shell profile — which restores today's behaviour globally and
  silently. Fails goal 3.

### Option B — Per-project trust store (`cljrs trust`)

A user-level file (`$XDG_CONFIG_HOME/cljrs/trusted-projects.edn`, outside any project) lists
project roots allowed to load native code. `cljrs trust [DIR]` adds one, `cljrs untrust` removes
one, `cljrs trust --list` shows them. An untrusted config with `:rust` (or a `:rust/load :dylib`
dep being reached) is skipped with a one-line notice naming the command to run. This is the model
of `direnv allow`, `mise trust`, and editor workspace trust.

- **For:** meets all four goals; one explicit step per checkout; CI can set
  `CLJRS_TRUSTED_PROJECTS=<path>[:<path>…]` or pass `--trust-project`.
- **Against:** new subcommand, new on-disk state, and a breaking change for existing projects on
  upgrade (their native functions stop registering until `cljrs trust` is run — the notice must be
  loud and specific).
- **Sub-decision — what the entry keys on:**
  - *Canonical path only* (recommended). Simple, and honest about what it protects: anyone who can
    later write to a trusted directory can already edit the `.so` in `target/` or the Clojure
    sources the user runs, so pinning the config's bytes adds little.
  - *Path + digest of `cljrs.edn`* (direnv-style). Re-prompts on every config edit; catches a
    config swapped under a trusted path but not a swapped library.
- **Sub-decision — does `cljrs build-native` grant trust?** It is an explicit native action in
  that project, so auto-trusting on a successful build makes the common path zero extra steps.
  The cost is that trust becomes a side effect rather than a stated decision. Recommended: yes,
  with a printed line saying so.

### Option C — Restrict discovery

Independently of trust, narrow which `cljrs.edn` is found:

- **C1.** Stop the upward walk at `$HOME` and at filesystem/mount boundaries.
- **C2.** Refuse a config (or its directory) not owned by the current user, in the manner of git's
  `safe.directory`. This closes the shared-`/tmp` and multi-user-box cases outright.
- **C3.** Stop at the nearest VCS root.
- **C4.** Current directory only — no ancestor walk.

C2 is cheap and high-value. C1 is harmless. C3 and C4 break legitimate use (running from `src/`
or a subcrate) and, as the issue says, none of these addresses an untrusted config in the cwd
itself (a cloned repository). Useful hardening, **not a fix**.

### Option D — Scope by subcommand

`cljrs eval` stops loading project config, or at least its native part; only `run`, `repl`, `test`,
`nrepl` do.

- **For:** kills the most surprising case (a one-liner triggering a project build artifact).
- **Against:** `cljrs run script.cljrs` inside a hostile checkout is just as exposed, and `eval`
  against project namespaces is a legitimate use. Partial at best; changes `eval` semantics.

### Option E — Off switches

`--no-native` / `CLJRS_NO_NATIVE=1` (never load native code, trusted or not) and `--no-config` /
`CLJRS_NO_CONFIG=1` (do not discover `cljrs.edn` at all). The issue asks for these explicitly, and
they are wanted under every other option — for sandboxes, for scripts, and for debugging "is my
native lib the problem".

### Option F — Loader hardening

Things that reduce blast radius without constituting a trust decision:

- **F1.** Derive the library path without spawning `cargo` in the project directory unless the
  project is trusted (closes vector 2). Either have `build-native` record the artifact path it
  produced, or only call `cargo metadata` after the gate.
- **F2.** Have `build-native` record a digest of the artifact in user-level state and load only a
  library whose digest matches. Binds "what gets loaded" to "what cljrs itself built". Stronger
  than path trust but breaks anyone running `cargo build` directly, which the docs currently
  permit.
- **F3.** Check for the init symbol by parsing the object file before `dlopen`. Turns a typo into a
  clean error, but a malicious library simply exports the symbol. Cosmetic for this issue.

---

## Comparison

| | Blocks #476 repro | Blocks vector 2 | Blocks hostile cwd config | Existing projects | Non-interactive |
|---|---|---|---|---|---|
| A flag | yes | yes | yes | flag on every command | env var |
| B trust store | yes | yes (with F1) | yes | one `cljrs trust` per checkout | env var / flag |
| C2 ownership | only cross-user | only cross-user | no | unaffected | unaffected |
| C4 cwd only | yes | yes | no | breaks subdirectory use | unaffected |
| D scope `eval` | yes | yes for `eval` | no (`run` still exposed) | `eval` loses native fns | unaffected |
| E off switches | only if set | only if set | only if set | unaffected | unaffected |

---

## Recommendation

**B + E + F1, with C1/C2 as hardening**, delivered in two steps so that a fix can ship quickly
without waiting on the trust-store design.

### Step 1 — stop the bleeding (small, suitable for a patch release)

- Add a single gate function in `crates/cljrs/src/native/mod.rs` (say `native_allowed(project_root)
  -> Decision`) consulted by `load_project_lib` *before* `native_lib_path`, and by the two loader
  hooks in `native/pinned.rs` before any `cargo` invocation.
- In this step the gate is Option A's rule: allow only with `--native` / `CLJRS_NATIVE=1`, never
  with `--no-native` / `CLJRS_NO_NATIVE=1`. Otherwise skip and print one line:
  `cljrs: <path>/cljrs.edn declares native code; not loaded (pass --native to allow)`.
- Add `--no-config` / `CLJRS_NO_CONFIG=1`, checked in `setup_globals` before `apply_deps_config`
  and in the `deps`/`compile`/`build-native` call sites of `load_config`.
- Add C1 and C2 to `find_config_file` (a skipped foreign-owned config warns rather than silently
  continuing upward to a different one).
- Tests: an integration test reproducing the issue's script and asserting the constructor does not
  run under `eval` — including with an unbalanced expression — and that no `cargo` process is
  spawned; unit tests for the gate's precedence and for discovery stopping rules.
- Docs: `docs/book/src/rust-interop/{project-setup,interpreter}.md`, `cli/build-native.md`, the
  README, the `cljrs` and `cljrs-project` crate READMEs, and a changelog entry flagged as a
  behaviour change.

### Step 2 — make it livable (the trust store)

- New module `cljrs_project::trust`: load/save the store, canonicalise roots, `is_trusted(root)`.
- New `cljrs trust` / `cljrs untrust` subcommands; `build-native` records trust on success (if
  that sub-decision goes that way).
- The gate from step 1 gains a third allow condition: project root is in the store or in
  `CLJRS_TRUSTED_PROJECTS`. `--native` remains as the one-off override; the skip notice changes to
  point at `cljrs trust`.
- Interactive `cljrs repl` may offer the prompt inline; non-TTY sessions never prompt.

Step 1 alone resolves the issue as filed. Step 2 is what keeps people from exporting
`CLJRS_NATIVE=1` globally and undoing it.

---

## Open decisions

1. **Is a breaking default acceptable in a patch release?** After step 1, existing mixed projects
   need `--native` (then `cljrs trust`) or their native functions are unregistered. The
   alternative — warn now, enforce later — leaves the hole open for a release.
2. **Trust key:** path only, or path + config digest? (Recommended: path only.)
3. **Does `build-native` imply trust?** (Recommended: yes, announced.)
4. **Should `:paths` / dependency source roots from an untrusted config also be withheld?** That
   would make trust cover Clojure code too, which is the stricter and more uniform rule, but it
   means a fresh clone does nothing useful until trusted. Recommended: no for now — keep the gate
   about native code, rely on C1/C2 for the ancestor case, and revisit.
5. **Should `eval` consult project config at all?** Orthogonal once the gate exists; leaving it
   as-is avoids a second behaviour change.
6. **F2 (digest-pinned artifacts):** worth doing later, or does it cost more workflow than it buys?
