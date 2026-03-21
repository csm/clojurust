//! Convert Clojure Value data (maps/vectors/keywords) → Rust IR types.
//!
//! This module is the boundary contract between the Clojure-based compiler
//! front-end and the Rust-based codegen backend. It reads IR data structures
//! represented as plain Clojure data and produces the `IrFunction`, `Block`,
//! `Inst`, and `Terminator` types that `codegen.rs` consumes.

use std::sync::Arc;

use cljrs_types::span::Span;
use cljrs_value::{Keyword, MapValue, Value};

use crate::ir::*;

// ── Error type ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ConvertError {
    /// A required field is missing from the map.
    MissingField(String),
    /// A field has the wrong type.
    TypeError(String),
    /// An unknown keyword was encountered.
    UnknownVariant(String),
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConvertError::MissingField(s) => write!(f, "missing field: {s}"),
            ConvertError::TypeError(s) => write!(f, "type error: {s}"),
            ConvertError::UnknownVariant(s) => write!(f, "unknown variant: {s}"),
        }
    }
}

impl std::error::Error for ConvertError {}

type ConvertResult<T> = Result<T, ConvertError>;

// ── Map field extraction helpers ────────────────────────────────────────────

fn get_field(map: &MapValue, key: &str) -> ConvertResult<Value> {
    let kw = Value::keyword(Keyword::simple(key));
    map.get(&kw)
        .filter(|v| !matches!(v, Value::Nil))
        .ok_or_else(|| ConvertError::MissingField(key.to_string()))
}

fn get_field_opt(map: &MapValue, key: &str) -> Option<Value> {
    let kw = Value::keyword(Keyword::simple(key));
    map.get(&kw).filter(|v| !matches!(v, Value::Nil))
}

fn as_map(val: &Value) -> ConvertResult<&MapValue> {
    match val {
        Value::Map(m) => Ok(m),
        other => Err(ConvertError::TypeError(format!(
            "expected map, got {}",
            other.type_name()
        ))),
    }
}

fn as_long(val: &Value) -> ConvertResult<i64> {
    match val {
        Value::Long(n) => Ok(*n),
        other => Err(ConvertError::TypeError(format!(
            "expected long, got {}",
            other.type_name()
        ))),
    }
}

fn as_u32(val: &Value) -> ConvertResult<u32> {
    as_long(val).map(|n| n as u32)
}

fn as_str(val: &Value) -> ConvertResult<Arc<str>> {
    match val {
        Value::Str(s) => Ok(Arc::from(s.get().as_str())),
        other => Err(ConvertError::TypeError(format!(
            "expected string, got {}",
            other.type_name()
        ))),
    }
}

fn as_keyword_name(val: &Value) -> ConvertResult<Arc<str>> {
    match val {
        Value::Keyword(kw) => Ok(Arc::clone(&kw.get().name)),
        other => Err(ConvertError::TypeError(format!(
            "expected keyword, got {}",
            other.type_name()
        ))),
    }
}

fn as_vec(val: &Value) -> ConvertResult<Vec<Value>> {
    match val {
        Value::Vector(v) => {
            let pv = v.get();
            let mut result = Vec::with_capacity(pv.count());
            for i in 0..pv.count() {
                if let Some(item) = pv.nth(i) {
                    result.push(item.clone());
                }
            }
            Ok(result)
        }
        Value::List(l) => {
            let mut result = Vec::new();
            let mut cur = l.get().clone();
            while let Some(v) = cur.first() {
                result.push(v.clone());
                let rest = cur.rest();
                cur = (*rest).clone();
            }
            Ok(result)
        }
        Value::Nil => Ok(vec![]),
        other => Err(ConvertError::TypeError(format!(
            "expected vector/list, got {}",
            other.type_name()
        ))),
    }
}

fn as_var_id(val: &Value) -> ConvertResult<VarId> {
    as_u32(val).map(VarId)
}

fn as_block_id(val: &Value) -> ConvertResult<BlockId> {
    as_u32(val).map(BlockId)
}

// ── Top-level conversion ────────────────────────────────────────────────────

