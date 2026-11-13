# cljrs trust

Allow a project to load native code.

```
cljrs trust [DIR]
cljrs trust --list
cljrs untrust [DIR]
```

A `cljrs.edn` can ask for native code to be loaded into the interpreter: the
project's own [`:rust` crate](../rust-interop/interpreter.md), or a dependency
declared with [`:rust/load :dylib`](deps.md#native-dependencies). Loading a
shared library runs whatever it contains, with your privileges, and `cljrs`
finds `cljrs.edn` by walking up from the current directory — so a config file
is not allowed to authorise its own native code. You do, once per project.

Until a project is trusted, `cljrs` uses everything else in its `cljrs.edn`
(source paths, dependencies, signature settings) but builds and loads no
native code, and says so:

```
cljrs: native code not loaded: the project at /work/my-project is not trusted
to run native code; run `cljrs trust` there to allow it, or pass --native for
this run only
```

## Trusting a project

`cljrs trust` records the directory holding the nearest `cljrs.edn` (or `DIR`,
if given) in `~/.cljrs/trusted-projects`. From then on `run`, `repl`, `eval`,
`test` and `nrepl` load that project's native code automatically.

- Trust is keyed on the directory's canonical path. Editing `cljrs.edn` does
  not revoke it; moving or re-cloning the project does.
- Only that exact directory is covered — not its subdirectories' own
  `cljrs.edn` files, and not its parents'.
- A successful [`cljrs build-native`](build-native.md) trusts the project for
  you, and prints a line saying so.

`cljrs trust --list` prints the trusted roots, one per line. `cljrs untrust`
removes one.

## One-off and non-interactive use

| Setting | Effect |
|---|---|
| `--native` / `CLJRS_NATIVE=1` | Load native code for this run without trusting the project. |
| `--no-native` / `CLJRS_NO_NATIVE=1` | Never load native code, even for a trusted project. Wins over `--native`. |
| `--no-config` / `CLJRS_NO_CONFIG=1` | Do not look for `cljrs.edn` at all. |
| `CLJRS_TRUSTED_PROJECTS=<dir>[:<dir>…]` | Treat these project roots as trusted. For CI, where there is no store to write to. |

## Which `cljrs.edn` is used

Two further limits apply to finding the config, independent of native code:

- The upward search stops at your home directory. A `cljrs.edn` above it is
  never used for a project inside it.
- On Unix, a `cljrs.edn` that is owned by another user (or sits in a directory
  owned by another user) is ignored with a warning, unless you have trusted
  that project. Files owned by root are accepted. This is most often met when
  running as root over a bind-mounted checkout; `cljrs trust` or
  `CLJRS_TRUSTED_PROJECTS` lifts it.

## What is not gated

`cljrs build-native` and `cljrs compile` are explicit build commands: they run
`cargo` in the project because that is what you asked for. AOT-compiled
binaries link their Rust code statically and do not consult the trust store.
