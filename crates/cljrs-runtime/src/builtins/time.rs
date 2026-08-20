use cljrs_value::{Value, ValueError, ValueResult};
use num_traits::ToPrimitive;
use std::sync::OnceLock;
use std::time::{Instant, SystemTime};

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

/// Milliseconds since the Unix epoch (`System/currentTimeMillis`).
pub(crate) fn builtin_current_time_millis(_args: &[Value]) -> ValueResult<Value> {
    match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
        Ok(since_epoch) => Ok(Value::Long(
            since_epoch
                .as_millis()
                .to_i64()
                .ok_or_else(|| ValueError::OutOfRange)?,
        )),
        Err(e) => Err(ValueError::Other(format!("{}", e))),
    }
}

/// Process start, the origin `system_nano_time` counts from.
fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// Nanoseconds from an arbitrary fixed origin (`System/nanoTime`).
///
/// Monotonic and unaffected by wall-clock adjustments, so differences of two
/// readings measure elapsed time. Only differences are meaningful.
pub(crate) fn builtin_system_nano_time(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Long(
        epoch()
            .elapsed()
            .as_nanos()
            .to_i64()
            .ok_or_else(|| ValueError::OutOfRange)?,
    ))
}

/// Force the origin to be taken at startup rather than at the first reading.
pub fn init_clock() {
    let _ = epoch();
}
