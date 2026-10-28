use cljrs_interop::{NativeFn, Value, export, exports::ExportEntry};

#[export(ns = "parking-test", blocking)]
fn owned_input(input: String) -> Result<String, String> {
    if input == "fail" {
        return Err("native failure".into());
    }
    Ok(format!(
        "{input}:{}",
        std::thread::current().name().unwrap_or("unnamed")
    ))
}
#[export(blocking, ns = "parking-test")]
fn plain_result(input: i64) -> i64 {
    input + 1
}

fn exported(name: &str) -> NativeFn {
    let entry = cljrs_interop::inventory::iter::<ExportEntry>
        .into_iter()
        .find(|e| e.qualified == name)
        .unwrap();
    (entry.make_fn)()
}

#[test]
fn blocking_exports_convert_values_on_the_caller_thread() {
    let native = exported("parking-test/plain-result");
    assert_eq!((native.func)(&[Value::Long(41)]).unwrap(), Value::Long(42));
    assert_eq!(
        native.call_blocking(&[Value::Long(41)]).unwrap(),
        Value::Long(42)
    );
    cljrs_value::parking::install(cljrs_value::parking::Hooks {
        current: || Some(1),
        park: |_, _| unreachable!(),
        sleep: |_| unreachable!(),
        blocking: |job| {
            std::thread::Builder::new()
                .name("export-worker".into())
                .spawn(job)
                .unwrap()
                .join()
                .unwrap()
        },
    });
    let native = exported("parking-test/owned-input");
    assert_eq!(
        (native.func)(&[Value::string("ok")]).unwrap(),
        Value::string("ok:export-worker")
    );
    assert!(
        (native.func)(&[Value::string("fail")])
            .unwrap_err()
            .to_string()
            .contains("native failure")
    );
    assert!((native.func)(&[Value::Long(1)]).is_err());
    assert_eq!(
        exported("parking-test/no-result")
            .call_blocking(&[])
            .unwrap(),
        Value::Nil
    );
    assert_eq!(
        exported("parking-test/static-result")
            .call_blocking(&[])
            .unwrap(),
        Value::string("static result")
    );
    let ordinary = exported("parking-test/ordinary-export");
    assert_ne!(
        (ordinary.func)(&[Value::string("ordinary")]).unwrap(),
        Value::string("ordinary:export-worker")
    );
    assert_eq!(
        ordinary.call_blocking(&[Value::string("owned")]).unwrap(),
        Value::string("owned:export-worker")
    );
    assert!(ordinary.call_blocking(&[]).is_err());
    for (name, args) in [
        ("parking-test/managed-input", vec![Value::Long(1)]),
        ("parking-test/managed-output", vec![]),
        ("parking-test/managed-variadic", vec![]),
    ] {
        assert!(
            exported(name)
                .call_blocking(&args)
                .unwrap_err()
                .to_string()
                .contains("blocking cannot offload")
        );
    }
    assert_eq!(REJECTED_CALLS.load(std::sync::atomic::Ordering::Relaxed), 0);
    let conversion = exported("parking-test/custom-conversion");
    assert_eq!(
        (conversion.func)(&[Value::string("converted")]).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        conversion
            .call_blocking(&[Value::string("converted")])
            .unwrap(),
        Value::Bool(true)
    );
}

#[export(ns = "parking-test")]
fn ordinary_export(input: String) -> String {
    format!(
        "{input}:{}",
        std::thread::current().name().unwrap_or("unnamed")
    )
}
static REJECTED_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[export(ns = "parking-test")]
fn managed_input(_input: Value) -> i64 {
    REJECTED_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    1
}
#[export(ns = "parking-test")]
fn managed_output() -> Value {
    REJECTED_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Value::Long(1)
}
#[export(ns = "parking-test")]
fn managed_variadic(_args: &[Value]) -> Value {
    REJECTED_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Value::Nil
}

struct Input {
    caller: std::thread::ThreadId,
    text: String,
}
impl cljrs_interop::FromValue for Input {
    fn from_value(value: &Value) -> cljrs_interop::ValueResult<Self> {
        Ok(Self {
            caller: std::thread::current().id(),
            text: String::from_value(value)?,
        })
    }
}
struct Output {
    caller: std::thread::ThreadId,
    worker: std::thread::ThreadId,
    text: String,
}
impl cljrs_interop::IntoValue for Output {
    fn into_value(self) -> Value {
        assert_eq!(
            self.caller,
            std::thread::current().id(),
            "result conversion moved off the isolate"
        );
        assert_eq!(self.text, "converted");
        Value::Bool(self.caller != self.worker)
    }
}
#[export(ns = "parking-test")]
fn custom_conversion(input: Input) -> Output {
    Output {
        caller: input.caller,
        worker: std::thread::current().id(),
        text: input.text,
    }
}

#[export(ns = "parking-test")]
fn no_result() {}
#[export(ns = "parking-test")]
fn static_result() -> &'static str {
    "static result"
}
