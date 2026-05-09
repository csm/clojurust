use cljrs_value::{Value, ValueError, ValueResult};
use num_traits::ToPrimitive;
use std::time::SystemTime;

pub(crate) fn builtin_nanotime(_args: &[Value]) -> ValueResult<Value> {
    match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
        Ok(nanos) => Ok(Value::Long(
            nanos
                .as_nanos()
                .to_i64()
                .ok_or_else(|| ValueError::OutOfRange)?,
        )),
        Err(e) => Err(ValueError::Other(format!("{}", e))),
    }
}
