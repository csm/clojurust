//! Interpreter-thread side of the server: session registry and op handlers.
//!
//! Runs on the thread that owns the `GlobalEnv` (GC'd values are not `Send`).
//! Jobs arrive from the network thread one at a time; replies go back as
//! ready-made bencode messages over the connection's reply channel.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cljrs_env::dynamics;
use cljrs_eval::{EvalError, GlobalEnv};
use cljrs_gc::GcPtr;
use cljrs_value::{Keyword, Value, Var};
use tokio::sync::mpsc::UnboundedSender;

use crate::bencode::Bencode;
use crate::protocol::{Request, Response};
use crate::{EvalForm, Job};

/// Hidden namespace holding each session's retained values (`*1`/`*2`/`*3`/
/// `*e`) as interned vars. Namespaces are GC roots, so this keeps values
/// alive between evals — nothing else traces values held by Rust across
/// evaluations.
const STATE_NS: &str = "cljrs.nrepl.session-state";

/// Slot names used for the per-session state vars in [`STATE_NS`].
const STAR_SLOTS: [&str; 4] = ["1", "2", "3", "e"];

pub(crate) struct Engine {
    globals: Arc<GlobalEnv>,
    sessions: HashMap<String, Session>,
    /// `clojure.core`'s `*1`/`*2`/`*3`/`*e` vars, bound per-request via the
    /// dynamics stack so user code sees session-correct values.
    star_vars: Option<[GcPtr<Var>; 4]>,
    session_counter: u64,
}

struct Session {
    env: cljrs_eval::Env,
    /// Most recent values, indexed as [*1, *2, *3, *e].
    stars: [Value; 4],
}

impl Session {
    fn new(globals: Arc<GlobalEnv>) -> Session {
        Session {
            env: cljrs_eval::Env::new(globals, "user"),
            stars: [Value::Nil, Value::Nil, Value::Nil, Value::Nil],
        }
    }
}

impl Engine {
    pub(crate) fn new(globals: Arc<GlobalEnv>) -> Engine {
        let star_var = |name: &str| globals.lookup_var_in_ns("clojure.core", name);
        let star_vars = match (
            star_var("*1"),
            star_var("*2"),
            star_var("*3"),
            star_var("*e"),
        ) {
            (Some(v1), Some(v2), Some(v3), Some(ve)) => Some([v1, v2, v3, ve]),
            _ => None, // stdlib without REPL vars — *1/*2/*3/*e support disabled
        };
        Engine {
            globals,
            sessions: HashMap::new(),
            star_vars,
            session_counter: 0,
        }
    }

    pub(crate) fn handle(&mut self, job: Job, eval_form: &mut impl EvalForm) {
        let Job {
            req,
            replies,
            cancelled,
            pending_key,
            pending,
        } = job;
        if cancelled.load(Ordering::SeqCst) {
            // Interrupted while still queued: drop the work entirely.
            let sid = req.session.clone().unwrap_or_default();
            let _ = replies.send(
                Response::for_request(&req, &sid)
                    .status(&["done", "interrupted"])
                    .build(),
            );
        } else {
            match req.op.as_str() {
                "clone" => self.op_clone(&req, &replies),
                "close" => self.op_close(&req, &replies),
                "ls-sessions" => self.op_ls_sessions(&req, &replies),
                "eval" => self.op_eval(&req, &replies, eval_form, &cancelled),
                "load-file" => self.op_load_file(&req, &replies, eval_form, &cancelled),
                "completions" => self.op_completions(&req, &replies),
                "lookup" => self.op_lookup(&req, &replies),
                _ => {
                    let sid = self.ensure_session(req.session.as_deref());
                    let _ = replies.send(
                        Response::for_request(&req, &sid)
                            .str_field("op", &req.op)
                            .status(&["error", "unknown-op", "done"])
                            .build(),
                    );
                }
            }
        }
        if let Some(key) = &pending_key {
            pending.remove(key);
        }
    }

    // ── Sessions ──────────────────────────────────────────────────────────────

    /// Resolve the request's session, creating it if unknown. Requests that
    /// carry no session share a `"default"` session (real nREPL hands each a
    /// transient one; a stable default is simpler and friendlier to scripted
    /// clients).
    fn ensure_session(&mut self, sid: Option<&str>) -> String {
        let sid = sid.unwrap_or("default").to_string();
        if !self.sessions.contains_key(&sid) {
            self.sessions
                .insert(sid.clone(), Session::new(self.globals.clone()));
        }
        sid
    }

