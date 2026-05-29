//! Integration tests for `clojure.rust.io.async`: round-tripping writes/reads
//! and streaming a file's characters, all driven on a current-thread LocalSet.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use cljrs_async::eval_async::eval_async;
use cljrs_env::env::{Env, GlobalEnv};
use cljrs_reader::Parser;
use cljrs_value::Value;

/// Standard env with both the async runtime and async I/O registered.
fn io_env() -> Arc<GlobalEnv> {
    let globals = cljrs_interp::standard_env(None, None, None);
    cljrs_async::init(&globals);
    cljrs_io::init(&globals);
    globals
}

fn parse_one(src: &str) -> cljrs_reader::Form {
    let mut p = Parser::new(src.to_string(), "<test>".to_string());
    p.parse_all()
        .expect("parse error")
        .into_iter()
        .next()
        .expect("no form")
}

fn eval_sync(src: &str, env: &mut Env) -> Value {
    let mut p = Parser::new(src.to_string(), "<test>".to_string());
    let mut result = Value::Nil;
    for form in p.parse_all().expect("parse error") {
        result = cljrs_interp::eval::eval(&form, env).expect("eval error");
    }
    result
}

fn block_on_local<F: std::future::Future>(f: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("build runtime");
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, f)
}

/// A unique temp path so concurrent test runs don't collide.
fn temp_path(tag: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!("cljrs-io-{}-{}-{}.tmp", tag, std::process::id(), n));
    p.to_string_lossy().into_owned()
}

#[test]
fn spit_then_slurp_round_trip() {
    let globals = io_env();
    let path = temp_path("roundtrip");
    let cleanup = path.clone();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");

        // spit delivers the number of bytes written on its promise channel.
        let written = eval_async(
            &parse_one(&format!(
                "(await (clojure.core.async/take! \
                   (clojure.rust.io.async/spit \"{path}\" \"hello, world\")))"
            )),
            &mut env,
        )
        .await
        .unwrap();
        assert_eq!(written, Value::Long(12));

        // slurp reads it back as a string on its promise channel.
        let read = eval_async(
            &parse_one(&format!(
                "(await (clojure.core.async/take! \
                   (clojure.rust.io.async/slurp \"{path}\")))"
            )),
            &mut env,
        )
        .await
        .unwrap();
        assert_eq!(read, Value::string("hello, world"));
    });
    let _ = std::fs::remove_file(&cleanup);
}

#[test]
fn char_chan_streams_then_closes() {
    let globals = io_env();
    let path = temp_path("chars");
    std::fs::write(&path, "abc").unwrap();
    let cleanup = path.clone();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(
            &format!("(def ch (clojure.rust.io.async/char-chan \"{path}\"))"),
            &mut env,
        );
        for expected in ['a', 'b', 'c'] {
            let c = eval_async(&parse_one("(await (clojure.core.async/take! ch))"), &mut env)
                .await
                .unwrap();
            assert_eq!(c, Value::Char(expected));
        }
        // Channel closes at EOF: the next take yields nil.
        let end = eval_async(&parse_one("(await (clojure.core.async/take! ch))"), &mut env)
            .await
            .unwrap();
        assert_eq!(end, Value::Nil);
    });
    let _ = std::fs::remove_file(&cleanup);
}

#[test]
fn read_bytes_returns_prefix_byte_array() {
    let globals = io_env();
    let path = temp_path("prefix");
    std::fs::write(&path, "hello").unwrap();
    let cleanup = path.clone();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        let len = eval_async(
            &parse_one(&format!(
                "(alength (await (clojure.core.async/take! \
                   (clojure.rust.io.async/read-bytes \"{path}\" 3))))"
            )),
            &mut env,
        )
        .await
        .unwrap();
        assert_eq!(len, Value::Long(3));
    });
    let _ = std::fs::remove_file(&cleanup);
}
