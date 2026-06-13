//! Known-function name → KnownFn dispatch table.
//!
//! Maps Clojure symbol names (possibly namespace-qualified) to `KnownFn`
//! variants.

use crate::KnownFn;

/// Strip a namespace prefix from a symbol/function name.
/// `"clojure.core/+"` → `"+"`, `"+"` → `"+"`, `"/"` → `"/"`.
pub fn strip_ns_prefix(s: &str) -> &str {
    if s == "/" {
        return s;
    }
    match s.rfind('/') {
        Some(pos) => &s[pos + 1..],
        None => s,
    }
}

/// Resolve a symbol name to a `KnownFn`, or `None` if not recognized.
/// Strips namespace prefix so `"clojure.core/+"` and `"+"` both resolve.
pub fn resolve_known_fn(sym_name: &str) -> Option<KnownFn> {
    match strip_ns_prefix(sym_name) {
        "vector" => Some(KnownFn::Vector),
        "hash-map" => Some(KnownFn::HashMap),
        "hash-set" => Some(KnownFn::HashSet),
        "list" => Some(KnownFn::List),
        "assoc" => Some(KnownFn::Assoc),
        "dissoc" => Some(KnownFn::Dissoc),
        "conj" => Some(KnownFn::Conj),
        "disj" => Some(KnownFn::Disj),
        "get" => Some(KnownFn::Get),
        "nth" => Some(KnownFn::Nth),
        "count" => Some(KnownFn::Count),
        "contains?" => Some(KnownFn::Contains),
        "transient" => Some(KnownFn::Transient),
        "assoc!" => Some(KnownFn::AssocBang),
        "conj!" => Some(KnownFn::ConjBang),
        "persistent!" => Some(KnownFn::PersistentBang),
        "first" => Some(KnownFn::First),
        "rest" => Some(KnownFn::Rest),
        "next" => Some(KnownFn::Next),
        "cons" => Some(KnownFn::Cons),
        "seq" => Some(KnownFn::Seq),
        "lazy-seq" => Some(KnownFn::LazySeq),
        "make-lazy-seq" => Some(KnownFn::LazySeq),
        "empty?" => Some(KnownFn::IsEmpty),
        "peek" => Some(KnownFn::Peek),
        "pop" => Some(KnownFn::Pop),
        "vec" => Some(KnownFn::Vec),
        "mapcat" => Some(KnownFn::Mapcat),
        "repeatedly" => Some(KnownFn::Repeatedly),
        "+" => Some(KnownFn::Add),
        "-" => Some(KnownFn::Sub),
        "*" => Some(KnownFn::Mul),
        "/" => Some(KnownFn::Div),
        "rem" => Some(KnownFn::Rem),
        "mod" => Some(KnownFn::Rem),
        "=" => Some(KnownFn::Eq),
        "<" => Some(KnownFn::Lt),
        ">" => Some(KnownFn::Gt),
        "<=" => Some(KnownFn::Lte),
        ">=" => Some(KnownFn::Gte),
        "nil?" => Some(KnownFn::IsNil),
        "seq?" => Some(KnownFn::IsSeq),
        "vector?" => Some(KnownFn::IsVector),
        "map?" => Some(KnownFn::IsMap),
        "identical?" => Some(KnownFn::Identical),
        "str" => Some(KnownFn::Str),
        "deref" => Some(KnownFn::Deref),
        "println" => Some(KnownFn::Println),
        "pr" => Some(KnownFn::Pr),
        "apply" => Some(KnownFn::Apply),
        "reduce" => Some(KnownFn::Reduce2), // dispatched by arity in lower_call
        "map" => Some(KnownFn::Map),
        "filter" => Some(KnownFn::Filter),
        "mapv" => Some(KnownFn::Mapv),
        "filterv" => Some(KnownFn::Filterv),
        "some" => Some(KnownFn::Some),
        "every?" => Some(KnownFn::Every),
        "into" => Some(KnownFn::Into), // dispatched by arity
        "concat" => Some(KnownFn::Concat),
        "range" => Some(KnownFn::Range1), // dispatched by arity
        "take" => Some(KnownFn::Take),
        "drop" => Some(KnownFn::Drop),
        "reverse" => Some(KnownFn::Reverse),
        "sort" => Some(KnownFn::Sort),
        "sort-by" => Some(KnownFn::SortBy),
        "keys" => Some(KnownFn::Keys),
        "vals" => Some(KnownFn::Vals),
        "merge" => Some(KnownFn::Merge),
        "update" => Some(KnownFn::Update),
        "get-in" => Some(KnownFn::GetIn),
        "assoc-in" => Some(KnownFn::AssocIn),
        "number?" => Some(KnownFn::IsNumber),
        "string?" => Some(KnownFn::IsString),
        "keyword?" => Some(KnownFn::IsKeyword),
        "symbol?" => Some(KnownFn::IsSymbol),
        "boolean?" => Some(KnownFn::IsBool),
        "int?" => Some(KnownFn::IsInt),
        "prn" => Some(KnownFn::Prn),
        "print" => Some(KnownFn::Print),
        "atom" => Some(KnownFn::Atom),
        "reset!" => Some(KnownFn::AtomReset),
        "swap!" => Some(KnownFn::AtomSwap),
        "group-by" => Some(KnownFn::GroupBy),
        "partition" => Some(KnownFn::Partition2), // dispatched by arity
        "frequencies" => Some(KnownFn::Frequencies),
        "keep" => Some(KnownFn::Keep),
        "remove" => Some(KnownFn::Remove),
        "map-indexed" => Some(KnownFn::MapIndexed),
        "zipmap" => Some(KnownFn::Zipmap),
        "juxt" => Some(KnownFn::Juxt),
        "comp" => Some(KnownFn::Comp),
        "partial" => Some(KnownFn::Partial),
        "complement" => Some(KnownFn::Complement),
        _ => None,
    }
}
