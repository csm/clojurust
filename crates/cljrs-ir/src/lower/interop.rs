//! The method-call sugar, recognized once.
//!
//! Read by `lower/anf.rs` (which name-marks it as a `CallDirect`), by
//! `cljrs-compiler`'s AOT driver (which routes a form containing one to the
//! interpreted preamble) and by the tree-walking interpreter (which dispatches
//! it directly).

/// True when `head` is the `.method` / `.-field` interop sugar.
///
/// Not a member: `..`, which is the threading macro, and a bare `.`, which is
/// the classic `(. target method args…)` form with a handler of its own.
pub fn is_method_sugar(head: &str) -> bool {
    head.len() > 1 && head != ".." && head.starts_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dotted_head_is_method_sugar() {
        for name in [".toUpperCase", ".-field", ".x"] {
            assert!(is_method_sugar(name), "{name} must be method sugar");
        }
    }

    #[test]
    fn the_threading_macro_and_the_classic_form_are_not() {
        for name in ["..", ".", "", "toUpperCase", "clojure.core/inc"] {
            assert!(!is_method_sugar(name), "{name} must not be method sugar");
        }
    }
}
