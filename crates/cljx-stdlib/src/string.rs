//! Native implementations for `clojure.string`.

use std::sync::Arc;

use cljx_gc::GcPtr;
use cljx_value::{Arity, PersistentVector, Value, ValueError, ValueResult};

use crate::register_fns;

pub fn register(globals: &Arc<cljx_eval::GlobalEnv>, ns: &str) {
    register_fns!(
        globals,
        ns,
        [
            ("upper-case", Arity::Fixed(1), upper_case),
            ("lower-case", Arity::Fixed(1), lower_case),
            ("capitalize", Arity::Fixed(1), capitalize),
            ("trim", Arity::Fixed(1), trim),
            ("triml", Arity::Fixed(1), triml),
            ("trimr", Arity::Fixed(1), trimr),
            ("trim-newline", Arity::Fixed(1), trim_newline),
            ("blank?", Arity::Fixed(1), blank_q),
            ("starts-with?", Arity::Fixed(2), starts_with_q),
            ("ends-with?", Arity::Fixed(2), ends_with_q),
            ("includes?", Arity::Fixed(2), includes_q),
            ("replace", Arity::Fixed(3), replace),
            ("replace-first", Arity::Fixed(3), replace_first),
            ("split", Arity::Variadic { min: 2 }, split),
            ("split-lines", Arity::Fixed(1), split_lines),
            ("join", Arity::Variadic { min: 1 }, join),
            ("index-of", Arity::Variadic { min: 2 }, index_of),
            ("last-index-of", Arity::Variadic { min: 2 }, last_index_of),
            ("reverse", Arity::Fixed(1), string_reverse),
            ("escape", Arity::Fixed(2), string_escape),
        ]
    );
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Coerce a value to a string via `.toString()` semantics.
/// Keywords → ":foo", symbols → "foo", numbers → "42", nil throws.
fn get_str(v: &Value) -> ValueResult<Arc<str>> {
    match v {
        Value::Str(s) => Ok(s.get().clone().into()),
        Value::Keyword(k) => Ok(Arc::from(format!(":{}", k.get().full_name()))),
        Value::Symbol(s) => {
            let sym = s.get();
            Ok(Arc::from(match &sym.namespace {
                Some(ns) => format!("{}/{}", ns, sym.name),
                None => sym.name.as_ref().to_string(),
            }))
        }
        Value::Long(n) => Ok(Arc::from(n.to_string())),
        Value::Double(f) => Ok(Arc::from(f.to_string())),
        Value::Char(c) => Ok(Arc::from(c.to_string())),
        Value::Bool(b) => Ok(Arc::from(b.to_string())),
        Value::Nil => Err(ValueError::WrongType {
            expected: "string",
            got: "nil".to_string(),
        }),
        other => Err(ValueError::WrongType {
            expected: "string",
            got: other.type_name().to_string(),
        }),
    }
}

/// Strict: only accepts strings, throws on everything else (including nil).
fn get_strict_str(v: &Value) -> ValueResult<Arc<str>> {
    match v {
        Value::Str(s) => Ok(s.get().clone().into()),
        other => Err(ValueError::WrongType {
            expected: "string",
            got: other.type_name().to_string(),
        }),
    }
}

fn make_str(s: String) -> Value {
    Value::Str(GcPtr::new(s))
}

// ── Implementations ────────────────────────────────────────────────────────────

fn upper_case(args: &[Value]) -> ValueResult<Value> {
    let s = get_str(&args[0])?;
    Ok(make_str(s.to_uppercase()))
}

fn lower_case(args: &[Value]) -> ValueResult<Value> {
    let s = get_str(&args[0])?;
    Ok(make_str(s.to_lowercase()))
}

fn capitalize(args: &[Value]) -> ValueResult<Value> {
    let s = get_str(&args[0])?;
    let mut chars = s.chars();
    let result = match chars.next() {
        None => String::new(),
        Some(first) => {
            let upper: String = first.to_uppercase().collect();
            upper + &chars.as_str().to_lowercase()
        }
    };
    Ok(make_str(result))
}

fn trim(args: &[Value]) -> ValueResult<Value> {
    let s = get_str(&args[0])?;
    Ok(make_str(s.trim().to_string()))
}

fn triml(args: &[Value]) -> ValueResult<Value> {
    let s = get_str(&args[0])?;
    Ok(make_str(s.trim_start().to_string()))
}

fn trimr(args: &[Value]) -> ValueResult<Value> {
    let s = get_str(&args[0])?;
    Ok(make_str(s.trim_end().to_string()))
}

fn trim_newline(args: &[Value]) -> ValueResult<Value> {
    let s = get_str(&args[0])?;
    let result = s.trim_end_matches(['\n', '\r']);
    Ok(make_str(result.to_string()))
}

/// Java-compatible `Character.isWhitespace` check.
fn is_java_whitespace(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t'
            | '\n'
            | '\r'
            | '\x0B'
            | '\x0C'
            | '\u{1C}'
            | '\u{1D}'
            | '\u{1E}'
            | '\u{1F}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

fn blank_q(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Nil => Ok(Value::Bool(true)),
        Value::Str(s) => Ok(Value::Bool(s.get().chars().all(is_java_whitespace))),
        other => Err(ValueError::WrongType {
            expected: "string or nil",
            got: other.type_name().to_string(),
        }),
    }
}

fn starts_with_q(args: &[Value]) -> ValueResult<Value> {
    let s = get_str(&args[0])?;
    let prefix = get_strict_str(&args[1])?;
    Ok(Value::Bool(s.starts_with(prefix.as_ref())))
}

fn ends_with_q(args: &[Value]) -> ValueResult<Value> {
    let s = get_str(&args[0])?;
    let suffix = get_strict_str(&args[1])?;
    Ok(Value::Bool(s.ends_with(suffix.as_ref())))
}

fn includes_q(args: &[Value]) -> ValueResult<Value> {
    let s = get_str(&args[0])?;
    let needle = get_str(&args[1])?;
    Ok(Value::Bool(s.contains(needle.as_ref())))
}

fn replace(args: &[Value]) -> ValueResult<Value> {
    let s = get_str(&args[0])?;
    let from = get_str(&args[1])?;
    let to = get_str(&args[2])?;
    Ok(make_str(s.replace(from.as_ref(), to.as_ref())))
}

fn replace_first(args: &[Value]) -> ValueResult<Value> {
    let s = get_str(&args[0])?;
    let from = get_str(&args[1])?;
    let to = get_str(&args[2])?;
    Ok(make_str(s.replacen(from.as_ref(), to.as_ref(), 1)))
}

/// `(split s delim)` or `(split s delim limit)`.
fn split(args: &[Value]) -> ValueResult<Value> {
    let s = get_str(&args[0])?;
    let delim = get_str(&args[1])?;
    let limit = if args.len() >= 3 {
        match &args[2] {
            Value::Long(n) => Some(*n as usize),
            other => {
                return Err(ValueError::WrongType {
                    expected: "integer limit",
                    got: other.type_name().to_string(),
                });
            }
        }
    } else {
        None
    };

    let parts: Vec<Value> = match limit {
        Some(n) => s
            .splitn(n, delim.as_ref())
            .map(|p| make_str(p.to_string()))
            .collect(),
        None => s
            .split(delim.as_ref())
            .map(|p| make_str(p.to_string()))
            .collect(),
    };
    Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter(
        parts,
    ))))
}