/// Convert a Clojure Value (map) → `IrFunction`.
pub fn value_to_ir_function(val: &Value) -> ConvertResult<IrFunction> {
    let map = as_map(val)?;

    let name = get_field_opt(map, "name").map(|v| as_str(&v)).transpose()?;
    let next_var = as_u32(&get_field(map, "next-var")?)?;
    let next_block = as_u32(&get_field(map, "next-block")?)?;

    // Parse params: vector of [name var-id] pairs
    let params_val = get_field(map, "params")?;
    let params_vec = as_vec(&params_val)?;
    let mut params = Vec::with_capacity(params_vec.len());
    for p in &params_vec {
        let pair = as_vec(p)?;
        if pair.len() != 2 {
            return Err(ConvertError::TypeError(
                "param must be [name var-id]".into(),
            ));
        }
        let name = as_str(&pair[0])?;
        let var_id = as_var_id(&pair[1])?;
        params.push((name, var_id));
    }

    // Parse blocks
    let blocks_val = get_field(map, "blocks")?;
    let blocks_vec = as_vec(&blocks_val)?;
    let mut blocks = Vec::with_capacity(blocks_vec.len());
    for b in &blocks_vec {
        blocks.push(value_to_block(b)?);
    }

    // Parse subfunctions (optional — may not be present)
    let subfunctions = if let Some(subs_val) = get_field_opt(map, "subfunctions") {
        let subs_vec = as_vec(&subs_val)?;
        let mut subs = Vec::with_capacity(subs_vec.len());
        for s in &subs_vec {
            subs.push(value_to_ir_function(s)?);
        }
        subs
    } else {
        vec![]
    };

    Ok(IrFunction {
        name,
        params,
        blocks,
        next_var,
        next_block,
        span: None,
        subfunctions,
    })
}

/// Convert a Clojure Value (map) → `Block`.
fn value_to_block(val: &Value) -> ConvertResult<Block> {
    let map = as_map(val)?;

    let id = as_block_id(&get_field(map, "id")?)?;

    let phis_val = get_field(map, "phis")?;
    let phis_vec = as_vec(&phis_val)?;
    let mut phis = Vec::with_capacity(phis_vec.len());
    for p in &phis_vec {
        phis.push(value_to_inst(p)?);
    }

    let insts_val = get_field(map, "insts")?;
    let insts_vec = as_vec(&insts_val)?;
    let mut insts = Vec::with_capacity(insts_vec.len());
    for i in &insts_vec {
        insts.push(value_to_inst(i)?);
    }

    let term_val = get_field(map, "terminator")?;
    let terminator = value_to_terminator(&term_val)?;

    Ok(Block {
        id,
        phis,
        insts,
        terminator,
    })
}

