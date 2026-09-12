//! The datatype, protocol and multimethod family — one definition, read by
//! every pass that has to route these forms away from compiled code.
//!
//! Membership is a property of the *language*, not of any one pass, so it is
//! stated once here. `crates/cljrs-ir/src/lower/anf.rs` reads it to decline
//! lowering; `crates/cljrs-compiler/src/aot.rs` reads it in both
//! `needs_interpreter` (pre-expansion) and `expanded_needs_interpreter`
//! (post-expansion).

/// Head symbols of the datatype, protocol and multimethod family: the surface
/// names and the `*` primitives they expand to.
///
/// Both spellings are members. A pre-expansion caller sees `deftype`; a
/// post-expansion caller sees `deftype*`, because `deftype`, `defrecord` and
/// `reify` are macros in `bootstrap.cljrs` that all expand to it, and
/// `defprotocol` expands to `protocol*`. A pass that knows only one spelling
/// is a pass with a hole in it.
///
/// Membership is the contract; what a caller does about it is the caller's.
pub const DISPATCH_FAMILY: &[&str] = &[
    "defprotocol",
    "protocol*",
    "extend-type",
    "extend-protocol",
    "defmulti",
    "defmethod",
    "defrecord",
    "deftype",
    "deftype*",
    "reify",
];

/// True when `head` names a member of [`DISPATCH_FAMILY`].
///
/// Any namespace qualifier is stripped first, so `clojure.core/deftype` is a
/// member exactly as `deftype` is.
pub fn in_dispatch_family(head: &str) -> bool {
    let base = head.rsplit('/').next().unwrap_or(head);
    DISPATCH_FAMILY.contains(&base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_names_and_primitives_are_both_members() {
        for name in ["deftype", "deftype*", "defrecord", "reify"] {
            assert!(in_dispatch_family(name), "{name} must be a member");
        }
        for name in ["defprotocol", "protocol*"] {
            assert!(in_dispatch_family(name), "{name} must be a member");
        }
    }

    #[test]
    fn a_namespace_qualifier_is_stripped() {
        assert!(in_dispatch_family("clojure.core/deftype"));
        assert!(in_dispatch_family("clojure.core/deftype*"));
    }

    #[test]
    fn an_unrelated_head_is_not_a_member() {
        for name in ["defn", "let*", "fn*", "def", "deftype-ish", "user/reifyish"] {
            assert!(!in_dispatch_family(name), "{name} must not be a member");
        }
    }
}
