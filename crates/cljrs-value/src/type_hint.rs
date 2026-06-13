//! Primitive type hints (`^long`, `^double`, `^longs`, …).
//!
//! Clojure lets a name be tagged with a type via reader metadata
//! (`^long x`, expanded to `{:tag long}`).  For the *primitive* tags the
//! compiler can keep the value unboxed and emit specialized arithmetic and
//! array access.  This enum is the parsed, normalized form of such a tag; the
//! interpreter records one per function parameter (`CljxFnArity::param_hints`)
//! and the IR lowering maps it onto a representation seed for type inference.
//!
//! Non-primitive tags (`^String`, `^MyRecord`, …) are *advisory* in Clojure and
//! carry no unboxing semantics here, so they resolve to `None` and are ignored.

/// A primitive type hint usable for unboxing / array specialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeHint {
    /// `^long` — unboxed `i64`.
    Long,
    /// `^double` — unboxed `f64`.
    Double,
    /// `^int` — 32-bit int (treated as a long-family scalar for inference).
    Int,
    /// `^float` — 32-bit float (treated as a double-family scalar).
    Float,
    /// `^boolean` — unboxed `i8` truth value.
    Bool,
    /// `^longs` — array of `i64`.
    LongArray,
    /// `^doubles` — array of `f64`.
    DoubleArray,
    /// `^ints` — array of `i32`.
    IntArray,
    /// `^floats` — array of `f32`.
    FloatArray,
    /// `^booleans` — array of `bool`.
    BooleanArray,
    /// `^objects` — array of boxed values.
    ObjectArray,
}

impl TypeHint {
    /// Resolve a `:tag` symbol name to a primitive hint, or `None` for tags
    /// that carry no primitive/unboxing meaning (e.g. `^String`).
    ///
    /// Recognizes the standard Clojure spellings.  A leading namespace, if any,
    /// is ignored by callers before this point.
    pub fn from_tag(name: &str) -> Option<TypeHint> {
        Some(match name {
            "long" => TypeHint::Long,
            "double" => TypeHint::Double,
            "int" => TypeHint::Int,
            "float" => TypeHint::Float,
            "boolean" => TypeHint::Bool,
            "longs" => TypeHint::LongArray,
            "doubles" => TypeHint::DoubleArray,
            "ints" => TypeHint::IntArray,
            "floats" => TypeHint::FloatArray,
            "booleans" => TypeHint::BooleanArray,
            "objects" => TypeHint::ObjectArray,
            _ => return None,
        })
    }

    /// Whether this hint denotes a primitive array type.
    pub fn is_array(&self) -> bool {
        matches!(
            self,
            TypeHint::LongArray
                | TypeHint::DoubleArray
                | TypeHint::IntArray
                | TypeHint::FloatArray
                | TypeHint::BooleanArray
                | TypeHint::ObjectArray
        )
    }

    /// The scalar element hint of an array hint (`^longs` → `^long`), or `None`
    /// for scalar hints and the boxed object array.
    pub fn element(&self) -> Option<TypeHint> {
        Some(match self {
            TypeHint::LongArray => TypeHint::Long,
            TypeHint::DoubleArray => TypeHint::Double,
            TypeHint::IntArray => TypeHint::Int,
            TypeHint::FloatArray => TypeHint::Float,
            TypeHint::BooleanArray => TypeHint::Bool,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_scalar_tags() {
        assert_eq!(TypeHint::from_tag("long"), Some(TypeHint::Long));
        assert_eq!(TypeHint::from_tag("double"), Some(TypeHint::Double));
        assert_eq!(TypeHint::from_tag("boolean"), Some(TypeHint::Bool));
    }

    #[test]
    fn resolves_array_tags() {
        assert_eq!(TypeHint::from_tag("longs"), Some(TypeHint::LongArray));
        assert_eq!(TypeHint::from_tag("doubles"), Some(TypeHint::DoubleArray));
        assert!(TypeHint::LongArray.is_array());
        assert_eq!(TypeHint::LongArray.element(), Some(TypeHint::Long));
    }

    #[test]
    fn unknown_tag_is_none() {
        assert_eq!(TypeHint::from_tag("String"), None);
        assert_eq!(TypeHint::from_tag("MyRecord"), None);
    }
}
