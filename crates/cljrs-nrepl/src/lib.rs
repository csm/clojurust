#![allow(clippy::result_large_err)]
//! nREPL server for clojurust.
//!
//! Speaks the [nREPL protocol](https://nrepl.org) — bencode-encoded messages
//! over TCP — so editors (CIDER, Calva, Conjure, …) can connect to a running
//! clojurust instance.
//!
//! # Architecture
//!
//! `GcPtr` (and therefore `Value`, `Env`, `GlobalEnv`) is not `Send`, so all
//! interpreter state stays on the thread that created the `GlobalEnv`. The
//! server splits accordingly:
//!
//! - the **network thread** (spawned by [`start`]) runs a current-thread tokio
//!   runtime with the TCP listener; per-connection tasks decode bencode frames
//!   and answer `describe`/`interrupt` directly,
//! - every other op is packaged as a [`Job`] (plain `Send` data: strings plus
//!   a reply channel) and handed over an mpsc channel to the **interpreter
//!   thread**, which processes jobs in [`Server::serve`] /
//!   [`Server::serve_with`].
//!
//! # Usage
//!
//! ```no_run
//! let globals = cljrs_stdlib::standard_env();
//! let config = cljrs_nrepl::Config::default();
//! let server = cljrs_nrepl::start(config, globals).unwrap();
//! println!("nREPL listening on port {}", server.port());
//! server.serve().unwrap(); // blocks this thread processing evals
//! ```

pub mod bencode;
mod engine;
pub mod protocol;
mod server;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;

use cljrs_eval::{Env, EvalError, GlobalEnv};
use cljrs_value::Value;

use crate::bencode::Bencode;
use crate::protocol::Request;

/// Server configuration.
pub struct Config {
    /// Address to bind. Port 0 lets the OS pick a free port.
    pub addr: SocketAddr,
    /// When set, the bound port is written to this file (the `.nrepl-port`
    /// convention editors use to auto-connect). Removed again when
    /// [`Server::serve`] returns.
    pub port_file: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            addr: ([127, 0, 0, 1], 0).into(),
            port_file: None,
        }
    }
}

/// A unit of work sent from the network thread to the interpreter thread.
/// Carries only `Send` data — never GC'd values.
pub(crate) struct Job {
    pub req: Request,
    /// Channel back to the connection's writer task.
    pub replies: tokio::sync::mpsc::UnboundedSender<Bencode>,
    /// Set by an `interrupt` op; checked before the job starts evaluating.
    pub cancelled: Arc<AtomicBool>,
    /// Entry to clear from the in-flight registry when the job completes.
    pub pending_key: Option<server::PendingKey>,
    pub pending: Arc<server::Pending>,
}

/// Signature of the per-form evaluator the interpreter thread uses.
///
/// The CLI passes its own driver (which runs each form on the process-wide
/// async `LocalSet` so core.async / `^:async` code makes progress); library
/// embedders can pass [`cljrs_eval::eval`] or their own wrapper.
pub trait EvalForm: FnMut(&cljrs_reader::Form, &mut Env) -> Result<Value, EvalError> {}
impl<F: FnMut(&cljrs_reader::Form, &mut Env) -> Result<Value, EvalError>> EvalForm for F {}

/// A running nREPL server. The network thread is already accepting
/// connections; call [`Server::serve`] on the interpreter thread to start
/// processing evaluation requests.
pub struct Server {
    port: u16,
    job_rx: mpsc::Receiver<Job>,
    shutdown: ShutdownHandle,
    net_thread: Option<std::thread::JoinHandle<()>>,
    port_file: Option<PathBuf>,
    globals: Arc<GlobalEnv>,
}

/// Cloneable, `Send` handle that stops the server: the network thread shuts
/// down, which in turn ends the interpreter thread's `serve` loop.
#[derive(Clone)]
pub struct ShutdownHandle {
    tx: tokio::sync::watch::Sender<bool>,
}

impl ShutdownHandle {
    pub fn shutdown(&self) {
        let _ = self.tx.send(true);
    }
}

/// Bind the listener, write the port file, and spawn the network thread.
///
/// Must be called on the thread that owns `globals` — the same thread must
/// later call [`Server::serve`] (or [`Server::serve_with`]), because
/// evaluation state cannot move across threads.
pub fn start(config: Config, globals: Arc<GlobalEnv>) -> miette::Result<Server> {
    let listener = std::net::TcpListener::bind(config.addr)
        .map_err(|e| miette::miette!("nrepl: failed to bind {}: {e}", config.addr))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| miette::miette!("nrepl: set_nonblocking failed: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| miette::miette!("nrepl: local_addr failed: {e}"))?
        .port();

    if let Some(path) = &config.port_file {
        std::fs::write(path, format!("{port}\n"))
            .map_err(|e| miette::miette!("nrepl: failed to write {}: {e}", path.display()))?;
    }

    let (job_tx, job_rx) = mpsc::channel::<Job>();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let net_thread = std::thread::Builder::new()
        .name("cljrs-nrepl-net".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("nrepl: failed to build tokio runtime");
            rt.block_on(server::run(listener, job_tx, shutdown_rx));
            // Dropping the runtime aborts connection tasks, which drops their
            // job senders and unblocks the interpreter thread's recv loop.
        })
        .map_err(|e| miette::miette!("nrepl: failed to spawn network thread: {e}"))?;

    Ok(Server {
        port,
        job_rx,
        shutdown: ShutdownHandle { tx: shutdown_tx },
        net_thread: Some(net_thread),
        port_file: config.port_file,
        globals,
    })
}

impl Server {
    /// The port the listener is bound to.
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn shutdown_handle(&self) -> ShutdownHandle {
        self.shutdown.clone()
    }

    /// Process evaluation jobs on the current thread until the server is shut
    /// down. Forms are evaluated with the plain tree-walking/IR evaluator.
    pub fn serve(self) -> miette::Result<()> {
        self.serve_with(cljrs_eval::eval)
    }

    /// Like [`Server::serve`], but every top-level form is evaluated through
    /// `eval_form`, letting the caller drive async forms on its own runtime.
    pub fn serve_with(mut self, mut eval_form: impl EvalForm) -> miette::Result<()> {
        let mut engine = engine::Engine::new(self.globals.clone());
        // recv() errors once every job sender is gone, i.e. the network
        // thread (and all its connection tasks) has shut down.
        while let Ok(job) = self.job_rx.recv() {
            engine.handle(job, &mut eval_form);
        }
        if let Some(handle) = self.net_thread.take() {
            let _ = handle.join();
        }
        if let Some(path) = &self.port_file {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }
}
