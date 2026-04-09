use cljrs_gc::GcPtr;
use cljrs_value::{Value, ValueError, ValueResult};
use cljrs_value::error::ExceptionInfo;

// Work-alike for 'new' for a limited set of "classes"
pub(crate) fn builtin_new(args: &[Value]) -> ValueResult<Value> {
    if let Value::Symbol(symbol) = &args[0] {
        match symbol.get().full_name().as_str() {
            "Exception" | "java.lang.Exception" => builtin_exception_dot(&args[1..]),
            _ => Err(ValueError::Other(format!("unknown type {}", symbol.get().full_name())))
        }
    } else {
        Err(ValueError::WrongType {
            expected: "symbol",
            got: args[0].type_name().to_string(),
        })
    }
}

pub(crate) fn builtin_exception_dot(args: &[Value]) -> ValueResult<Value> {
    if args.len() == 0 {
        Ok(Value::Error(GcPtr::new(ExceptionInfo::new(ValueError::Other("".to_string()), "".to_string(), None, None))))
    } else if args.len() == 1 {
        match &args[0] {
            Value::Str(s) => Ok(Value::Error(GcPtr::new(ExceptionInfo::new(ValueError::Other(s.get().to_string()), s.get().to_string(), None, None)))),
            Value::Error(e) =>
                Ok(Value::Error(GcPtr::new(ExceptionInfo::new(ValueError::Other("".to_string()), "".to_string(), None, Some(e.clone()))))),
            v => Err(ValueError::WrongType {
                expected: "str or error",
                got: v.type_name().to_string(),
            })
        }
    } else {
        let message = match &args[0] {
            Value::Str(s) => s.get().to_string(),
            v => return Err(ValueError::WrongType {
                expected: "str",
                got: v.type_name().to_string()
            })
        };
        let cause = match &args[1] {
            Value::Error(e) => Some(e.clone()),
            v => return Err(ValueError::WrongType {
                expected: "error",
                got: v.type_name().to_string()
            })
        };
        Ok(Value::Error(GcPtr::new(ExceptionInfo::new(ValueError::Other(message.to_string()), message.to_string(), None, cause))))
    }
}