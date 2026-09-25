//! Native implementation of `clojure.rust.process`: run a host program and
//! capture its result. The substrate under `clojure.java.shell/sh`.
//!
//! `(run argv)` / `(run argv {:in "..." :dir "..." :env {"NAME" "value"} :out-bytes true})`
//! returns `{:exit n :out "..." :err "..."}`. A non-zero exit is data, not an
//! error. `:env` replaces the child environment (JVM `sh` semantics); the
//! program is still found on the parent's PATH. A child killed by signal N
//! reports exit 128+N. The program is started by argv, never through a shell.
//! stdout/stderr are decoded as UTF-8, lossily; `:out-bytes true` returns
//! stdout as a byte array instead.
//!
//! Strata: `collect` reads a `Request` value out of the Clojure arguments,
//! `command` and `exit_code` / `result_value` are pure promotions, `execute`
//! is the only boundary that touches the OS, `builtin_run` composes them.

use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};

use cljrs_gc::GcPtr;
use cljrs_runtime::env::env::GlobalEnv;
use cljrs_value::{Arity, Keyword, MapValue, NativeFn, Value, ValueError, ValueResult};

/// The native fn's name, which is also its entry in the transaction policy
/// denylist (`cljrs_runtime::env::policy::check_native`).
pub const RUN_NAME: &str = "clojure.rust.process/run";

pub fn register(globals: &Arc<GlobalEnv>, ns: &str) {
    let nf = NativeFn::new(RUN_NAME, Arity::Variadic { min: 1 }, builtin_run);
    globals.intern(ns, Arc::from("run"), Value::NativeFunction(GcPtr::new(nf)));
}

// ── Value objects ───────────────────────────────────────────────────────────────

/// One program invocation, independent of how it was asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub argv: Vec<String>,
    pub stdin: Option<String>,
    pub dir: Option<String>,
    pub env: Option<Vec<(String, String)>>,
    pub out_bytes: bool,
}

// ── Collect: Clojure arguments -> Request ───────────────────────────────────────

fn kw(name: &str) -> Value {
    Value::keyword(Keyword::simple(name))
}

fn arg_error(detail: &str) -> ValueError {
    ValueError::Other(format!("clojure.rust.process/run: {detail}"))
}

/// A variable name `Command::env` accepts: non-empty, no `=`, no NUL.
fn env_name(k: &Value) -> Option<String> {
    let name = match k {
        Value::Str(s) => s.get().clone(),
        Value::Keyword(k) => k.get().name.to_string(),
        _ => return None,
    };
    let valid = !name.is_empty() && !name.contains('=') && !name.contains('\0');
    valid.then_some(name)
}

fn opt_string(opts: &MapValue, key: &str) -> ValueResult<Option<String>> {
    match opts.get(&kw(key)) {
        None | Some(Value::Nil) => Ok(None),
        Some(Value::Str(s)) => Ok(Some(s.get().clone())),
        Some(_) => Err(arg_error(&format!("the :{key} option must be a string"))),
    }
}

fn collect_argv(v: &Value) -> ValueResult<Vec<String>> {
    let bad = || arg_error("the command must be a non-empty vector of strings");
    let argv: Vec<String> = match v {
        Value::Vector(v) => v
            .get()
            .iter()
            .map(|e| match e {
                Value::Str(s) => Ok(s.get().clone()),
                _ => Err(arg_error("every command element must be a string")),
            })
            .collect::<ValueResult<_>>()?,
        _ => return Err(bad()),
    };
    if argv.is_empty() {
        Err(bad())
    } else {
        Ok(argv)
    }
}

fn collect_env(opts: &MapValue) -> ValueResult<Option<Vec<(String, String)>>> {
    match opts.get(&kw("env")) {
        None | Some(Value::Nil) => Ok(None),
        Some(Value::Map(m)) => m
            .iter()
            .map(|(k, v)| match (env_name(k), v) {
                (Some(name), Value::Str(s)) => Ok((name, s.get().clone())),
                _ => Err(arg_error(
                    "every :env key must be a valid variable name (string/keyword) and every value a string",
                )),
            })
            .collect::<ValueResult<Vec<_>>>()
            .map(Some),
        Some(_) => Err(arg_error("the :env option must be a map of name to string value")),
    }
}