/// Convert a Clojure Value (map) → `Inst`.
fn value_to_inst(val: &Value) -> ConvertResult<Inst> {
    let map = as_map(val)?;
    let op = as_keyword_name(&get_field(map, "op")?)?;

    match op.as_ref() {
        "const" => {
            let dst = as_var_id(&get_field(map, "dst")?)?;
            let const_val = get_field(map, "value")?;
            let c = value_to_const(&const_val)?;
            Ok(Inst::Const(dst, c))
        }
        "load-local" => {
            let dst = as_var_id(&get_field(map, "dst")?)?;
            let name = as_str(&get_field(map, "name")?)?;
            Ok(Inst::LoadLocal(dst, name))
        }
        "load-global" => {
            let dst = as_var_id(&get_field(map, "dst")?)?;
            let ns = as_str(&get_field(map, "ns")?)?;
            let name = as_str(&get_field(map, "name")?)?;
            Ok(Inst::LoadGlobal(dst, ns, name))
        }
        "alloc-vector" => {
            let dst = as_var_id(&get_field(map, "dst")?)?;
            let elems = as_var_id_vec(&get_field(map, "elems")?)?;
            Ok(Inst::AllocVector(dst, elems))
        }
        "alloc-map" => {
            let dst = as_var_id(&get_field(map, "dst")?)?;
            let pairs_val = get_field(map, "pairs")?;
            let pairs_vec = as_vec(&pairs_val)?;
            let mut pairs = Vec::with_capacity(pairs_vec.len());
            for p in &pairs_vec {
                let pair = as_vec(p)?;
                if pair.len() != 2 {
                    return Err(ConvertError::TypeError("map pair must be [k v]".into()));
                }
                pairs.push((as_var_id(&pair[0])?, as_var_id(&pair[1])?));
            }
            Ok(Inst::AllocMap(dst, pairs))
        }
        "alloc-set" => {
            let dst = as_var_id(&get_field(map, "dst")?)?;
            let elems = as_var_id_vec(&get_field(map, "elems")?)?;
            Ok(Inst::AllocSet(dst, elems))
        }
        "alloc-list" => {
            let dst = as_var_id(&get_field(map, "dst")?)?;
            let elems = as_var_id_vec(&get_field(map, "elems")?)?;
            Ok(Inst::AllocList(dst, elems))
        }
        "alloc-cons" => {
            let dst = as_var_id(&get_field(map, "dst")?)?;
            let head = as_var_id(&get_field(map, "head")?)?;
            let tail = as_var_id(&get_field(map, "tail")?)?;
            Ok(Inst::AllocCons(dst, head, tail))
        }
        "alloc-closure" => {
            let dst = as_var_id(&get_field(map, "dst")?)?;
            let captures = as_var_id_vec(&get_field(map, "captures")?)?;
            let name = get_field_opt(map, "closure-name")
                .map(|v| as_str(&v))
                .transpose()?;
            // Parse arity function names
            let arity_fn_names = if let Some(v) = get_field_opt(map, "arity-fn-names") {
                as_vec(&v)?
                    .iter()
                    .map(as_str)
                    .collect::<ConvertResult<Vec<_>>>()?
            } else {
                vec![]
            };
            // Parse parameter counts
            let param_counts = if let Some(v) = get_field_opt(map, "param-counts") {
                as_vec(&v)?
                    .iter()
                    .map(|n| as_long(n).map(|i| i as usize))
                    .collect::<ConvertResult<Vec<_>>>()?
            } else {
                vec![]
            };
            // Parse capture names
            let capture_names = if let Some(v) = get_field_opt(map, "capture-names") {
                as_vec(&v)?
                    .iter()
                    .map(as_str)
                    .collect::<ConvertResult<Vec<_>>>()?
            } else {
                vec![]
            };
            let tmpl = ClosureTemplate {
                name,
                arity_fn_names,
                param_counts,
                capture_names,
            };
            Ok(Inst::AllocClosure(dst, tmpl, captures))
        }
        "call-known" => {
            let dst = as_var_id(&get_field(map, "dst")?)?;
            let func_kw = as_keyword_name(&get_field(map, "func")?)?;
            let known = keyword_to_known_fn(&func_kw).ok_or_else(|| {
                ConvertError::UnknownVariant(format!("unknown KnownFn: {func_kw}"))
            })?;
            let args = as_var_id_vec(&get_field(map, "args")?)?;
            Ok(Inst::CallKnown(dst, known, args))
        }
        "call" => {
            let dst = as_var_id(&get_field(map, "dst")?)?;
            let callee = as_var_id(&get_field(map, "callee")?)?;
            let args = as_var_id_vec(&get_field(map, "args")?)?;
            Ok(Inst::Call(dst, callee, args))
        }
        "deref" => {
            let dst = as_var_id(&get_field(map, "dst")?)?;
            let src = as_var_id(&get_field(map, "src")?)?;
            Ok(Inst::Deref(dst, src))
        }
        "def-var" => {
            let dst = as_var_id(&get_field(map, "dst")?)?;
            let ns = as_str(&get_field(map, "ns")?)?;
            let name = as_str(&get_field(map, "name")?)?;
            let value = as_var_id(&get_field(map, "value")?)?;
            Ok(Inst::DefVar(dst, ns, name, value))
        }
        "set!" => {
            let var = as_var_id(&get_field(map, "var")?)?;
            let value = as_var_id(&get_field(map, "value")?)?;
            Ok(Inst::SetBang(var, value))
        }
        "throw" => {
            let val = as_var_id(&get_field(map, "value")?)?;
            Ok(Inst::Throw(val))
        }
        "phi" => {
            let dst = as_var_id(&get_field(map, "dst")?)?;
            let entries_val = get_field(map, "entries")?;
            let entries_vec = as_vec(&entries_val)?;
            let mut entries = Vec::with_capacity(entries_vec.len());
            for e in &entries_vec {
                let pair = as_vec(e)?;
                if pair.len() != 2 {
                    return Err(ConvertError::TypeError(
                        "phi entry must be [block-id var-id]".into(),
                    ));
                }
                entries.push((as_block_id(&pair[0])?, as_var_id(&pair[1])?));
            }
            Ok(Inst::Phi(dst, entries))
        }
        "recur" => {
            let args = as_var_id_vec(&get_field(map, "args")?)?;
            Ok(Inst::Recur(args))
        }
        "source-loc" => {
            let file = as_str(&get_field(map, "file")?)?;
            let line = as_u32(&get_field(map, "line")?)?;
            let col = as_u32(&get_field(map, "col")?)?;
            Ok(Inst::SourceLoc(Span::new(
                Arc::new(file.to_string()),
                0,
                0,
                line,
                col,
            )))
        }
        other => Err(ConvertError::UnknownVariant(format!(
            "unknown inst op: {other}"
        ))),
    }
}

