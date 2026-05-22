# cljrs-env

Environments for running programs in.

---

## Public API additions — native versioned symbol registry

`GlobalEnv` carries two registries related to versioned symbol resolution:

| Method | Description |
|--------|-------------|
| `register_native_versioned(ns, name, commit, val)` | Store an explicit versioned binding for a native Rust function. Key: `"ns/name@commit"`. Called by `Registry::define_versioned`. |
| `get_native_versioned(ns, name, commit) -> Option<Value>` | Retrieve an explicitly registered versioned native binding. Checked by `resolve_versioned_symbol` before the git-source lookup path. |
| `cache_versioned(ns, name, commit, val)` | Store a resolved value (Clojure-defined *or* native) in the session-scoped `version_cache`. |
| `get_cached_versioned(ns, name, commit) -> Option<Value>` | Retrieve a cached versioned value. Checked first on every versioned symbol lookup. |

The `native_version_registry` field (`Mutex<HashMap<Arc<str>, Value>>`) is separate from `version_cache` so that explicitly registered native bindings persist independently of the resolution cache and are never evicted.