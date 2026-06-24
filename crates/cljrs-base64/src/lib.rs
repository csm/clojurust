use std::sync::{Arc, Mutex};

use base64::Engine;
use base64::alphabet::{STANDARD, URL_SAFE};
use base64::engine::general_purpose::{NO_PAD, PAD};
use cljrs_env::env::GlobalEnv;
use cljrs_gc::GcPtr;
use cljrs_interop::{Registry, wrap_fn1};
use cljrs_value::Value;

pub const NS: &str = "cljrs.base64";

/// Register the `cljrs.base64` namespace into `globals`.
///
/// Idempotent: the namespace is built only on the first call.
pub fn init(globals: &Arc<GlobalEnv>) {
    if globals.is_loaded(NS) {
        return;
    }
    globals.get_or_create_ns(NS);
    globals.refer_all(NS, "clojure.core");
    let mut registry = Registry::for_require(globals.clone());
    register(&mut registry);
}

fn to_bytes(value: Value) -> Result<Vec<u8>, String> {
    match value {
        Value::Str(s) => Ok(s.get().as_bytes().to_vec()),
        Value::Vector(vec) => {
            let mut bytes = Vec::with_capacity(vec.get().count());
            for elem in vec.get().iter() {
                match elem {
                    Value::Long(n) if (0..=255).contains(n) => bytes.push(*n as u8),
                    Value::Long(n) => return Err(format!("byte out of range 0–255: {n}")),
                    other => {
                        return Err(format!(
                            "byte vector must contain integers 0–255, got {}",
                            other.type_name()
                        ));
                    }
                }
            }
            Ok(bytes)
        }
        Value::ByteArray(b) => Ok(vec_i8_into_u8(b.get().lock().unwrap().to_vec())),
        Value::ByteBlob(b) => Ok(b.to_vec()),
        other => Err(format!(
            "expected string or byte vector, got {}",
            other.type_name()
        )),
    }
}

fn vec_u8_into_i8(v: Vec<u8>) -> Vec<i8> {
    let mut v = std::mem::ManuallyDrop::new(v);
    let ptr = v.as_mut_ptr();
    let len = v.len();
    let cap = v.capacity();
    unsafe { Vec::from_raw_parts(ptr as *mut i8, len, cap) }
}

fn vec_i8_into_u8(v: Vec<i8>) -> Vec<u8> {
    let mut v = std::mem::ManuallyDrop::new(v);
    let ptr = v.as_mut_ptr();
    let len = v.len();
    let cap = v.capacity();
    unsafe { Vec::from_raw_parts(ptr as *mut u8, len, cap) }
}

pub fn register(registry: &mut Registry) {
    registry.define(
        "cljrs.base64/encode",
        wrap_fn1(
            "cljrs.base64/encode",
            |data: Value| -> Result<String, String> {
                let bytes = to_bytes(data)?;
                let engine = base64::engine::GeneralPurpose::new(&STANDARD, PAD);
                Ok(engine.encode(&bytes))
            },
        ),
    );

    registry.define(
        "cljrs.base64/decode",
        wrap_fn1(
            "cljrs.base64/decode",
            |data: Value| -> Result<Value, String> {
                let bytes = to_bytes(data)?;
                let engine = base64::engine::GeneralPurpose::new(&STANDARD, PAD);
                let decoded = engine.decode(&bytes).map_err(|e| e.to_string())?;
                Ok(Value::ByteArray(GcPtr::new(Mutex::new(vec_u8_into_i8(
                    decoded,
                )))))
            },
        ),
    );

    registry.define(
        "cljrs.base64/encode-url",
        wrap_fn1(
            "cljrs.base64/encode-url",
            |data: Value| -> Result<String, String> {
                let bytes = to_bytes(data)?;
                let engine = base64::engine::GeneralPurpose::new(&URL_SAFE, NO_PAD);
                let encoded = engine.encode(&bytes);
                Ok(encoded)
            },
        ),
    );

    registry.define(
        "cljrs.base64/decode-url",
        wrap_fn1(
            "cljrs.base64/decode-url",
            |data: Value| -> Result<Value, String> {
                let bytes = to_bytes(data)?;
                let engine = base64::engine::GeneralPurpose::new(&URL_SAFE, NO_PAD);
                let decoded = engine.decode(&bytes).map_err(|e| e.to_string())?;
                Ok(Value::ByteArray(GcPtr::new(Mutex::new(vec_u8_into_i8(
                    decoded,
                )))))
            },
        ),
    );

    registry.env().mark_loaded("cljrs.base64");
}

/// # Safety
/// `registry` must be a valid, non-null `*mut Registry` and must remain
/// uniquely borrowed for the duration of the call. The cljrs CLI satisfies
/// both: it allocates the `Registry` on its stack and hands the only pointer
/// to it across the FFI boundary.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cljrs_init(registry: *mut Registry) {
    register(unsafe { &mut *registry });
}
