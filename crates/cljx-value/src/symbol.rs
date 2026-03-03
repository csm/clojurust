use std::sync::Arc;

/// An interned Clojure symbol, optionally namespace-qualified.
///
/// `"foo"` → `Symbol { namespace: None, name: "foo" }`
/// `"clojure.core/map"` → `Symbol { namespace: Some("clojure.core"), name: "map" }`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Symbol {
    pub namespace: Option<Arc<str>>,
    pub name: Arc<str>,
}

impl Symbol {
    /// Unqualified symbol.
    pub fn simple(name: impl Into<Arc<str>>) -> Self {
        Self {
            namespace: None,
            name: name.into(),
        }
    }

    /// Namespace-qualified symbol.
    pub fn qualified(ns: impl Into<Arc<str>>, name: impl Into<Arc<str>>) -> Self {
        Self {
            namespace: Some(ns.into()),
            name: name.into(),
        }
    }

    /// Parse a symbol from a string of the form `"ns/name"` or `"name"`.
    pub fn parse(s: &str) -> Self {
        match s.find('/') {
            Some(idx) if idx > 0 && idx < s.len() - 1 => Self::qualified(&s[..idx], &s[idx + 1..]),
            _ => Self::simple(s),
        }
    }

    /// The fully-qualified string, e.g. `"clojure.core/map"` or `"foo"`.
    pub fn full_name(&self) -> String {
        match &self.namespace {
            Some(ns) => format!("{}/{}", ns, self.name),
            None => self.name.to_string(),
        }
    }
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.namespace {
            Some(ns) => write!(f, "{}/{}", ns, self.name),
            None => write!(f, "{}", self.name),
        }
    }
}

impl cljx_gc::Trace for Symbol {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple() {
        let s = Symbol::simple("foo");
        assert_eq!(s.name.as_ref(), "foo");
        assert!(s.namespace.is_none());
        assert_eq!(s.full_name(), "foo");
    }

    #[test]
    fn test_qualified() {
        let s = Symbol::qualified("clojure.core", "map");
        assert_eq!(s.full_name(), "clojure.core/map");
    }

    #[test]
    fn test_parse() {
        assert_eq!(Symbol::parse("foo"), Symbol::simple("foo"));
        assert_eq!(Symbol::parse("a/b"), Symbol::qualified("a", "b"));
        // bare "/" is unqualified
        assert_eq!(Symbol::parse("/").namespace, None);
    }
}
