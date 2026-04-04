use std::any::Any;
use std::sync::Mutex;
use cljrs_gc::{GcPtr, MarkVisitor, Trace};
use cljrs_value::{NativeObject, NativeObjectBox, ObjectArray, Value, ValueError, ValueResult};
use crate::util::numeric_as_i64;

#[derive(Debug)]
struct ArrayList {
    elements: Mutex<Vec<Value>>
}

const NAME: &'static str = "ArrayList";

impl Trace for ArrayList {
    fn trace(&self, visitor: &mut MarkVisitor) {
        let elements = self.elements.lock().unwrap();
        for element in elements.iter() {
            element.trace(visitor);
        }
    }
}

impl NativeObject for ArrayList {
    fn type_tag(&self) -> &str {
        NAME
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn builtin_array_list(args: &[Value]) -> ValueResult<Value> {
    let elements: Vec<Value> = if args.len() == 0 {
        Vec::new()
    } else {
        match &args[0] {
            Value::Long(n) => Vec::with_capacity(*n as usize),
            // TODO, collection types, init array-list.
            v => return Err(ValueError::WrongType {
                expected: "long or collection",
                got: v.type_name().to_string(),
            })
        }
    };
    Ok(Value::NativeObject(GcPtr::new(NativeObjectBox::new(ArrayList { elements: Mutex::new(elements) }))))
}

pub fn builtin_array_list_push(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::NativeObject(obj) if obj.get().type_tag() == NAME => {
            if let Some(array_list) = obj.get().downcast_ref::<ArrayList>() {
                let mut elements = array_list.elements.lock().unwrap();
                elements.push(args[1].clone());
                Ok(args[0].clone())
            } else {
                Err(ValueError::WrongType {
                    expected: "array-list",
                    got: obj.get().type_tag().to_string(),
                })
            }
        }
        v => Err(ValueError::WrongType {
            expected: "array-list",
            got: v.type_name().to_string(),
        })
    }
}

pub fn builtin_array_list_length(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::NativeObject(obj) if obj.get().type_tag() == NAME => {
            if let Some(array_list) = obj.get().downcast_ref::<ArrayList>() {
                let elements = array_list.elements.lock().unwrap();
                Ok(Value::Long(elements.len() as i64))
            } else {
                Err(ValueError::WrongType {
                    expected: "array-list",
                    got: obj.get().type_tag().to_string(),
                })
            }
        }
        v => Err(ValueError::WrongType {
            expected: "array-list",
            got: v.type_name().to_string(),
        })
    }
}

pub fn builtin_array_list_remove(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::NativeObject(obj) if obj.get().type_tag() == NAME => {
            let index = numeric_as_i64(&args[1])? as usize;
            if let Some(array_list) = obj.get().downcast_ref::<ArrayList>() {
                let mut elements = array_list.elements.lock().unwrap();
                if index >= elements.len() {
                    return Err(ValueError::IndexOutOfBounds {
                        idx: index,
                        count: elements.len()
                    })
                }
                let removed = elements.remove(index);
                Ok(removed.clone())
            } else {
                Err(ValueError::WrongType {
                    expected: "array-list",
                    got: obj.get().type_tag().to_string(),
                })
            }
        }
        v => Err(ValueError::WrongType {
            expected: "array-list",
            got: v.type_name().to_string(),
        })
    }
}

pub fn builtin_array_list_to_array(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::NativeObject(obj) if obj.get().type_tag() == NAME => {
            let elements = obj.get().downcast_ref::<ArrayList>().unwrap().elements.lock().unwrap();
            Ok(Value::ObjectArray(GcPtr::new(ObjectArray(Mutex::new(elements.iter().cloned().collect())))))
        },
        v => Err(
            ValueError::WrongType {
                expected: "array-list",
                got: v.type_name().to_string(),
            }
        )
    }
}

pub fn builtin_array_list_clear(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::NativeObject(obj) if obj.get().type_tag() == NAME => {
            let mut elements = obj.get().downcast_ref::<ArrayList>().unwrap().elements.lock().unwrap();
            elements.clear();
            Ok(args[0].clone())
        }
        v => Err(ValueError::WrongType {
            expected: "array-list",
            got: v.type_name().to_string(),
        })
    }
}