/// Read a `Request` out of `(run argv)` / `(run argv opts)`.
pub fn collect(args: &[Value]) -> ValueResult<Request> {
    if args.len() > 2 {
        return Err(arg_error("expects (run argv) or (run argv opts)"));
    }
    let argv = collect_argv(&args[0])?;
    let opts = match args.get(1) {
        None | Some(Value::Nil) => {
            return Ok(Request {
                argv,
                stdin: None,
                dir: None,
                env: None,
                out_bytes: false,
            });
        }
        Some(Value::Map(m)) => m,
        Some(_) => return Err(arg_error("the options argument must be a map")),
    };
    Ok(Request {
        argv,
        stdin: opt_string(opts, "in")?,
        dir: opt_string(opts, "dir")?,
        env: collect_env(opts)?,
        out_bytes: matches!(opts.get(&kw("out-bytes")), Some(Value::Bool(true))),
    })
}

// ── Promote: pure derivations ───────────────────────────────────────────────────

/// The `Command` a request describes. Builds, never spawns.
pub fn command(req: &Request) -> Command {
    let mut cmd = Command::new(&req.argv[0]);
    cmd.args(&req.argv[1..])
        .stdin(if req.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = &req.dir {
        cmd.current_dir(dir);
    }
    if let Some(env) = &req.env {
        cmd.env_clear()
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    }
    cmd
}

/// A process status as the integer `sh` reports: the exit code, or 128+N for
/// a child killed by signal N.
#[cfg(unix)]
pub fn exit_code(status: std::process::ExitStatus) -> i64 {
    use std::os::unix::process::ExitStatusExt;
    match (status.code(), status.signal()) {
        (Some(code), _) => code as i64,
        (None, Some(sig)) => 128 + sig as i64,
        (None, None) => -1,
    }
}

#[cfg(not(unix))]
pub fn exit_code(status: std::process::ExitStatus) -> i64 {
    status.code().map(|c| c as i64).unwrap_or(-1)
}

/// The `{:exit :out :err}` map for a finished process.
pub fn result_value(output: &Output, out_bytes: bool) -> Value {
    let out = if out_bytes {
        let bytes: Vec<i8> = output.stdout.iter().map(|b| *b as i8).collect();
        Value::ByteArray(GcPtr::new(Mutex::new(bytes)))
    } else {
        Value::string(String::from_utf8_lossy(&output.stdout).into_owned())
    };
    Value::Map(MapValue::from_pairs(vec![
        (kw("exit"), Value::Long(exit_code(output.status))),
        (kw("out"), out),
        (
            kw("err"),
            Value::string(String::from_utf8_lossy(&output.stderr).into_owned()),
        ),
    ]))
}

// ── Boundary: the only code that touches the OS ─────────────────────────────────

/// Spawn `cmd`, feed `stdin`, drain stdout/stderr, wait.
///
/// stdin is written from its own thread while `wait_with_output` drains, so a
/// child that answers before it finishes reading cannot deadlock the pipes. A
/// child that exits unread makes the write fail with EPIPE (Rust ignores
/// SIGPIPE); that is the child's choice, not an error of the call.
pub fn execute(mut cmd: Command, program: &str, stdin: Option<String>) -> ValueResult<Output> {
    let mut child = cmd.spawn().map_err(|e| {
        ValueError::Other(format!(
            "clojure.rust.process/run: cannot run program '{program}' ({e})"
        ))
    })?;
    let feeder = match (stdin, child.stdin.take()) {
        (Some(bytes), Some(mut pipe)) => Some(std::thread::spawn(move || {
            let _ = pipe.write_all(bytes.as_bytes());
        })),
        _ => None,
    };
    let output = child.wait_with_output().map_err(|e| {
        ValueError::Other(format!(
            "clojure.rust.process/run: waiting for '{program}' failed ({e})"
        ))
    })?;
    if let Some(handle) = feeder {
        let _ = handle.join();
    }
    Ok(output)
}

// ── Facade ──────────────────────────────────────────────────────────────────────

fn builtin_run(args: &[Value]) -> ValueResult<Value> {
    let req = collect(args)?;
    let output = execute(command(&req), &req.argv[0], req.stdin.clone())?;
    Ok(result_value(&output, req.out_bytes))
}
