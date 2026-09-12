# cljrs deps

Manage project dependencies declared in `cljrs.edn`.

```
cljrs deps <SUBCOMMAND>
```

## Subcommands

| Subcommand | Description |
|---|---|
| [`fetch`](#fetch) | Clone or update git dependencies |
| [`status`](#status) | Show which dependencies are cached and which are missing |

---

## fetch

```
cljrs deps fetch [NAME]
```

Clone or update git dependencies from `cljrs.edn`. Without a `NAME`, fetches
every git dependency declared in the nearest `cljrs.edn`. With a `NAME`, fetches
only that one dependency.

Git repositories are cached in `~/.cljrs/cache/git/`. Network access **only**
occurs when this command is run explicitly — the runtime never fetches
dependencies automatically.

If a versioned symbol or namespace requires a git dependency that is not in the
local cache, the runtime raises a clear error:

```
error: dependency 'my.lib' is not cached locally.
       run `cljrs deps fetch` to download it.
```

### Examples

```
cljrs deps fetch           # fetch all git deps
cljrs deps fetch my.lib    # fetch only 'my.lib'
```

---

## status

```
cljrs deps status
```

Print the cache status of every dependency declared in the nearest `cljrs.edn`.

```
my.lib:    cached (sha: abc1234ef, url: https://github.com/user/my-lib)
dev-tools: NOT cached — run `cljrs deps fetch` (sha: 9f3a112b, url: ...)
vendor:    local dep at ../vendor/utils — ok
```

Exits with code `0` if all dependencies are satisfied, `1` otherwise.

---

## `cljrs.edn` format

clojurust discovers project configuration by walking up the directory tree from
the current working directory until it finds a `cljrs.edn` file. The file is
valid clojurust EDN:

```clojure
{:paths ["src" "resources"]

 :deps
 {my.lib    {:git/url "https://github.com/user/my-lib"
              :git/sha "abc1234ef"}
  dev-tools {:git/url "https://github.com/user/dev-tools"
              :git/sha "9f3a112b"}
  vendor    {:local/root "../vendor/utils"}}

 :aliases
 {:dev  {:extra-paths ["dev"]}
  :test {:extra-paths ["test"]
         :extra-deps  {test-tools {:git/url "..."
                                   :git/sha "..."}}}}

 :verify-commit-signatures true

 ; Keys allowed to sign versioned commits (inline key or path to a key file).
 :trusted-signers ["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5... maintainer@example.com"
                   "keys/release-signing.asc"]

 ; Optional: embed a Rust crate for native interop
 :rust {:crate "."
        :init  "my_project::cljrs_init_my_project"}}
```

### Keys

| Key | Type | Description |
|---|---|---|
| `:paths` | vector of strings | Directories to add to the source path. Equivalent to `--src-path` on the CLI. |
| `:deps` | map | Map from dependency name (symbol) to dependency descriptor. |
| `:aliases` | map | Named alias maps with `:extra-paths` and `:extra-deps`. |
| `:verify-commit-signatures` | boolean | If `true`, require valid PGP/SSH signatures (verified natively) on all versioned commits. |
| `:trusted-signers` | vector of strings | Public keys allowed to sign versioned commits. Each entry is an inline key (armored PGP or OpenSSH) or a path to a key file relative to `cljrs.edn`. |
| `:rust` | map | Embedded Rust crate for native interop. See [Rust Interop](../rust-interop/index.md). |

#### Git dependency URLs

`:git/url` accepts `https://` URLs and local paths (and `file://`), all fetched
in-process with pure-Rust gitoxide — no `git` binary required. The `cljrs`
binary additionally fetches `ssh://` and scp-like `git@host:path` URLs natively
over SSH: host keys are verified against `~/.ssh/known_hosts`, and authentication
uses a running ssh-agent (`$SSH_AUTH_SOCK`). Other schemes (`git://`, `http://`)
are rejected.

#### `:rust` key

```clojure
:rust {:crate "."                       ; path to Cargo.toml directory
       :init  "my_project::cljrs_init_my_project"} ; Rust path to the init function
```

| Sub-key | Description |
|---|---|
| `:crate` | Directory containing the user's `Cargo.toml`, relative to `cljrs.edn`. |
| `:init` | Fully-qualified Rust path to the init function. The first `::` segment is treated as the crate name. |

When `:rust` is present, `cljrs run` and `cljrs repl` automatically load the
compiled shared library from `<crate>/target/debug/lib<name>.so` (or
equivalent). Build it first with `cljrs build-native`.

### Dependency descriptors

**Git dependency:**

```clojure
my.lib {:git/url "https://github.com/user/my-lib"
        :git/sha "abc1234ef"}
```

`:git/sha` must be at least a 7-character commit prefix. The full commit hash
is recommended for reproducibility.

**Local dependency:**

```clojure
vendor {:local/root "../vendor/utils"}
```

`:local/root` is a path relative to the `cljrs.edn` file's directory.

### Native dependencies

A dependency that ships Rust code can have that code built as a cdylib and
loaded into the runtime, so `(require '[my.lib])` brings in a namespace no
Clojure source provides. Opt in with `:rust/load :dylib` and name the crate's
init function:

```clojure
{:deps
 {my.lib {:git/url  "https://github.com/user/my-lib"
          :git/sha  "abc1234ef"
          :rust/load :dylib
          :rust/init "my_lib::cljrs_init"}}}
```

`:rust/init` is the fully-qualified path to a `pub fn(&mut Registry)`. Add
`:rust/crate "path/to/crate"` when the crate is not at the dependency's root.

**Developing one locally.** The same keys work on a `:local/root` dependency,
which builds the working tree as it currently stands — no commit, no push:

```clojure
{:deps
 {my.lib {:local/root "../my-lib"
          :rust/load  :dylib
          :rust/init  "my_lib::cljrs_init"
          :rust/crate "src/crates/thing"}}}
```

Edit the crate and the next `require` rebuilds it: a local dependency is
versioned by a digest of its source files, so a changed tree is a different
build. The digest covers the whole `:local/root`, not just the `:rust/crate`
subdirectory, so editing a sibling crate the extension depends on also
rebuilds. This is the loop for writing an extension; pin it with `:git/sha`
to ship it.

Two limits apply to a local native dependency, both following from its having
no commit:

- It cannot serve a **versioned symbol** (`my.lib/f@<sha>`). Those resolve
  against pinned dependencies only; a local one is skipped.
- It is **not reproducible**. Two machines with different working trees get
  different builds, silently. Only `:git/sha` makes a build repeatable.

Builds are cached under `~/.cljrs/cache/dylibs/` per crate, version, compiler
and cljrs version, and are guarded by an ABI handshake: a wrapper built by a
different `rustc`, profile, or cljrs version is refused rather than loaded.
The generated wrapper crate and its cargo target directory are shared by every
version of one dependency, so an edit costs an incremental rebuild; only the
built library is copied out to a per-version path, which is what makes a
rebuilt library load instead of the one already open.

Set `CLJRS_DYLIB_OFFLINE=1` to build wrappers with `cargo --offline`, for a
machine whose cargo cache already holds everything the dependency needs.
