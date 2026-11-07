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
//! let runtime = cljrs_runtime::Runtime::builder()
//!     .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
//!     .build()
//!     .unwrap();
//! cljrs_stdlib::install(&runtime);
//! let globals = runtime.globals().clone();
//! let config = cljrs_nrepl::Config::default();
//! let server = cljrs_nrepl::start(config, globals).unwrap();
//! println!("nREPL listening on port {}", server.port());
//! server.serve().unwrap(); // blocks this thread processing evals
//! ```

pub mod bencode;
mod engine;
pub mod protocol;
mod server;

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc;

use cljrs_runtime::tiered::{Env, EvalError, GlobalEnv};
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
    /// Set by an `interrupt` op. Checked before the job starts, and installed
    /// on the gas meter while it evaluates so a running eval stops too.
    pub cancelled: Arc<cljrs_runtime::env::gas::InterruptFlag>,
    /// Entry to clear from the in-flight registry when the job completes.
    pub pending_key: Option<server::PendingKey>,
    pub pending: Arc<server::Pending>,
}

/// Signature of the per-form evaluator the interpreter thread uses.
///
/// The CLI passes its own driver (which runs each form on the process-wide
/// async `LocalSet` so core.async / `^:async` code makes progress); library
/// embedders can pass [`cljrs_runtime::tiered::eval`] or their own wrapper.
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
        self.serve_with(cljrs_runtime::tiered::eval)
    }

    /// Like [`Server::serve`], but every top-level form is evaluated through
    /// `eval_form`, letting the caller drive async forms on its own runtime.
    pub fn serve_with(mut self, mut eval_form: impl EvalForm) -> miette::Result<()> {
        let engine = engine::Engine::new(self.globals.clone());
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

    /// Turn the server into a [`Poller`] for an interpreter thread that also
    /// serves other work. Same thread rule as [`Server::serve`].
    pub fn into_poller(self) -> Poller {
        let engine = Rc::new(engine::Engine::new(self.globals.clone()));
        Poller {
            server: self,
            engine,
            lanes: Rc::default(),
        }
    }
}

/// Whether a [`Poller`]'s server can still receive jobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollStatus {
    /// Still accepting connections; poll again later.
    Open,
    /// The network thread is gone (the server was shut down). No job will
    /// ever arrive again; drop the poller.
    Closed,
}

/// A non-blocking nREPL driver.
///
/// [`Server::serve_with`] blocks the interpreter thread on the job channel,
/// so a host whose interpreter thread also runs its own command loop (an
/// embedded addon isolate, say) could not have an nREPL and keep serving
/// itself. A `Poller` handles only the jobs already queued and returns, so
/// the host interleaves them with its own work.
///
/// There are two ways to handle a job:
///
/// - [`Poller::poll`] / [`Poller::poll_timeout`] run each job to completion
///   before returning. An eval that waits (`Thread/sleep`, a channel take,
///   `@(future ...)`) holds the host for the whole wait, even when the wait
///   parks a fiber and leaves the isolate free.
/// - [`Poller::spawn_pending`] starts each job on a fiber on the host's
///   `LocalSet` and returns at once. The host drives that `LocalSet` as it
///   already does (for example with [`cljrs_async::fiber::drive_until`] once
///   per frame), and replies are sent as the fibers run. A parked eval then
///   costs the host nothing.
///
/// Either way, jobs for one session run one at a time, in arrival order.
/// Spawned jobs for different sessions can interleave at parks. The two
/// modes can be mixed: while a session has spawned jobs in flight, `poll`
/// queues that session's new jobs behind them instead of running them.
///
/// Dropping it shuts the server down: the network thread stops, its thread
/// is joined, and the port file is removed.
pub struct Poller {
    server: Server,
    engine: Rc<engine::Engine>,
    lanes: Rc<Lanes>,
}

/// Jobs waiting behind a session's in-flight spawned job, by lane key. A key
/// is present exactly while a lane task for it is alive on the `LocalSet`.
type Lanes = RefCell<HashMap<String, VecDeque<Job>>>;

/// The ordering key of a job: its session, or the shared default session.
fn lane_key(job: &Job) -> String {
    job.req
        .session
        .clone()
        .unwrap_or_else(|| "default".to_string())
}

impl Poller {
    /// The port the listener is bound to.
    pub fn port(&self) -> u16 {
        self.server.port
    }

    pub fn shutdown_handle(&self) -> ShutdownHandle {
        self.server.shutdown.clone()
    }

    /// Handle every job queued right now, without waiting for more.
    pub fn poll(&mut self, eval_form: &mut impl EvalForm) -> PollStatus {
        loop {
            match self.server.job_rx.try_recv() {
                Ok(job) => self.handle_now(job, eval_form),
                Err(mpsc::TryRecvError::Empty) => return PollStatus::Open,
                Err(mpsc::TryRecvError::Disconnected) => return PollStatus::Closed,
            }
        }
    }

