//! Blocking-style Clojure handlers on one isolate. The large case needs an
//! increased file-descriptor limit: ulimit -n 65536.
#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::result_large_err)]
use cljrs_async::{channel::chan_ref, fiber};
use cljrs_runtime::env::{
    env::Env,
    gc_roots::{push_env_root, root_value},
};
use cljrs_value::{Keyword, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn echo_connections(count: usize) {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            tokio::task::LocalSet::new().block_on(&rt, async move {
                let runtime = cljrs_runtime::Runtime::builder()
                    .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
                    .build()
                    .unwrap();
                let globals = runtime.globals().clone();
                cljrs_net::init(&globals);
                let carrier = Env::new(globals.clone(), "user");
                let _root = push_env_root(&carrier);
                let server = cljrs_net::tcp::listen_on("127.0.0.1", 0, count, 1, 1).unwrap();
                let _root = root_value(&server);
                let Value::Map(server_map) = &server else {
                    unreachable!()
                };
                let get = |key| {
                    server_map
                        .get(&Value::keyword(Keyword::simple(key)))
                        .unwrap()
                };
                let Value::Str(addr) = get("local-addr") else {
                    unreachable!()
                };
                let address = addr.get().clone();
                let conns = get("conns");
                let resource = get("resource");
                let handler = cljrs_reader::Parser::new(
                    r#"
              (do (loop []
                (let [v (clojure.core.async/<!! (:in conn))]
                  (when (some? v)
                    (clojure.core.async/>!! (:out conn) v)
                    (recur))))
                (clojure.rust.net.tcp/close conn))"#
                        .into(),
                    "<fiber-echo>".into(),
                )
                .parse_all()
                .unwrap()
                .remove(0);
                let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
                let server_task = fiber::spawn_rooted(vec![conns.clone()], move || {
                    let Value::NativeObject(channel) = conns else {
                        unreachable!()
                    };
                    let mut handlers = Vec::new();
                    for _ in 0..count {
                        let conn = chan_ref(channel.get()).take_blocking();
                        let roots = vec![conn.clone()];
                        let globals = globals.clone();
                        let handler = handler.clone();
                        handlers.push(fiber::spawn_rooted(roots, move || {
                            let mut env = Env::new(globals, "user");
                            env.push_frame();
                            env.bind("conn".into(), conn);
                            let _root = push_env_root(&env);
                            cljrs_runtime::interp::eval::eval(&handler, &mut env)
                        })?);
                    }
                    accepted_tx.send(()).unwrap();
                    for handler in handlers {
                        fiber::park_on(handler).unwrap().into_result()?;
                    }
                    Ok(Value::Nil)
                })
                .unwrap();
                let barrier = Arc::new(tokio::sync::Barrier::new(count + 1));
                let mut clients = Vec::new();
                for _ in 0..count {
                    let address = address.clone();
                    let barrier = barrier.clone();
                    clients.push(tokio::task::spawn_local(async move {
                        let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
                        barrier.wait().await;
                        socket.write_all(b"fiber echo").await.unwrap();
                        let mut response = [0; 10];
                        socket.read_exact(&mut response).await.unwrap();
                        assert_eq!(&response, b"fiber echo");
                        socket.shutdown().await.unwrap();
                    }));
                }
                tokio::time::timeout(Duration::from_secs(120), async {
                    accepted_rx.await.unwrap(); // All connections/handlers are live together.
                    barrier.wait().await;
                    let responses = async {
                        for client in clients {
                            client.await.map_err(|e| e.to_string())?;
                        }
                        Ok::<(), String>(())
                    };
                    let server = async {
                        server_task
                            .await
                            .map_err(|e| e.to_string())?
                            .into_result()
                            .map_err(|e| e.to_string())?;
                        Ok::<(), String>(())
                    };
                    tokio::try_join!(responses, server).unwrap();
                })
                .await
                .expect("fiber echo timed out");
                let Value::Resource(resource) = resource else {
                    unreachable!()
                };
                resource.close().unwrap();
            });
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn blocking_style_echo_on_one_isolate() {
    echo_connections(128);
}

#[test]
#[ignore = "10,000 simultaneous sockets require ulimit -n 65536"]
fn ten_thousand_blocking_style_connections() {
    echo_connections(10_000);
}
