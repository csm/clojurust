//! Global keyword and symbol intern tables (Phase B3).
//!
//! Interned keywords and symbols are allocated once into program-lifetime
//! memory and reused for every subsequent request with the same identity.
//! This gives each unique (namespace, name) pair a stable address that is
//! consistent across all isolates — required for correct hash-map key lookups
//! when maps move between isolates via the structured-clone boundary.
//!
//! ## Design
//!
//! Each table is a `OnceLock<Mutex<HashMap<…, StaticGcPtr<T>>>>`.  On the
//! first call for a given key the value is allocated via [`cljrs_gc::static_alloc`]
//! (arena in `no-gc` builds, `Box::leak` in GC builds) and the pointer is
//! inserted.  Subsequent calls return a clone of the stored pointer (O(1),
//! just copies a `NonNull`).
//!
//! ## Contention
//!
//! The global `Mutex` is only held during the brief table lookup + optional
//! insert.  Keywords are created at read/compile time, not in hot evaluation
//! loops, so contention is not a concern for now.  A sharded or lock-free
//! table can replace this if profiling ever shows otherwise.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use cljrs_gc::{StaticGcPtr, static_alloc};

use crate::keyword::Keyword;
use crate::symbol::Symbol;

// ── Keyword intern table ──────────────────────────────────────────────────────

type KwKey = (Option<Arc<str>>, Arc<str>);
static KEYWORD_TABLE: OnceLock<Mutex<HashMap<KwKey, StaticGcPtr<Keyword>>>> = OnceLock::new();

fn kw_table() -> &'static Mutex<HashMap<KwKey, StaticGcPtr<Keyword>>> {
    KEYWORD_TABLE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Intern a keyword into program-lifetime memory.
///
/// The first call for a given `(namespace, name)` pair allocates the
/// `Keyword` and stores it; subsequent calls return the same `StaticGcPtr`.
/// The returned pointer is `Send + Sync` and valid for the lifetime of the
/// process.
pub fn intern_keyword(namespace: Option<&str>, name: &str) -> StaticGcPtr<Keyword> {
    let ns_arc: Option<Arc<str>> = namespace.map(Arc::from);
    let name_arc: Arc<str> = Arc::from(name);
    let key = (ns_arc.clone(), name_arc.clone());

    let mut table = kw_table().lock().unwrap();
    if let Some(existing) = table.get(&key) {
        return existing.clone();
    }
    let kw = Keyword {
        namespace: ns_arc,
        name: name_arc,
    };
    let ptr = static_alloc(kw);
    table.insert(key, ptr.clone());
    ptr
}

// ── Symbol intern table ───────────────────────────────────────────────────────

type SymKey = (Option<Arc<str>>, Arc<str>, Option<Arc<str>>);
static SYMBOL_TABLE: OnceLock<Mutex<HashMap<SymKey, StaticGcPtr<Symbol>>>> = OnceLock::new();

fn sym_table() -> &'static Mutex<HashMap<SymKey, StaticGcPtr<Symbol>>> {
    SYMBOL_TABLE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Intern a symbol into program-lifetime memory.
///
/// The first call for a given `(namespace, name, version)` triple allocates
/// the `Symbol`; subsequent calls return the same `StaticGcPtr`.
pub fn intern_symbol(
    namespace: Option<&str>,
    name: &str,
    version: Option<&str>,
) -> StaticGcPtr<Symbol> {
    let ns_arc: Option<Arc<str>> = namespace.map(Arc::from);
    let name_arc: Arc<str> = Arc::from(name);
    let ver_arc: Option<Arc<str>> = version.map(Arc::from);
    let key = (ns_arc.clone(), name_arc.clone(), ver_arc.clone());

    let mut table = sym_table().lock().unwrap();
    if let Some(existing) = table.get(&key) {
        return existing.clone();
    }
    let sym = Symbol {
        namespace: ns_arc,
        name: name_arc,
        version: ver_arc,
    };
    let ptr = static_alloc(sym);
    table.insert(key, ptr.clone());
    ptr
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cljrs_gc::StaticGcPtr;

    #[test]
    fn keyword_intern_returns_same_ptr_for_same_name() {
        let a = intern_keyword(None, "foo");
        let b = intern_keyword(None, "foo");
        assert!(
            StaticGcPtr::ptr_eq(&a, &b),
            "same name must return same pointer"
        );
    }

    #[test]
    fn keyword_intern_qualified() {
        let a = intern_keyword(Some("clojure.core"), "map");
        let b = intern_keyword(Some("clojure.core"), "map");
        assert!(StaticGcPtr::ptr_eq(&a, &b));
        assert_eq!(a.get().namespace.as_deref(), Some("clojure.core"));
        assert_eq!(a.get().name.as_ref(), "map");
    }

    #[test]
    fn keyword_intern_different_names_differ() {
        let a = intern_keyword(None, "foo");
        let b = intern_keyword(None, "bar");
        assert!(!StaticGcPtr::ptr_eq(&a, &b));
    }

    #[test]
    fn symbol_intern_returns_same_ptr() {
        let a = intern_symbol(None, "foo", None);
        let b = intern_symbol(None, "foo", None);
        assert!(StaticGcPtr::ptr_eq(&a, &b));
    }

    #[test]
    fn symbol_intern_versioned() {
        let a = intern_symbol(Some("my.ns"), "myfn", Some("abc1234"));
        let b = intern_symbol(Some("my.ns"), "myfn", Some("abc1234"));
        assert!(StaticGcPtr::ptr_eq(&a, &b));
        assert_eq!(a.get().version.as_deref(), Some("abc1234"));
    }
}
