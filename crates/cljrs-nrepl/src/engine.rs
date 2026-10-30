//! Interpreter-thread side of the server: session registry and op handlers.
//!
//! Runs on the thread that owns the `GlobalEnv` (GC'd values are not `Send`).
//! Jobs arrive from the network thread one at a time; replies go back as
//! ready-made bencode messages over the connection's reply channel.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cljrs_gc::GcPtr;
use cljrs_reader::Form;
use cljrs_reader::form::FormKind;
use cljrs_runtime::env::dynamics;
use cljrs_runtime::tiered::{EvalError, GlobalEnv};
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
    env: cljrs_runtime::tiered::Env,
    /// Most recent values, indexed as [*1, *2, *3, *e].
    stars: [Value; 4],
    #[cfg(feature = "no-gc")]
    ownership: cljrs_gc::region::RegionLease,
}

impl Session {
    fn new(globals: Arc<GlobalEnv>) -> Session {
        Session {
            env: cljrs_runtime::tiered::Env::new(globals, "user"),
            stars: [Value::Nil, Value::Nil, Value::Nil, Value::Nil],
            #[cfg(feature = "no-gc")]
            ownership: Default::default(),
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
        #[cfg(feature = "no-gc")]
        let _reads = cljrs_gc::region::ReadScope::enter();
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
                    .status(&["interrupted"])
                    .build(),
            );
            let _ = replies.send(Response::for_request(&req, &sid).status(&["done"]).build());
        } else {
            match req.op.as_str() {
                "clone" => self.op_clone(&req, &replies),
                "close" => self.op_close(&req, &replies),
                "ls-sessions" => self.op_ls_sessions(&req, &replies),
                "eval" | "load-file" => {
                    // The network thread's `interrupt` sets `cancelled`; with
                    // it installed on the execution-credit meter, the running
                    // eval stops at its next checkpoint in any tier.
                    let _interrupt =
                        cljrs_runtime::env::gas::InterruptGuard::install(cancelled.clone());
                    if req.op == "eval" {
                        self.op_eval(&req, &replies, eval_form, &cancelled);
                    } else {
                        self.op_load_file(&req, &replies, eval_form, &cancelled);
                    }
                }
                "completions" => self.op_completions(&req, &replies),
                "lookup" => self.op_lookup(&req, &replies),
                "macroexpand" => self.op_macroexpand(&req, &replies),
                "analyze-last-stacktrace" | "stacktrace" => {
                    self.op_analyze_last_stacktrace(&req, &replies)
                }
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
                if let Some(var) = interns.remove(format!("{sid}-{slot}").as_str()) {
                    #[cfg(feature = "no-gc")]
                    var.get().bind(Value::Nil);
                    #[cfg(not(feature = "no-gc"))]
                    let _ = var;
                }
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
            // Interrupted between top-level forms (a running form is stopped
            // by the interrupt flag installed on the gas meter, below).
            if cancelled.load(Ordering::SeqCst) {
                interrupted = true;
                break;
            }
            let _alloc_frame = cljrs_gc::push_alloc_frame();
            // Stream output while the form runs (at newlines / a size
            // threshold); the pop flushes whatever is left.
            let (out_req, out_sid, out_replies) = (req.clone(), sid.clone(), replies.clone());
            cljrs_runtime::builtins::builtins::push_streaming_output_capture(Box::new(
                move |text: &str| {
                    let _ = out_replies.send(
                        Response::for_request(&out_req, &out_sid)
                            .str_field("out", text)
                            .build(),
                    );
                },
            ));
            let result = eval_form(form, &mut session.env);
            let _ = cljrs_runtime::builtins::builtins::pop_output_capture();
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
                    #[cfg(feature = "no-gc")]
                    {
                        session.ownership =
                            cljrs_gc::region::RegionLease::for_values(&session.stars);
                    }
                    if let Some(vars) = &star_vars {
                        for (var, val) in vars.iter().zip(session.stars.iter()).take(3) {
                            dynamics::set_thread_local(var, val.clone());
                        }
                    }
                }
                // An interrupt unwinds as `GasExhausted`; any failure once the
                // flag is set is reported as the interrupt (a native bridge
                // may have wrapped the signal in another error).
                Err(_) if cancelled.load(Ordering::SeqCst) => {
                    interrupted = true;
                    break;
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
                    #[cfg(feature = "no-gc")]
                    {
                        session.ownership =
                            cljrs_gc::region::RegionLease::for_values(&session.stars);
                    }
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

        if interrupted {
            let _ = replies.send(
                Response::for_request(req, &sid)
                    .status(&["interrupted"])
                    .build(),
            );
        }
        let _ = replies.send(Response::for_request(req, &sid).status(&["done"]).build());
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
            let full = self.globals.resolve_ns_part_in(&context_ns, alias);
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

    /// cider-nrepl `macroexpand`: expand `code` in `ns` with the requested
    /// expander and reply with the printed expansion.
    fn op_macroexpand(&mut self, req: &Request, replies: &UnboundedSender<Bencode>) {
        let sid = self.ensure_session(req.session.as_deref());
        let globals = self.globals.clone();
        let session = self.sessions.get_mut(&sid).expect("session just ensured");
        let ns = req
            .ns
            .as_deref()
            .filter(|ns| globals.namespaces.read().unwrap().contains_key(*ns))
            .map(Arc::from)
            .unwrap_or_else(|| session.env.current_ns.clone());
        let mut env = cljrs_runtime::tiered::Env::new(globals.clone(), &ns);
        let expander = req.expander.as_deref().unwrap_or("macroexpand");
        let display = req.display_namespaces.as_deref();
        let code = req.code.clone().unwrap_or_default();

        let result: Result<String, String> = (|| {
            let mut parser = cljrs_reader::Parser::new(code, "<macroexpand>".to_string());
            let form = parser
                .parse_one()
                .map_err(|e| format!("{e}"))?
                .ok_or_else(|| "no form to expand".to_string())?;
            let _alloc_frame = cljrs_gc::push_alloc_frame();
            use cljrs_runtime::interp::macros;
            let mut expanded = match expander {
                "macroexpand-1" => macros::macroexpand_1(&form, &mut env),
                "macroexpand" => macros::macroexpand(&form, &mut env),
                "macroexpand-all" => macros::macroexpand_all(&form, &mut env),
                other => return Err(format!("unknown expander: {other}")),
            }
            .map_err(|e| eval_error_message(&e))?;
            match display {
                Some("none") => {
                    redisplay_symbols(&mut expanded, &|_, name| Some(name.to_string()));
                }
                Some("tidy") => redisplay_symbols(&mut expanded, &|qualifier, name| {
                    tidy_symbol(&globals, &ns, qualifier, name)
                }),
                _ => {}
            }
            let value = cljrs_runtime::builtins::form::form_to_value(&expanded)
                .map_err(|e| eval_error_message(&e))?;
            Ok(format!("{value}"))
        })();

        match result {
            Ok(expansion) => {
                let _ = replies.send(
                    Response::for_request(req, &sid)
                        .str_field("expansion", expansion)
                        .status(&["done"])
                        .build(),
                );
            }
            Err(msg) => {
                let _ = replies.send(
                    Response::for_request(req, &sid)
                        .str_field("err", format!("{msg}\n"))
                        .status(&["macroexpand-error", "done"])
                        .build(),
                );
            }
        }
    }

    /// cider-nrepl `analyze-last-stacktrace` (legacy `stacktrace`): one
    /// message per cause of the session's `*e`, outermost first.
    fn op_analyze_last_stacktrace(&mut self, req: &Request, replies: &UnboundedSender<Bencode>) {
        let sid = self.ensure_session(req.session.as_deref());
        let session = self.sessions.get(&sid).expect("session just ensured");
        let err = session.stars[3].clone();
        let causes: Vec<(String, String, Option<String>)> = match &err {
            Value::Nil => Vec::new(),
            Value::Error(e) => {
                let mut out = Vec::new();
                let mut cur = Some(e.clone());
                while let Some(ex) = cur {
                    let info = ex.get();
                    let data = info.data().map(|d| format!("{}", Value::Map(d)));
                    // An exception carrying ex-data is what Clojure calls an
                    // ExceptionInfo (CIDER keys its data display off that
                    // class); anything else keeps cljrs's own kind name.
                    let class = if data.is_some() {
                        "clojure.lang.ExceptionInfo".to_string()
                    } else {
                        info.type_name().to_string()
                    };
                    out.push((class, info.message(), data));
                    cur = info.cause();
                }
                out
            }
            // A thrown non-exception value (`(throw 42)`).
            other => vec![(other.type_name().to_string(), format!("{other}"), None)],
        };
        if causes.is_empty() {
            let _ = replies.send(
                Response::for_request(req, &sid)
                    .status(&["no-error", "done"])
                    .build(),
            );
            return;
        }
        for (class, message, data) in causes {
            let mut resp = Response::for_request(req, &sid)
                .str_field("class", class)
                .str_field("message", message)
                // cljrs records no stack frames on its exceptions yet; an
                // empty list is the honest answer.
                .field("stacktrace", Bencode::List(Vec::new()));
            if let Some(data) = data {
                resp = resp.str_field("data", data);
            }
            let _ = replies.send(resp.build());
        }
        let _ = replies.send(Response::for_request(req, &sid).status(&["done"]).build());
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
                let full = self.globals.resolve_ns_part_in(&context_ns, ns_part);
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

/// Replace every qualified symbol `qualifier/name` in `form` with what
/// `display(qualifier, name)` returns; `None` leaves the symbol as written.
fn redisplay_symbols(form: &mut Form, display: &impl Fn(&str, &str) -> Option<String>) {
    match &mut form.kind {
        FormKind::Symbol(s) => {
            if let Some((qualifier, name)) = s.split_once('/')
                && !qualifier.is_empty()
                && !name.is_empty()
                && let Some(shown) = display(qualifier, name)
            {
                *s = shown;
            }
        }
        FormKind::List(items)
        | FormKind::Vector(items)
        | FormKind::Map(items)
        | FormKind::Set(items)
        | FormKind::AnonFn(items) => items
            .iter_mut()
            .for_each(|item| redisplay_symbols(item, display)),
        FormKind::Quote(f)
        | FormKind::SyntaxQuote(f)
        | FormKind::Unquote(f)
        | FormKind::UnquoteSplice(f)
        | FormKind::Deref(f)
        | FormKind::Var(f)
        | FormKind::TaggedLiteral(_, f) => redisplay_symbols(f, display),
        FormKind::Meta(m, f) => {
            redisplay_symbols(m, display);
            redisplay_symbols(f, display);
        }
        _ => {}
    }
}

/// `display-namespaces` `tidy`: how `qualifier/name` reads from inside `ns`.
///
/// The qualifier is dropped only when the bare name resolves, in `ns`, to the
/// very var the qualified symbol names (defined there or referred), so the
/// expansion still evaluates as shown. Otherwise the namespace is shortened to
/// an alias `ns` has for it, or left as written.
fn tidy_symbol(globals: &GlobalEnv, ns: &str, qualifier: &str, name: &str) -> Option<String> {
    let full = globals.resolve_ns_part_in(ns, qualifier);
    let bare_is_same_var = globals.lookup_var_in_ns(ns, name).is_some_and(|var| {
        let var = var.get();
        var.namespace == full && var.name.as_ref() == name
    });
    if bare_is_same_var {
        return Some(name.to_string());
    }
    let namespaces = globals.namespaces.read().unwrap();
    let current = namespaces.get(ns)?.get();
    let aliases = current.aliases.lock().unwrap();
    aliases
        .iter()
        .filter(|(_, target)| **target == full)
        .map(|(alias, _)| alias)
        .min_by_key(|alias| (alias.len(), alias.to_string()))
        .map(|alias| format!("{alias}/{name}"))
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
        EvalError::GasExhausted => "gas exhausted".to_string(),
        EvalError::Recur(_) => "recur outside of loop/fn".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod display_namespaces_tests {
    use super::*;
    use proptest::prelude::*;

    /// A runtime with the stdlib, and a namespace `tidy.law` that has a core
    /// name shadowed, a namespace under two aliases, a referred var, and a
    /// required namespace with no alias.
    fn law_globals() -> Arc<GlobalEnv> {
        let runtime = cljrs_runtime::Runtime::builder()
            .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
            .build()
            .expect("runtime");
        cljrs_stdlib::install(&runtime);
        let globals = runtime.into_globals();
        let mut env = cljrs_runtime::tiered::Env::new(globals.clone(), "user");
        let src = "(ns tidy.law
                     (:refer-clojure :exclude [map])
                     (:require [clojure.string :as string]
                               [clojure.string :as s]
                               [clojure.set :refer [union]]
                               [clojure.walk]))
                   (def map 1)
                   (def local 2)";
        for form in parse(src) {
            cljrs_runtime::tiered::eval(&form, &mut env).expect("setup");
        }
        globals
    }

    fn parse(src: &str) -> Vec<Form> {
        cljrs_reader::Parser::new(src.to_string(), "<test>".to_string())
            .parse_all()
            .expect("parse")
    }

    fn tidied(globals: &GlobalEnv, symbol: &str) -> String {
        let (qualifier, name) = symbol.split_once('/').expect("a qualified symbol");
        tidy_symbol(globals, "tidy.law", qualifier, name).unwrap_or_else(|| symbol.to_string())
    }

    /// The var a printed symbol names when read from inside `tidy.law`.
    fn names_var(globals: &GlobalEnv, shown: &str) -> Option<(String, String)> {
        // The whole symbol first: core interns interop-style names such as
        // `Math/hypot` whole, so a `/` does not always separate a namespace.
        let whole = globals.lookup_var_in_ns("tidy.law", shown);
        let var = whole.or_else(|| {
            let (qualifier, name) = shown.split_once('/')?;
            let full = globals.resolve_ns_part_in("tidy.law", qualifier);
            globals.lookup_var_in_ns(&full, name)
        })?;
        let var = var.get();
        Some((var.namespace.to_string(), var.name.to_string()))
    }

    #[test]
    fn tidy_drops_a_namespace_only_where_the_bare_name_still_resolves() {
        let globals = law_globals();
        for (qualified, shown) in [
            ("clojure.core/+", "+"),
            ("tidy.law/local", "local"),
            ("tidy.law/map", "map"),
            // Excluded from the core refer and shadowed by a local def.
            ("clojure.core/map", "clojure.core/map"),
            ("clojure.set/union", "union"),
            // Required, not referred, no alias: left as written.
            ("clojure.walk/postwalk", "clojure.walk/postwalk"),
            // Not referred: the shorter of the two aliases.
            ("clojure.string/join", "s/join"),
            ("string/join", "s/join"),
            ("no.such.ns/f", "no.such.ns/f"),
        ] {
            assert_eq!(tidied(&globals, qualified), shown, "{qualified}");
        }
    }

    /// The law `tidy` exists to keep: whatever it prints for a var names that
    /// same var when read back in the namespace. Checked for every interned
    /// var of every loaded namespace.
    #[test]
    fn a_tidied_symbol_names_the_var_it_was_printed_for() {
        let globals = law_globals();
        let vars: Vec<(String, String)> = {
            let namespaces = globals.namespaces.read().unwrap();
            namespaces
                .iter()
                .flat_map(|(ns, ptr)| {
                    let interns = ptr.get().interns.lock().unwrap();
                    interns
                        .keys()
                        .map(|name| (ns.to_string(), name.to_string()))
                        .collect::<Vec<_>>()
                })
                .collect()
        };
        assert!(vars.len() > 500, "expected the stdlib, got {}", vars.len());
        let mut stripped = 0;
        for (ns, name) in vars {
            let shown = tidied(&globals, &format!("{ns}/{name}"));
            stripped += usize::from(shown == name);
            assert_eq!(
                names_var(&globals, &shown),
                Some((ns.clone(), name.clone())),
                "{ns}/{name} was shown as {shown}"
            );
        }
        assert!(stripped > 100, "core's refers should print bare");
    }

    /// A generated form: symbols, some qualified, under nested collections.
    #[derive(Clone, Debug)]
    enum Tree {
        Symbol(Option<String>, String),
        List(Vec<Tree>),
        Vector(Vec<Tree>),
        Quote(Box<Tree>),
    }

    impl Tree {
        fn source(&self, qualified: bool) -> String {
            let all = |items: &[Tree]| {
                items
                    .iter()
                    .map(|item| item.source(qualified))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            match self {
                Tree::Symbol(Some(qualifier), name) if qualified => format!("{qualifier}/{name}"),
                Tree::Symbol(_, name) => name.clone(),
                Tree::List(items) => format!("({})", all(items)),
                Tree::Vector(items) => format!("[{}]", all(items)),
                Tree::Quote(item) => format!("'{}", item.source(qualified)),
            }
        }
    }

    fn tree() -> impl Strategy<Value = Tree> {
        // The `x` keeps a generated name from reading as `nil`, `true`, …
        let symbol = (
            proptest::option::of("[a-z]{1,3}(\\.[a-z]{1,3})?"),
            "x[a-z]{0,3}",
        )
            .prop_map(|(qualifier, name)| Tree::Symbol(qualifier, name));
        symbol.prop_recursive(4, 24, 4, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 0..4).prop_map(Tree::List),
                proptest::collection::vec(inner.clone(), 0..4).prop_map(Tree::Vector),
                inner.prop_map(|item| Tree::Quote(Box::new(item))),
            ]
        })
    }

    fn printed(form: &Form) -> String {
        let value = cljrs_runtime::builtins::form::form_to_value(form).expect("form value");
        format!("{value}")
    }

    /// Metadata is part of the form: both the metadata and what it annotates
    /// are redisplayed.
    #[test]
    fn symbols_in_metadata_and_under_it_are_redisplayed() {
        let mut form = parse("^{:tag a.b/T} c.d/x").remove(0);
        redisplay_symbols(&mut form, &|_, name| Some(name.to_string()));
        let FormKind::Meta(meta, annotated) = &form.kind else {
            panic!("expected a metadata form, got {:?}", form.kind);
        };
        let _alloc_frame = cljrs_gc::push_alloc_frame();
        assert_eq!(printed(meta), "{:tag T}");
        assert_eq!(printed(annotated), "x");
    }

    proptest! {
        /// `none` prints exactly the form with every qualifier erased, and a
        /// second pass changes nothing.
        #[test]
        fn none_erases_every_qualifier_and_nothing_else(tree in tree()) {
            let _alloc_frame = cljrs_gc::push_alloc_frame();
            let mut form = parse(&tree.source(true)).remove(0);
            let bare = parse(&tree.source(false)).remove(0);
            let strip = |_: &str, name: &str| Some(name.to_string());
            redisplay_symbols(&mut form, &strip);
            prop_assert_eq!(printed(&form), printed(&bare));
            redisplay_symbols(&mut form, &strip);
            prop_assert_eq!(printed(&form), printed(&bare));
        }

        /// A display that answers `None` (what `tidy` does for a symbol it
        /// cannot shorten) leaves the form as written.
        #[test]
        fn a_symbol_with_no_shorter_display_is_left_as_written(tree in tree()) {
            let _alloc_frame = cljrs_gc::push_alloc_frame();
            let original = parse(&tree.source(true)).remove(0);
            let mut form = original.clone();
            redisplay_symbols(&mut form, &|_, _| None);
            prop_assert_eq!(printed(&form), printed(&original));
        }
    }
}