    /// Wait up to `timeout` for a job, then handle everything queued. For a
    /// host with nothing else to do at the moment, instead of spinning on
    /// [`Poller::poll`].
    pub fn poll_timeout(
        &mut self,
        eval_form: &mut impl EvalForm,
        timeout: std::time::Duration,
    ) -> PollStatus {
        match self.server.job_rx.recv_timeout(timeout) {
            Ok(job) => {
                self.handle_now(job, eval_form);
                self.poll(eval_form)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => PollStatus::Open,
            Err(mpsc::RecvTimeoutError::Disconnected) => PollStatus::Closed,
        }
    }

    /// Run `job` on this thread now, unless its session has spawned jobs in
    /// flight; then it waits its turn behind them.
    fn handle_now(&mut self, job: Job, eval_form: &mut impl EvalForm) {
        let job = match self.lanes.borrow_mut().get_mut(&lane_key(&job)) {
            Some(queue) => {
                queue.push_back(job);
                return;
            }
            None => job,
        };
        self.engine.handle(job, eval_form);
    }

    /// Start every job queued right now on a fiber on `local`, without
    /// running any of them, and return.
    ///
    /// Nothing is evaluated until the host drives `local` (with
    /// [`cljrs_async::fiber::drive_until`], [`cljrs_async::fiber::run_root`],
    /// or its own `block_on`). Each job's replies are sent from its fiber as
    /// it runs. Jobs for one session run in order, each after the previous
    /// one completes; jobs for different sessions may interleave whenever
    /// one parks. `interrupt` works as with [`Poller::poll`], including for
    /// an eval parked on a wait.
    ///
    /// `eval_form` runs inside a fiber, so it must not start a root drive
    /// itself: pass [`cljrs_runtime::tiered::eval`] (or a wrapper around
    /// it), not an evaluator built on [`cljrs_async::fiber::run_root`]. Waits
    /// inside the form park its fiber.
    ///
    /// Each job gets a fiber stack of the default size (see
    /// `CLJRS_FIBER_STACK_KB`), not the 64 MiB a root drive gets, so very
    /// deep non-tail recursion reaches its depth limit sooner than under
    /// [`Poller::poll`] with a `run_root` evaluator.
    pub fn spawn_pending<F>(&mut self, local: &tokio::task::LocalSet, eval_form: F) -> PollStatus
    where
        F: EvalForm + Clone + 'static,
    {
        loop {
            match self.server.job_rx.try_recv() {
                Ok(job) => self.spawn_job(local, job, &eval_form),
                Err(mpsc::TryRecvError::Empty) => return PollStatus::Open,
                Err(mpsc::TryRecvError::Disconnected) => return PollStatus::Closed,
            }
        }
    }

    /// Queue `job` on its session's lane, starting a lane task on `local`
    /// when the session has none.
    fn spawn_job<F>(&mut self, local: &tokio::task::LocalSet, job: Job, eval_form: &F)
    where
        F: EvalForm + Clone + 'static,
    {
        let key = lane_key(&job);
        {
            let mut lanes = self.lanes.borrow_mut();
            if let Some(queue) = lanes.get_mut(&key) {
                queue.push_back(job);
                return;
            }
            lanes.insert(key.clone(), VecDeque::from([job]));
        }
        // Closes the lane when the task ends, or when the `LocalSet` drops
        // it unfinished, so later jobs for the session are not stranded
        // behind a lane that no longer runs.
        struct LaneGuard {
            lanes: Rc<Lanes>,
            key: String,
        }
        impl Drop for LaneGuard {
            fn drop(&mut self) {
                if let Ok(mut lanes) = self.lanes.try_borrow_mut() {
                    lanes.remove(&self.key);
                }
            }
        }
        let guard = LaneGuard {
            lanes: self.lanes.clone(),
            key,
        };
        let engine = self.engine.clone();
        let eval_form = eval_form.clone();
        local.spawn_local(async move {
            loop {
                let next = guard
                    .lanes
                    .borrow_mut()
                    .get_mut(&guard.key)
                    .and_then(VecDeque::pop_front);
                let Some(job) = next else { break };
                run_on_fiber(engine.clone(), job, eval_form.clone()).await;
            }
            drop(guard);
        });
    }
}

/// Handle one job on a new fiber and wait for it.
async fn run_on_fiber<F>(engine: Rc<engine::Engine>, job: Job, mut eval_form: F)
where
    F: EvalForm + 'static,
{
    // Kept to answer the request if the fiber ends without finishing it
    // (it could not be created, or it unwound).
    let req = job.req.clone();
    let replies = job.replies.clone();
    let cancelled = job.cancelled.clone();
    let pending_key = job.pending_key.clone();
    let pending = job.pending.clone();
    let finished = Rc::new(std::cell::Cell::new(false));
    let done = finished.clone();
    let failure = match cljrs_async::fiber::Fiber::new(move || {
        engine.handle(job, &mut eval_form);
        done.set(true);
        Ok(Value::Nil)
    }) {
        Ok(fiber) => fiber.into_task().await.err(),
        Err(e) => Some(e),
    };
    if finished.get() {
        return;
    }
    let sid = req.session.clone().unwrap_or_default();
    let status: &[&str] = if cancelled.is_set() {
        &["interrupted"]
    } else {
        let msg = failure.map_or_else(
            || "nrepl: job did not finish".to_string(),
            |e| e.to_string(),
        );
        let _ = replies.send(
            protocol::Response::for_request(&req, &sid)
                .str_field("err", format!("{msg}\n"))
                .build(),
        );
        &["eval-error"]
    };
    let _ = replies.send(
        protocol::Response::for_request(&req, &sid)
            .status(status)
            .build(),
    );
    // Leave `Pending` before `done`, as `Engine::handle` does.
    if let Some(key) = &pending_key {
        pending.remove(key);
    }
    let _ = replies.send(
        protocol::Response::for_request(&req, &sid)
            .status(&["done"])
            .build(),
    );
}

impl Drop for Poller {
    fn drop(&mut self) {
        self.server.shutdown.shutdown();
        if let Some(handle) = self.server.net_thread.take() {
            let _ = handle.join();
        }
        if let Some(path) = &self.server.port_file {
            let _ = std::fs::remove_file(path);
        }
    }
}
