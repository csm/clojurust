use crate::util::numeric_as_i64;
use cljrs_value::{Value, ValueResult};

pub fn builtin_bit_and_not(args: &[Value]) -> ValueResult<Value> {
    let mut result = numeric_as_i64(&args[0])?;
    for arg in &args[1..] {
        let arg = numeric_as_i64(arg)?;
        result &= !arg;
    }
    Ok(Value::Long(result))
}

pub fn builtin_bit_clear(args: &[Value]) -> ValueResult<Value> {
    let input = numeric_as_i64(&args[0])?;
    let idx = numeric_as_i64(&args[1])?;
    let result = input & !(1 << idx);
    Ok(Value::Long(result))
}

pub fn builtin_bit_flip(args: &[Value]) -> ValueResult<Value> {
    let input = numeric_as_i64(&args[0])?;
    let idx = numeric_as_i64(&args[1])?;
    let result = input ^ (1 << idx);
    Ok(Value::Long(result))
}

pub fn builtin_bit_set(args: &[Value]) -> ValueResult<Value> {
    let input = numeric_as_i64(&args[0])?;
    let idx = numeric_as_i64(&args[1])?;
    let result = input | (1 << idx);
    Ok(Value::Long(result))
}

pub fn builtin_bit_test(args: &[Value]) -> ValueResult<Value> {
    let input = numeric_as_i64(&args[0])?;
    let idx = numeric_as_i64(&args[1])?;
    let result = input & (1 << idx);
    Ok(Value::Bool(result != 0))
}