    fn new_session_id(&mut self) -> String {
        self.session_counter += 1;
        let micros = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros())
            .unwrap_or(0);
        format!("session-{micros:x}-{:x}", self.session_counter)
    }

    fn op_clone(&mut self, req: &Request, replies: &UnboundedSender<Bencode>) {
        // The new session inherits the source session's namespace.
        let source_ns = req
            .session
            .as_deref()
            .and_then(|s| self.sessions.get(s))
            .map(|s| s.env.current_ns.clone());
        let new_sid = self.new_session_id();
        let mut session = Session::new(self.globals.clone());
        if let Some(ns) = source_ns {
            session.env.current_ns = ns;
        }
        self.sessions.insert(new_sid.clone(), session);
        let sid = req.session.clone().unwrap_or_else(|| new_sid.clone());
        let _ = replies.send(
            Response::for_request(req, &sid)
                .str_field("new-session", &new_sid)
                .status(&["done"])
                .build(),
        );
    }

    fn op_close(&mut self, req: &Request, replies: &UnboundedSender<Bencode>) {
        let sid = req.session.clone().unwrap_or_default();
        self.sessions.remove(&sid);
        // Drop the session's retained values so the GC can reclaim them.
        if let Some(ns) = self.globals.namespaces.read().unwrap().get(STATE_NS) {
            let mut interns = ns.get().interns.lock().unwrap();
            for slot in STAR_SLOTS {
                interns.remove(format!("{sid}-{slot}").as_str());
            }
        }
        let _ = replies.send(
            Response::for_request(req, &sid)
                .status(&["done", "session-closed"])
                .build(),
        );
    }

    fn op_ls_sessions(&mut self, req: &Request, replies: &UnboundedSender<Bencode>) {
        let sessions: Vec<Bencode> = self.sessions.keys().map(Bencode::str).collect();
        let sid = req.session.clone().unwrap_or_default();
        let _ = replies.send(
            Response::for_request(req, &sid)
                .field("sessions", Bencode::List(sessions))
                .status(&["done"])
                .build(),
        );
    }

    // ── Evaluation ────────────────────────────────────────────────────────────

    fn op_eval(
        &mut self,
        req: &Request,
        replies: &UnboundedSender<Bencode>,
        eval_form: &mut impl EvalForm,
        cancelled: &AtomicBool,
    ) {
        let Some(code) = req.code.clone() else {
            let sid = self.ensure_session(req.session.as_deref());
            let _ = replies.send(
                Response::for_request(req, &sid)
                    .status(&["error", "no-code", "done"])
                    .build(),
            );
            return;
        };
        self.eval_code(req, replies, eval_form, cancelled, &code, "<nrepl>");
    }

    fn op_load_file(
        &mut self,
        req: &Request,
        replies: &UnboundedSender<Bencode>,
        eval_form: &mut impl EvalForm,
        cancelled: &AtomicBool,
    ) {
        let Some(file) = req.file.clone() else {
            let sid = self.ensure_session(req.session.as_deref());
            let _ = replies.send(
                Response::for_request(req, &sid)
                    .status(&["error", "no-file", "done"])
                    .build(),
            );
            return;
        };
        let filename = req
            .file_name
            .clone()
            .unwrap_or_else(|| "<load-file>".into());
        self.eval_code(req, replies, eval_form, cancelled, &file, &filename);
    }

    /// Shared body of `eval` and `load-file`: evaluate all forms in `code`,
    /// streaming `out`/`value`/`err` messages, then a final `done`.
    fn eval_code(
        &mut self,
        req: &Request,
        replies: &UnboundedSender<Bencode>,
        eval_form: &mut impl EvalForm,
        cancelled: &AtomicBool,
        code: &str,
        filename: &str,
    ) {
        let sid = self.ensure_session(req.session.as_deref());
        let globals = self.globals.clone();
        let star_vars = self.star_vars.clone();
        let session = self.sessions.get_mut(&sid).expect("session just ensured");

        // Honor the request's namespace when it exists; an unknown namespace
        // would be created empty (no clojure.core refers) and break
        // resolution, so fall back to the session's namespace instead.
        if let Some(ns) = &req.ns
            && globals.namespaces.read().unwrap().contains_key(ns.as_str())
        {
            session.env.current_ns = Arc::from(ns.as_str());
        }

        let mut parser = cljrs_reader::Parser::new(code.to_string(), filename.to_string());
        let forms = match parser.parse_all() {
            Ok(forms) => forms,
            Err(e) => {
                let msg = format!("{e}");
                let _ = replies.send(
                    Response::for_request(req, &sid)
                        .str_field("err", format!("{msg}\n"))
                        .build(),
                );
                let _ = replies.send(
                    Response::for_request(req, &sid)
                        .str_field("ex", &msg)
                        .status(&["eval-error"])
                        .build(),
                );
                let _ = replies.send(Response::for_request(req, &sid).status(&["done"]).build());
                return;
            }
        };

        // Bind *1/*2/*3/*e to this session's values for the duration of the
        // request (updated after each form so `(+ 1 2) *1` works within one
        // message). The dynamics stack is also a GC root for the bound values.
        let guard = star_vars.as_ref().map(|vars| {
            let mut frame = HashMap::new();
            for (var, val) in vars.iter().zip(session.stars.iter()) {
                frame.insert(dynamics::var_key_of(var), val.clone());
            }
            dynamics::push_frame(frame)
        });

        let mut interrupted = false;
        for form in &forms {
            // Best-effort interrupt between top-level forms; a single form
            // that loops forever cannot be stopped.
            if cancelled.load(Ordering::SeqCst) {
                interrupted = true;
                break;
            }
            let _alloc_frame = cljrs_gc::push_alloc_frame();
            cljrs_builtins::builtins::push_output_capture();
            let result = eval_form(form, &mut session.env);
            let out = cljrs_builtins::builtins::pop_output_capture().unwrap_or_default();
            if !out.is_empty() {
                let _ = replies.send(
                    Response::for_request(req, &sid)
                        .str_field("out", &out)
                        .build(),
                );
            }
            match result {
                Ok(value) => {
                    let _ = replies.send(
                        Response::for_request(req, &sid)
                            .str_field("value", format!("{value}"))
                            .str_field("ns", session.env.current_ns.as_ref())
                            .build(),
                    );
                    session.stars[2] = session.stars[1].clone();
                    session.stars[1] = session.stars[0].clone();
                    session.stars[0] = value;
                    if let Some(vars) = &star_vars {
                        for (var, val) in vars.iter().zip(session.stars.iter()).take(3) {
                            dynamics::set_thread_local(var, val.clone());
                        }
                    }
                }
                Err(e) => {
                    let msg = eval_error_message(&e);
                    let _ = replies.send(
                        Response::for_request(req, &sid)
                            .str_field("err", format!("{msg}\n"))
                            .build(),
                    );
                    let _ = replies.send(
                        Response::for_request(req, &sid)
                            .str_field("ex", &msg)
                            .status(&["eval-error"])
                            .build(),
                    );
                    session.stars[3] = e.to_error_value();
                    if let Some(vars) = &star_vars {
                        dynamics::set_thread_local(&vars[3], session.stars[3].clone());
                    }
                    break;
                }
            }
        }
        drop(guard);

        // Persist the retained values where the GC will trace them.
        let stars = session.stars.clone();
        for (slot, val) in STAR_SLOTS.iter().zip(stars) {
            globals.intern(STATE_NS, format!("{sid}-{slot}").into(), val);
        }

        let status: &[&str] = if interrupted {
            &["done", "interrupted"]
        } else {
            &["done"]
        };
        let _ = replies.send(Response::for_request(req, &sid).status(status).build());
    }

    // ── Tooling ops ───────────────────────────────────────────────────────────

    fn op_completions(&mut self, req: &Request, replies: &UnboundedSender<Bencode>) {
        let sid = self.ensure_session(req.session.as_deref());
        let session = &self.sessions[&sid];
        let prefix = req.prefix.clone().unwrap_or_default();
        let context_ns: Arc<str> = match &req.ns {
            Some(ns)
                if self
                    .globals
                    .namespaces
                    .read()
                    .unwrap()
                    .contains_key(ns.as_str()) =>
            {
                Arc::from(ns.as_str())
            }
            _ => session.env.current_ns.clone(),
        };

        // (candidate, ns, kind) triples, sorted for stable output.
        let mut items: Vec<(String, String, &'static str)> = Vec::new();
        let namespaces = self.globals.namespaces.read().unwrap();

        if let Some((alias, name_prefix)) = prefix.split_once('/') {
            // Qualified prefix: complete interns of the aliased/named namespace.
            let full = self
                .globals
                .resolve_alias(&context_ns, alias)
                .unwrap_or_else(|| Arc::from(alias));
            if let Some(ns) = namespaces.get(&full) {
                for (name, var) in ns.get().interns.lock().unwrap().iter() {
                    if name.starts_with(name_prefix) {
                        items.push((format!("{alias}/{name}"), full.to_string(), var_kind(var)));
                    }
                }
            }
        } else {
            if let Some(ns) = namespaces.get(&context_ns) {
                let ns = ns.get();
                for map in [&ns.interns, &ns.refers] {
                    for (name, var) in map.lock().unwrap().iter() {
                        if name.starts_with(prefix.as_str()) {
                            items.push((
                                name.to_string(),
                                var.get().namespace.to_string(),
                                var_kind(var),
                            ));
                        }
                    }
                }
            }
            for ns_name in namespaces.keys() {
                if ns_name.starts_with(prefix.as_str()) && ns_name.as_ref() != STATE_NS {
                    items.push((ns_name.to_string(), ns_name.to_string(), "namespace"));
                }
            }
        }
        drop(namespaces);

        items.sort();
        items.dedup();
        let completions: Vec<Bencode> = items
            .into_iter()
            .map(|(candidate, ns, kind)| {
                let mut dict = BTreeMap::new();
                dict.insert(b"candidate".to_vec(), Bencode::str(candidate));
                dict.insert(b"ns".to_vec(), Bencode::str(ns));
                dict.insert(b"type".to_vec(), Bencode::str(kind));
                Bencode::Dict(dict)
            })
            .collect();

        let _ = replies.send(
            Response::for_request(req, &sid)
                .field("completions", Bencode::List(completions))
                .status(&["done"])
                .build(),
        );
    }

    fn op_lookup(&mut self, req: &Request, replies: &UnboundedSender<Bencode>) {
        let sid = self.ensure_session(req.session.as_deref());
        let session = &self.sessions[&sid];
        let context_ns: Arc<str> = match &req.ns {
            Some(ns)
                if self
                    .globals
                    .namespaces
                    .read()
                    .unwrap()
                    .contains_key(ns.as_str()) =>
            {
                Arc::from(ns.as_str())
            }
            _ => session.env.current_ns.clone(),
        };

        let sym = req.sym.clone().unwrap_or_default();
        let var = match sym.split_once('/') {
            Some((ns_part, name)) => {
                let full = self
                    .globals
                    .resolve_alias(&context_ns, ns_part)
                    .unwrap_or_else(|| Arc::from(ns_part));
                self.globals.lookup_var_in_ns(&full, name)
            }
            None => self.globals.lookup_var_in_ns(&context_ns, &sym),
        };

        let mut info = BTreeMap::new();
        if let Some(var) = var {
            let v = var.get();
            info.insert(b"ns".to_vec(), Bencode::str(v.namespace.as_ref()));
            info.insert(b"name".to_vec(), Bencode::str(v.name.as_ref()));
            let meta = v.meta.lock().unwrap().clone();
            if let Some(meta) = meta {
                if let Some(Value::Str(doc)) = meta_get(&meta, "doc") {
                    info.insert(b"doc".to_vec(), Bencode::str(doc.get().as_str()));
                }
                if let Some(arglists) = meta_get(&meta, "arglists") {
                    info.insert(
                        b"arglists-str".to_vec(),
                        Bencode::str(format!("{arglists}")),
                    );
                }
                if let Some(Value::Str(file)) = meta_get(&meta, "file") {
                    info.insert(b"file".to_vec(), Bencode::str(file.get().as_str()));
                }
                if let Some(Value::Long(line)) = meta_get(&meta, "line") {
                    info.insert(b"line".to_vec(), Bencode::Int(line));
                }
            }
        }
        let status: &[&str] = if info.is_empty() {
            &["done", "lookup-error"]
        } else {
            &["done"]
        };
        let _ = replies.send(
            Response::for_request(req, &sid)
                .field("info", Bencode::Dict(info))
                .status(status)
                .build(),
        );
    }
}

