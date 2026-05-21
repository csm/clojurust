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

 ; Optional: embed a Rust crate for native interop
 :rust {:crate "."
        :init  "my_project::cljrs_init"}}
```

### Keys

| Key | Type | Description |
|---|---|---|
| `:paths` | vector of strings | Directories to add to the source path. Equivalent to `--src-path` on the CLI. |
| `:deps` | map | Map from dependency name (symbol) to dependency descriptor. |
| `:aliases` | map | Named alias maps with `:extra-paths` and `:extra-deps`. |
| `:verify-commit-signatures` | boolean | If `true`, require GPG/SSH signatures on all versioned commits. |
| `:rust` | map | Embedded Rust crate for native interop. See [Rust Interop](../rust-interop/index.md). |

#### `:rust` key

```clojure
:rust {:crate "."                       ; path to Cargo.toml directory
       :init  "my_project::cljrs_init"} ; Rust path to the init function
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
