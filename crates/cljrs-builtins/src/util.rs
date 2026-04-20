// Utility functions.

use bigdecimal::BigDecimal;
use cljrs_env::error::{EvalError, EvalResult};
use cljrs_gc::GcPtr;
use cljrs_value::{Value, ValueError, ValueResult};
use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive};
use std::ops::{Div, Mul};

pub fn bigdec_to_i64(d: &BigDecimal) -> ValueResult<i64> {
    let (num, exp) = d.as_bigint_and_exponent();
    let res = if exp >= 0 {
        let pow = BigInt::from(10).pow(exp as u32);
        num.div(pow)
    } else {
        let scale = BigInt::from(10).pow((-exp) as u32);
        num.mul(scale)
    };
    res.to_i64()
        .ok_or_else(|| ValueError::Other("BigDecimal too large for i64".into()))
}

pub fn numeric_as_i64(v: &Value) -> ValueResult<i64> {
    match v {
        Value::Long(n) => Ok(*n),
        Value::Double(f) => {
            if f64::is_infinite(*f) || f64::is_nan(*f) {
                Err(ValueError::Other(
                    "cannot convert non-number to i64".to_string(),
                ))
            } else {
                Ok(*f as i64)
            }
        }
        Value::Char(c) => Ok(*c as i64),
        Value::BigInt(n) => n
            .get()
            .to_i64()
            .ok_or_else(|| ValueError::Other("BigInt too large for i64".into())),
        Value::Ratio(r) => {
            let trunc = if r.get().is_negative() {
                // Use ceiling to truncate towards zero.
                r.get().ceil()
            } else {
                r.get().floor()
            };
            trunc
                .to_i64()
                .ok_or_else(|| ValueError::Other("cannot convert ratio".into()))
        }
        Value::BigDecimal(d) => bigdec_to_i64(d.get()),
        Value::Bool(b) => Ok(*b as i64),
        Value::Str(s) => match s.get().parse::<BigDecimal>() {
            Ok(d) => bigdec_to_i64(&d),
            Err(_) => Err(ValueError::Other(
                "failed to parse string as number".to_string(),
            )),
        },
        _ => Err(ValueError::WrongType {
            expected: "integer",
            got: v.type_name().to_string(),
        }),
    }
}

// ── Numeric parsing ───────────────────────────────────────────────────────────

pub fn parse_bigint(s: &str) -> EvalResult {
    let s = s.trim_end_matches('N');
    s.parse::<num_bigint::BigInt>()
        .map(|n| Value::BigInt(GcPtr::new(n)))
        .map_err(|e| EvalError::Runtime(format!("bad bigint: {e}")))
}

pub fn parse_bigdecimal(s: &str) -> EvalResult {
    let s = s.trim_end_matches('M');
    s.parse::<bigdecimal::BigDecimal>()
        .map(|d| Value::BigDecimal(GcPtr::new(d)))
        .map_err(|e| EvalError::Runtime(format!("bad bigdecimal: {e}")))
}

pub fn parse_ratio(s: &str) -> EvalResult {
    use num_traits::{ToPrimitive, Zero};
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 2 {
        return Err(EvalError::Runtime(format!("bad ratio: {s}")));
    }
    let numer: num_bigint::BigInt = parts[0]
        .parse()
        .map_err(|e| EvalError::Runtime(format!("bad ratio numer: {e}")))?;
    let denom: num_bigint::BigInt = parts[1]
        .parse()
        .map_err(|e| EvalError::Runtime(format!("bad ratio denom: {e}")))?;
    if denom.is_zero() {
        return Err(EvalError::Runtime("ratio denominator is zero".into()));
    }
    let r = num_rational::Ratio::new(numer, denom);
    if r.is_integer() {
        let n = r.to_integer();
        match n.to_i64() {
            Some(i) => Ok(Value::Long(i)),
            None => Ok(Value::BigInt(GcPtr::new(n))),
        }
    } else {
        Ok(Value::Ratio(GcPtr::new(r)))
    }
}