/// Completion kind for a var, mirroring cider-nrepl's categories.
fn var_kind(var: &GcPtr<Var>) -> &'static str {
    let v = var.get();
    if v.is_macro {
        return "macro";
    }
    match v.deref() {
        Some(Value::Macro(_)) => "macro",
        Some(
            Value::Fn(_)
            | Value::NativeFunction(_)
            | Value::BoundFn(_)
            | Value::ProtocolFn(_)
            | Value::MultiFn(_),
        ) => "function",
        _ => "var",
    }
}

/// Fetch `key` (as a keyword) from a metadata map value.
fn meta_get(meta: &Value, key: &str) -> Option<Value> {
    match meta {
        Value::Map(m) => m.get(&Value::keyword(Keyword::simple(key))),
        Value::WithMeta(inner, _) => meta_get(inner, key),
        _ => None,
    }
}

/// User-facing message for an evaluation error (same phrasing as the CLI's
/// `format_eval_error` in `crates/cljrs/src/main.rs`).
fn eval_error_message(e: &EvalError) -> String {
    match e {
        EvalError::Thrown(val) => format!("Unhandled exception: {val}"),
        EvalError::UnboundSymbol(s) => format!("Unable to resolve symbol: {s}"),
        EvalError::Arity {
            name,
            expected,
            got,
        } => format!("Wrong number of args ({got}) passed to {name}; expected {expected}"),
        EvalError::NotCallable(s) => format!("Not a function: {s}"),
        EvalError::Recur(_) => "recur outside of loop/fn".to_string(),
        other => other.to_string(),
    }
}
