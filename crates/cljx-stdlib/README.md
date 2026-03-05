# cljx-stdlib

Built-in standard library namespaces for clojurust, distributed as embedded
source + native Rust helpers.

## Status

Phase 8-ext.  Provides `clojure.string`, `clojure.set`, and `clojure.test` as
lazily-loaded built-ins; no filesystem dependency at runtime.

## Purpose

Clojurust has no classpath or JAR mechanism.  This crate solves the distribution
problem by embedding `.cljrs` source files via `include_str!` and registering
them in `GlobalEnv::builtin_sources` so that `(require '[clojure.string :as str])`
works out of the box in any binary that calls `cljx_stdlib::standard_env()`.

## File layout

```
src/
  lib.rs                  Public API: register(), standard_env(), standard_env_with_paths()
  string.rs               Native Rust implementations for clojure.string
  set.rs                  Native Rust implementations for clojure.set
  clojure/
    string.cljrs          Clojure source for clojure.string (ns decl; natives pre-registered)
    set.cljrs             Clojure source for clojure.set   (ns decl; natives pre-registered)
    test.cljrs            Pure Clojure implementation of clojure.test
```

## Public API

### Entry points

```rust
/// Register all built-in stdlib namespaces into an existing GlobalEnv.
pub fn register(globals: &Arc<GlobalEnv>);

/// Create a GlobalEnv with bootstrap + stdlib registered (lazy loading).
pub fn standard_env() -> Arc<GlobalEnv>;

/// Like standard_env() but also sets user source paths for require.
pub fn standard_env_with_paths(source_paths: Vec<PathBuf>) -> Arc<GlobalEnv>;
```

### Namespaces provided

| Namespace | Implementation | Notes |
|-----------|---------------|-------|
| `clojure.string` | `string.rs` + `clojure/string.cljrs` | Native Rust, loaded lazily |
| `clojure.set` | `set.rs` + `clojure/set.cljrs` | Native Rust, loaded lazily |
| `clojure.test` | `clojure/test.cljrs` | Pure Clojure, loaded lazily |

### clojure.string functions

`upper-case`, `lower-case`, `capitalize`, `trim`, `triml`, `trimr`,
`trim-newline`, `blank?`, `starts-with?`, `ends-with?`, `includes?`,
`replace`, `replace-first`, `split`, `split-lines`, `join`,
`index-of`, `last-index-of`

### clojure.set functions

`union`, `intersection`, `difference`, `subset?`, `superset?`,
`select`, `map-invert`

## Dependency notes

- `cljx-stdlib` depends on `cljx-eval` (for `GlobalEnv`, `standard_env_minimal`)
- `cljx-eval` does **not** depend on `cljx-stdlib` (no circular dep)
- The `cljx` binary depends on both; use `cljx_stdlib::standard_env()` instead of
  `cljx_eval::standard_env()` so that stdlib namespaces are available
