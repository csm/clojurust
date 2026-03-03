use std::sync::Arc;

/// An interned Clojure keyword, optionally namespace-qualified.
///
/// `:foo` → `Keyword { namespace: None, name: "foo" }`
/// `:clojure.core/map` → `Keyword { namespace: Some("clojure.core"), name: "map" }`
///
/// Keywords are value types: two keywords with the same namespace and name
/// are always equal.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Keyword {
    pub namespace: Option<Arc<str>>,
    pub name: Arc<str>,
}

impl Keyword {
    /// Unqualified keyword.
    pub fn simple(name: impl Into<Arc<str>>) -> Self {
        Self {
            namespace: None,
            name: name.into(),
        }
    }

    /// Namespace-qualified keyword.
    pub fn qualified(ns: impl Into<Arc<str>>, name: impl Into<Arc<str>>) -> Self {
        Self {
            namespace: Some(ns.into()),
            name: name.into(),
        }
    }

    /// Parse a keyword from the bare name string (without leading `:`).
    pub fn parse(s: &str) -> Self {
        match s.find('/') {
            Some(idx) if idx > 0 && idx < s.len() - 1 => Self::qualified(&s[..idx], &s[idx + 1..]),
            _ => Self::simple(s),
        }
    }

    /// The fully-qualified string without the leading colon.
    pub fn full_name(&self) -> String {
        match &self.namespace {
            Some(ns) => format!("{}/{}", ns, self.name),
            None => self.name.to_string(),
        }
    }
}

impl std::fmt::Display for Keyword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.namespace {
            Some(ns) => write!(f, ":{}/{}", ns, self.name),
            None => write!(f, ":{}", self.name),
        }
    }
}

impl cljx_gc::Trace for Keyword {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display() {
        assert_eq!(Keyword::simple("foo").to_string(), ":foo");
        assert_eq!(Keyword::qualified("ns", "name").to_string(), ":ns/name");
    }

    #[test]
    fn test_parse() {
        assert_eq!(Keyword::parse("foo"), Keyword::simple("foo"));
        assert_eq!(Keyword::parse("a/b"), Keyword::qualified("a", "b"));
    }

    #[test]
    fn test_equality() {
        assert_eq!(Keyword::simple("a"), Keyword::simple("a"));
        assert_ne!(Keyword::simple("a"), Keyword::simple("b"));
    }
}
