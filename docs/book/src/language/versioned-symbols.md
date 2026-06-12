# Versioned symbols

clojurust lets you pin a symbol or namespace to a specific git commit by
appending `@<commit>` to its name. This lets callers use a historical
implementation without requiring the defining library to keep the old code
alongside the new.

## Syntax

```clojure
my-fn@abc1234           ; unqualified versioned symbol
my.ns/my-fn@abc1234     ; namespace-qualified versioned symbol
```

The commit suffix must be a valid hex prefix of at least 7 characters (the full
40-character hash is recommended for reproducibility).

## Versioned require

Whole namespaces can be loaded at a specific commit:

```clojure
(require '[my.lib@abc1234 :as lib-v1])
(require '[my.lib         :as lib])      ; HEAD

(lib/some-fn x)        ; current version
(lib-v1/some-fn x)     ; pinned to abc1234
```

Both aliases coexist in the same namespace; calls through `lib-v1/` always
resolve against commit `abc1234`.

## Propagation semantics

When a function body is evaluated in a versioned context — because it was
loaded via a versioned `require` or called through a versioned symbol — the
following resolution rules apply:

| Symbol form | Resolved at |
|---|---|
| Unqualified or same-namespace, no `@` | The inherited commit (propagated from the caller) |
| Qualified self-reference (`my.ns/x` written inside `my.ns`) | The inherited commit |
| Explicitly versioned `foo@D` | Commit `D` |
| External / cross-namespace, no `@` | HEAD (current) |

This means a versioned call behaves like a logical snapshot: internal helpers
in the same namespace are drawn from the same commit automatically, but
cross-namespace dependencies and the standard library use their current values
unless explicitly pinned.

Resolving any pinned symbol loads the **whole namespace** at that commit into
an immutable namespace named `my.ns@<commit>`: top-level side effects of the
pinned file run once (when the snapshot is first loaded), historical
definitions never overwrite the live (HEAD) bindings, and the snapshot is
cached for the rest of the session.

## Execution tiers

Versioned symbols behave identically everywhere code runs:

- **Interpreter** — symbols resolve through the shared versioned resolver.
- **JIT** — hot functions keep their pins: compiled code resolves each pinned
  reference once through a per-call-site inline cache (versioned bindings are
  immutable, so the cache never needs invalidation).
- **AOT (`cljrs compile`)** — pins are resolved *at compile time*: every
  versioned require and bare versioned symbol is fetched from git during
  compilation and embedded in the binary. The produced binary is
  self-contained — no git repository, source tree, or `~/.cljrs` cache is
  needed where it runs. A pin pointing at a missing commit fails the compile,
  and a versioned namespace that was not embedded fails at runtime with a
  clear "was not embedded at compile time" error.

## Native (Rust) functions

Native functions live in the running binary, so there is no historical
Clojure source to re-evaluate. By default a pinned lookup of a native
function resolves to the current binary's implementation — a **verified HEAD
binding**: the runtime compares the pin against the package's recorded
provenance (the commit it was built from, declared with
`cljrs_interop::register_provenance!`). A match is silent; a mismatch or
missing provenance warns once per pin, or errors when
`--enforce-native-versions` (or `:enforce-native-versions true` in
`cljrs.edn`) is set.

For true pinned native code, opt in per dependency (experimental; requires a
Rust toolchain at runtime):

```clojure
{:deps
 {my.native.lib {:git/url   "https://github.com/user/my-native-lib"
                 :git/sha   "abc1234ef"
                 :rust/init "my_native_lib::cljrs_init"
                 :rust/load :dylib}}}
```

The runtime then builds the dependency's crate at the pinned commit as a
shared library (cached under `~/.cljrs/cache/dylibs/`), verifies an ABI
fingerprint (cljrs version, rustc version, build profile must match the host
exactly), and registers the pinned implementations into the immutable
`my.native.lib@<commit>` namespace.

## Dependency setup

Versioned symbols require the referenced git repository to be cached locally.
Declare the dependency in `cljrs.edn` and run `cljrs deps fetch` before using
versioned symbols:

```clojure
; cljrs.edn
{:deps
 {my.lib {:git/url "https://github.com/user/my-lib"
           :git/sha "abc1234ef"}}}
```

```
cljrs deps fetch my.lib
```

If the required commit is not cached, clojurust raises a descriptive error
rather than attempting a network fetch:

```
error: dependency 'my.lib' is not cached locally.
       run `cljrs deps fetch` to download it.
```

## Signature verification

When `--verify-commit-signatures` is passed on the CLI (or
`:verify-commit-signatures true` is set in `cljrs.edn`), clojurust verifies
that every accessed versioned commit carries a valid GPG or SSH signature
before executing its code.

## Notes

- Versioned symbols are resolved lazily at call time, not at load time, so the
  dependency only needs to be cached the first time the code path is actually
  executed.
- The version cache is per-`GlobalEnv` and is keyed on
  `"<ns>/<name>@<commit>"`, so the same commit of the same namespace is loaded
  at most once per interpreter session.
