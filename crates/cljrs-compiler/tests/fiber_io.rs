//! Owned file/process work must leave the isolate free while it waits.
#![cfg(all(unix, not(target_arch = "wasm32")))]
#![allow(clippy::result_large_err)]
use cljrs_async::fiber;
use cljrs_runtime::env::env::Env;
use cljrs_value::Value;
use std::io::Write;
use std::time::Duration;

fn exercise(process: bool) {
    let directory = tempfile::tempdir().unwrap();
    let fifo = directory.path().join("gate");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let (progress_tx, progress_rx) = std::sync::mpsc::channel();
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let path = fifo.clone();
    std::thread::Builder::new().stack_size(16 * 1024 * 1024).spawn(move || {
        let _mutator = cljrs_gc::register_mutator();
        let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
        cljrs_stdlib::install(&runtime);
        let globals = runtime.into_globals();
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        tokio::task::LocalSet::new().block_on(&rt, async {
            cljrs_async::init(&globals);
            let started = std::rc::Rc::new(std::cell::Cell::new(false));
            let task_started = started.clone();
            let task = fiber::spawn(move || {
                task_started.set(true);
                if process {
                    let source = format!(
                        "(do (:out (clojure.rust.process/run [\"sh\" \"-c\" \"cat \\\"$1\\\"\" \"gate\" {:?}])))",
                        path.to_str().unwrap());
                    let form = cljrs_reader::Parser::new(source, "<fiber-io>".into()).parse_all().unwrap().remove(0);
                    cljrs_runtime::interp::eval::eval(&form, &mut Env::new(globals, "user"))
                } else {
                    let reader = cljrs_stdlib::io::IoReader::open(path.to_str().unwrap()).unwrap();
                    Ok(Value::string(reader.read_all().unwrap()))
                }
            }).unwrap();
            // This must run BEFORE the host releases the blocked file/process.
            while !started.get() { tokio::task::yield_now().await; }
            progress_tx.send(()).unwrap();
            let result = task.await.unwrap().into_result().unwrap();
            result_tx.send(format!("{result}")).unwrap();
        });
    }).unwrap();
    let progressed = progress_rx.recv_timeout(Duration::from_secs(5));
    // Release the gate even on failure, so a regressed carrier can unwind.
    let (released_tx, released_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut writer = std::fs::OpenOptions::new().write(true).open(fifo).unwrap();
        writer.write_all(b"ready").unwrap();
        released_tx.send(()).unwrap();
    });
    released_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        progressed.is_ok(),
        "the file/process operation pinned the isolate"
    );
    assert_eq!(
        result_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        "\"ready\""
    );
}

#[test]
fn file_open_and_read_park_while_another_task_runs() {
    exercise(false);
}
#[test]
fn process_wait_and_pipe_drain_park_while_another_task_runs() {
    exercise(true);
}