fn split_lines(args: &[Value]) -> ValueResult<Value> {
    let s = get_str(&args[0])?;
    let parts: Vec<Value> = s.lines().map(|l| make_str(l.to_string())).collect();
    Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter(
        parts,
    ))))
}

/// `(join coll)` or `(join sep coll)`.
fn join(args: &[Value]) -> ValueResult<Value> {
    let (sep, coll) = if args.len() == 1 {
        ("".to_string(), &args[0])
    } else {
        let s = get_str(&args[0])?;
        (s.to_string(), &args[1])
    };
    let items = cljx_eval::destructure::value_to_seq_vec(coll);
    let result = items
        .iter()
        .map(|v| match v {
            Value::Str(s) => s.get().clone(),
            other => format!("{other}"),
        })
        .collect::<Vec<_>>()
        .join(&sep);
    Ok(make_str(result))
}

/// `(index-of s substr)` or `(index-of s substr from)`.
fn index_of(args: &[Value]) -> ValueResult<Value> {
    let s = get_str(&args[0])?;
    let needle = get_str(&args[1])?;
    let from = if args.len() >= 3 {
        match &args[2] {
            Value::Long(n) => *n as usize,
            _ => 0,
        }
    } else {
        0
    };
    let haystack = if from == 0 {
        s.as_ref()
    } else {
        &s[from.min(s.len())..]
    };
    match haystack.find(needle.as_ref()) {
        Some(idx) => Ok(Value::Long((idx + from) as i64)),
        None => Ok(Value::Nil),
    }
}

/// `(last-index-of s substr)` or `(last-index-of s substr from)`.
fn last_index_of(args: &[Value]) -> ValueResult<Value> {
    let s = get_str(&args[0])?;
    let needle = get_str(&args[1])?;
    let haystack = if args.len() >= 3 {
        match &args[2] {
            Value::Long(n) => &s[..(*n as usize).min(s.len())],
            _ => s.as_ref(),
        }
    } else {
        s.as_ref()
    };
    match haystack.rfind(needle.as_ref()) {
        Some(idx) => Ok(Value::Long(idx as i64)),
        None => Ok(Value::Nil),
    }
}

fn string_reverse(args: &[Value]) -> ValueResult<Value> {
    let s = get_strict_str(&args[0])?;
    Ok(make_str(s.chars().rev().collect()))
}

/// `(escape s cmap)` — replace characters in s according to cmap (a map of
/// char → replacement-string).
fn string_escape(args: &[Value]) -> ValueResult<Value> {
    let s = get_strict_str(&args[0])?;
    let cmap = &args[1];
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        let key = Value::Char(ch);
        let replacement = match cmap {
            Value::Map(m) => m.get(&key),
            _ => None,
        };
        match replacement {
            Some(v) => match v {
                Value::Str(rs) => result.push_str(rs.get()),
                other => result.push_str(&format!("{other}")),
            },
            None => result.push(ch),
        }
    }
    Ok(make_str(result))
}
