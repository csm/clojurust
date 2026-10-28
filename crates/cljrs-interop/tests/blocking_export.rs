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
}
