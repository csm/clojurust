/// Transient collections.
use crate::util::numeric_as_i64;
use cljrs_gc::GcPtr;
use cljrs_value::collections::{TransientMap, TransientSet, TransientVector};
use cljrs_value::value::SetValue;
use cljrs_value::{MapValue, Value, ValueError, ValueResult};

pub fn builtin_transient(args: &[Value]) -> ValueResult<Value> {
    match &args[0].unwrap_meta() {
        Value::Map(MapValue::Array(m)) => {
            let map = TransientMap::new();
            for (k, v) in m.get().iter() {
                map.assoc(k.clone(), v.clone())?;
            }
            Ok(Value::TransientMap(GcPtr::new(map)))
        }
        Value::Map(MapValue::Hash(m)) => Ok(Value::TransientMap(GcPtr::new(
            TransientMap::new_from_map(m.get().inner()),
        ))),
        Value::Set(SetValue::Hash(s)) => Ok(Value::TransientSet(GcPtr::new(
            TransientSet::new_from_set(s.get().inner()),
        ))),
        Value::Vector(v) => Ok(Value::TransientVector(GcPtr::new(
            TransientVector::new_from_vector(v.get().inner()),
        ))),
        Value::TransientMap(_) | Value::TransientVector(_) | Value::TransientSet(_) => {
            Ok(args[0].clone())
        }
        v => Err(ValueError::WrongType {
            expected: "editable",
            got: v.type_name().to_string(),
        }),
    }
}

pub fn builtin_persistent_bang(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::TransientMap(m) => Ok(Value::Map(MapValue::Hash(GcPtr::new(
            m.get().persistent()?,
        )))),
        Value::TransientSet(s) => Ok(Value::Set(SetValue::Hash(GcPtr::new(
            s.get().persistent()?,
        )))),
        Value::TransientVector(v) => Ok(Value::Vector(GcPtr::new(v.get().persistent()?))),
        v => Err(ValueError::WrongType {
            expected: "transient",
            got: v.type_name().to_string(),
        }),
    }
}

pub fn builtin_assoc_bang(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::TransientMap(m) => {
            for kv in args[1..].windows(2).step_by(2) {
                let k = &kv[0];
                let v = &kv[1];
                m.get().assoc(k.clone(), v.clone())?;
            }
            // Odd number of args, assoc nil with the final key.
            if args.len().is_multiple_of(2) {
                let k = &args[args.len() - 1];
                m.get().assoc(k.clone(), Value::Nil)?;
            }
            Ok(args[0].clone())
        }
        Value::TransientVector(v) => {
            for kv in args[1..].windows(2).step_by(2) {
                let k = numeric_as_i64(&kv[0])? as usize;
                let val = &kv[1];
                if k < v.get().count() {
                    v.get().set(k, val.clone())?;
                } else if k == v.get().count() {
                    v.get().append(val.clone())?;
                } else {
                    return Err(ValueError::IndexOutOfBounds {
                        idx: k,
                        count: v.get().count(),
                    });
                }
            }
            if args.len().is_multiple_of(2) {
                let k = numeric_as_i64(&args[args.len() - 1])? as usize;
                if k < v.get().count() {
                    v.get().set(k, Value::Nil)?;
                } else if k == v.get().count() {
                    v.get().append(Value::Nil)?;
                } else {
                    return Err(ValueError::IndexOutOfBounds {
                        idx: k,
                        count: v.get().count(),
                    });
                }
            }
            Ok(args[0].clone())
        }
        v => Err(ValueError::WrongType {
            expected: "transient",
            got: v.type_name().to_string(),
        }),
    }
}

pub fn builtin_conj_bang(args: &[Value]) -> ValueResult<Value> {
    if args.is_empty() {
        return Ok(Value::TransientVector(GcPtr::new(TransientVector::new())));
    }
    match &args[0] {
        Value::TransientMap(m) => {
            for item in args[1..].iter() {
                match item {
                    Value::Vector(v) if v.get().count() == 2 => {
                        let k = v.get().nth(0).unwrap();
                        let val = v.get().nth(1).unwrap();
                        m.get().assoc(k.clone(), val.clone())?;
                    }
                    Value::Map(map) => {
                        for (k, v) in map.iter() {
                            m.get().assoc(k.clone(), v.clone())?;
                        }
                    }
                    Value::Nil => {} // skip nil
                    v => {
                        return Err(ValueError::WrongType {
                            expected: "map-entry",
                            got: v.type_name().to_string(),
                        });
                    }
                }
            }
            Ok(args[0].clone())
        }
        Value::TransientVector(v) => {
            for item in args[1..].iter() {
                v.get().append(item.clone())?;
            }
            Ok(args[0].clone())
        }
        Value::TransientSet(s) => {
            for item in args[1..].iter() {
                s.get().conj(item.clone())?;
            }
            Ok(args[0].clone())
        }
        v if args.len() == 1 => Ok(v.clone()),
        v => Err(ValueError::WrongType {
            expected: "transient",
            got: v.type_name().to_string(),
        }),
    }
}

pub fn builtin_disj_bang(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::TransientSet(s) => {
            for v in args[1..].iter() {
                s.get().disj(v)?;
            }
            Ok(args[0].clone())
        }
        v => Err(ValueError::WrongType {
            expected: "transient-set",
            got: v.type_name().to_string(),
        }),
    }
}

pub fn builtin_dissoc_bang(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::TransientMap(m) => {
            for k in args[1..].iter() {
                m.get().dissoc(k)?;
            }
            Ok(args[0].clone())
        }
        v => Err(ValueError::WrongType {
            expected: "transient-map",
            got: v.type_name().to_string(),
        }),
    }
}

pub fn builtin_pop_bang(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::TransientVector(v) => {
            v.get().pop()?;
            Ok(args[0].clone())
        }
        v => Err(ValueError::WrongType {
            expected: "transient-vector",
            got: v.type_name().to_string(),
        }),
    }
}