/// Convert a Clojure Value (map) → `Terminator`.
fn value_to_terminator(val: &Value) -> ConvertResult<Terminator> {
    let map = as_map(val)?;
    let op = as_keyword_name(&get_field(map, "op")?)?;

    match op.as_ref() {
        "jump" => {
            let target = as_block_id(&get_field(map, "target")?)?;
            Ok(Terminator::Jump(target))
        }
        "branch" => {
            let cond = as_var_id(&get_field(map, "cond")?)?;
            let then_block = as_block_id(&get_field(map, "then-block")?)?;
            let else_block = as_block_id(&get_field(map, "else-block")?)?;
            Ok(Terminator::Branch {
                cond,
                then_block,
                else_block,
            })
        }
        "return" => {
            let var = as_var_id(&get_field(map, "var")?)?;
            Ok(Terminator::Return(var))
        }
        "recur-jump" => {
            let target = as_block_id(&get_field(map, "target")?)?;
            let args = as_var_id_vec(&get_field(map, "args")?)?;
            Ok(Terminator::RecurJump { target, args })
        }
        "unreachable" => Ok(Terminator::Unreachable),
        other => Err(ConvertError::UnknownVariant(format!(
            "unknown terminator op: {other}"
        ))),
    }
}

/// Convert a Clojure Value (map) → `Const`.
fn value_to_const(val: &Value) -> ConvertResult<Const> {
    let map = as_map(val)?;
    let ty = as_keyword_name(&get_field(map, "type")?)?;

    match ty.as_ref() {
        "nil" => Ok(Const::Nil),
        "bool" => {
            let v = get_field(map, "val")?;
            match v {
                Value::Bool(b) => Ok(Const::Bool(b)),
                _ => Err(ConvertError::TypeError(
                    "expected bool for :bool const".into(),
                )),
            }
        }
        "long" => {
            let v = as_long(&get_field(map, "val")?)?;
            Ok(Const::Long(v))
        }
        "double" => {
            let v = get_field(map, "val")?;
            match v {
                Value::Double(d) => Ok(Const::Double(d)),
                Value::Long(n) => Ok(Const::Double(n as f64)),
                _ => Err(ConvertError::TypeError(
                    "expected double for :double const".into(),
                )),
            }
        }
        "string" => {
            let v = as_str(&get_field(map, "val")?)?;
            Ok(Const::Str(v))
        }
        "keyword" => {
            let v = as_str(&get_field(map, "val")?)?;
            Ok(Const::Keyword(v))
        }
        "symbol" => {
            let v = as_str(&get_field(map, "val")?)?;
            Ok(Const::Symbol(v))
        }
        "char" => {
            let v = get_field(map, "val")?;
            match v {
                Value::Char(c) => Ok(Const::Char(c)),
                _ => Err(ConvertError::TypeError(
                    "expected char for :char const".into(),
                )),
            }
        }
        other => Err(ConvertError::UnknownVariant(format!(
            "unknown const type: {other}"
        ))),
    }
}

// ── Known function mapping ──────────────────────────────────────────────────

