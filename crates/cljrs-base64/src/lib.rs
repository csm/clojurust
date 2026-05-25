use std::sync::Mutex;
use base64::alphabet::{STANDARD, URL_SAFE};
use base64::Engine;
use base64::engine::general_purpose::{NO_PAD, PAD};
use cljrs_gc::GcPtr;
use cljrs_interop::{wrap_fn1, Registry};
use cljrs_value::Value;

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

pub fn register(registry: &mut Registry) {
    registry.define(
        "base64/encode",
        wrap_fn1("base64/encode", |data: Value| -> Result<String, String> {
            let bytes = to_bytes(data)?;
            let engine = base64::engine::GeneralPurpose::new(&STANDARD, PAD);
            Ok(engine.encode(&bytes))
        })
    );

    registry.define(
        "base64/decode",
        wrap_fn1("base64/decode", |data: Value| -> Result<Value, String> {
            let bytes = to_bytes(data)?;
            let engine = base64::engine::GeneralPurpose::new(&STANDARD, PAD);
            let decoded = engine.decode(&bytes).map_err(|e| e.to_string())?;
            Ok(Value::ByteArray(GcPtr::new(Mutex::new(vec_u8_into_i8(decoded)))))
        })
    );

    registry.define(
        "base64/encode-url",
        wrap_fn1("base64/encode-url", |data: Value| -> Result<String, String> {
            let bytes = to_bytes(data)?;
            let engine = base64::engine::GeneralPurpose::new(&URL_SAFE, NO_PAD);
            let encoded = engine.encode(&bytes);
            Ok(encoded)
        })
    );

    registry.define(
        "base64/decode-url",
        wrap_fn1("base64/decode-url", |data: Value| -> Result<Value, String> {
            let bytes = to_bytes(data)?;
            let engine = base64::engine::GeneralPurpose::new(&URL_SAFE, NO_PAD);
            let decoded = engine.decode(&bytes).map_err(|e| e.to_string())?;
            Ok(Value::ByteArray(GcPtr::new(Mutex::new(vec_u8_into_i8(decoded)))))
        })
    );

    registry.env().mark_loaded("base64");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cljrs_init(registry: *mut Registry) {
    register(unsafe { &mut *registry });
}