/// Map a keyword name to a `KnownFn` variant.
pub fn keyword_to_known_fn(kw: &str) -> Option<KnownFn> {
    match kw {
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
        "contains" => Some(KnownFn::Contains),
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
        "+" => Some(KnownFn::Add),
        "-" => Some(KnownFn::Sub),
        "*" => Some(KnownFn::Mul),
        "/" => Some(KnownFn::Div),
        "rem" => Some(KnownFn::Rem),
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
        "atom-deref" => Some(KnownFn::AtomDeref),
        "atom-reset" => Some(KnownFn::AtomReset),
        "atom-swap" => Some(KnownFn::AtomSwap),
        "apply" => Some(KnownFn::Apply),
        "try-catch-finally" => Some(KnownFn::TryCatchFinally),
        _ => None,
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn as_var_id_vec(val: &Value) -> ConvertResult<Vec<VarId>> {
    let vec = as_vec(val)?;
    vec.iter().map(as_var_id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cljrs_value::Keyword;

    fn kw(s: &str) -> Value {
        Value::keyword(Keyword::simple(s))
    }

    fn make_map(pairs: Vec<(Value, Value)>) -> Value {
        let mut m = MapValue::empty();
        for (k, v) in pairs {
            m = m.assoc(k, v);
        }
        Value::Map(m)
    }

    fn make_vec(items: Vec<Value>) -> Value {
        use cljrs_gc::GcPtr;
        use cljrs_value::collections::vector::PersistentVector;
        Value::Vector(GcPtr::new(PersistentVector::from_iter(items)))
    }

    #[test]
    fn test_const_nil() {
        let val = make_map(vec![(kw("type"), kw("nil"))]);
        let c = value_to_const(&val).unwrap();
        assert!(matches!(c, Const::Nil));
    }

    #[test]
    fn test_const_long() {
        let val = make_map(vec![(kw("type"), kw("long")), (kw("val"), Value::Long(42))]);
        let c = value_to_const(&val).unwrap();
        assert!(matches!(c, Const::Long(42)));
    }

    #[test]
    fn test_terminator_return() {
        let val = make_map(vec![(kw("op"), kw("return")), (kw("var"), Value::Long(0))]);
        let t = value_to_terminator(&val).unwrap();
        assert!(matches!(t, Terminator::Return(VarId(0))));
    }

    #[test]
    fn test_inst_const() {
        let const_map = make_map(vec![(kw("type"), kw("long")), (kw("val"), Value::Long(42))]);
        let val = make_map(vec![
            (kw("op"), kw("const")),
            (kw("dst"), Value::Long(0)),
            (kw("value"), const_map),
        ]);
        let inst = value_to_inst(&val).unwrap();
        assert!(matches!(inst, Inst::Const(VarId(0), Const::Long(42))));
    }

    #[test]
    fn test_simple_ir_function() {
        // Build: {:name "test" :params [["x" 0]] :blocks [...] :next-var 2 :next-block 1}
        let const_val = make_map(vec![(kw("type"), kw("long")), (kw("val"), Value::Long(42))]);
        let inst = make_map(vec![
            (kw("op"), kw("const")),
            (kw("dst"), Value::Long(1)),
            (kw("value"), const_val),
        ]);
        let terminator = make_map(vec![(kw("op"), kw("return")), (kw("var"), Value::Long(1))]);
        let block = make_map(vec![
            (kw("id"), Value::Long(0)),
            (kw("phis"), make_vec(vec![])),
            (kw("insts"), make_vec(vec![inst])),
            (kw("terminator"), terminator),
        ]);
        let param = make_vec(vec![Value::string("x".to_string()), Value::Long(0)]);
        let ir_val = make_map(vec![
            (kw("name"), Value::string("test".to_string())),
            (kw("params"), make_vec(vec![param])),
            (kw("blocks"), make_vec(vec![block])),
            (kw("next-var"), Value::Long(2)),
            (kw("next-block"), Value::Long(1)),
        ]);

        let ir = value_to_ir_function(&ir_val).unwrap();
        assert_eq!(ir.name.as_deref(), Some("test"));
        assert_eq!(ir.params.len(), 1);
        assert_eq!(ir.params[0].0.as_ref(), "x");
        assert_eq!(ir.blocks.len(), 1);
        assert_eq!(ir.next_var, 2);
        assert_eq!(ir.next_block, 1);
    }
}
