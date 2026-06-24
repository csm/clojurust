use std::rc::Rc;
use std::sync::Arc;

use tokio::task::LocalSet;
use wasm_bindgen::prelude::*;

use cljrs_async::eval_async::{await_value, eval_async as eval_form_async};
use cljrs_builtins::builtins::{pop_output_capture, push_output_capture};
use cljrs_env::env::{Env, GlobalEnv};
use cljrs_reader::Parser;
use cljrs_value::Value;

/// A stateful Clojure REPL running entirely in an async context.
///
/// Backed by a Tokio `LocalSet` driven from the browser's Promise scheduler via
/// `wasm-bindgen-futures`.  Each `eval` call drives the LocalSet for the
/// duration of the evaluation, giving background tasks (goroutines, channel
/// operations) a chance to make progress.  Top-level `Value::Future` results
/// are implicitly awaited before the result is returned.
#[wasm_bindgen]
pub struct Repl {
    globals: Arc<GlobalEnv>,
    local: Rc<LocalSet>,
}

/// The result of a single `Repl::eval` call.
#[wasm_bindgen]
pub struct EvalResult {
    /// Text emitted by `print` / `println` / `prn` during evaluation.
    output: String,
    /// The readable (`pr-str`) representation of the last evaluated form,
    /// or an error message when `is_error` is true.
    result: String,
    is_error: bool,
}

#[wasm_bindgen]
impl EvalResult {
    #[wasm_bindgen(getter)]
    pub fn output(&self) -> String {
        self.output.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn result(&self) -> String {
        self.result.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn is_error(&self) -> bool {
        self.is_error
    }
}

impl Default for Repl {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl Repl {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Repl {
        console_error_panic_hook::set_once();
        let globals = cljrs_interp::standard_env_minimal(None, None, None);
        cljrs_stdlib::register(&globals);
        cljrs_dom::set_globals(globals.clone());
        cljrs_dom::register(&globals);

        let local = Rc::new(LocalSet::new());

        // Schedule a persistent LocalSet pump on the browser's microtask queue.
        // All of cljrs_async::init() — including spawn_gc_service() which calls
        // tokio::task::spawn_local — runs from *inside* the LocalSet context here.
        // We must NOT call init() before this block: on WASM, panics are JS
        // exceptions that std::panic::catch_unwind cannot catch, so calling
        // spawn_local outside a LocalSet would propagate an exception to the JS
        // caller and show "failed" in the UI.
        {
            let local2 = local.clone();
            let globals2 = globals.clone();
            wasm_bindgen_futures::spawn_local(async move {
                local2
                    .run_until(async move {
                        cljrs_async::init(&globals2);
                        std::future::pending::<()>().await
                    })
                    .await;
            });
        }

        Repl { globals, local }
    }

    /// Evaluate one or more Clojure forms.
    ///
    /// Runs inside the session's `LocalSet`, so `^:async` functions, `await`,
    /// channels, and `go` all work.  A top-level `Value::Future` result is
    /// implicitly awaited before the readable representation is returned.
    pub async fn eval(&self, input: String) -> EvalResult {
        let local = self.local.clone();
        let globals = self.globals.clone();

        local
            .run_until(async move {
                let forms = match Parser::new(input, "<repl>".to_string()).parse_all() {
                    Ok(f) => f,
                    Err(e) => {
                        return EvalResult {
                            output: String::new(),
                            result: format!("Read error: {e}"),
                            is_error: true,
                        };
                    }
                };

                if forms.is_empty() {
                    return EvalResult {
                        output: String::new(),
                        result: String::new(),
                        is_error: false,
                    };
                }

                push_output_capture();

                let mut env = Env::new(globals.clone(), "user");
                env.is_async = true;

                let mut last_val = Value::Nil;
                let mut error: Option<String> = None;

                for form in &forms {
                    let _frame = cljrs_gc::push_alloc_frame();
                    match eval_form_async(form, &mut env).await {
                        Ok(val) => last_val = val,
                        Err(e) => {
                            error = Some(e.to_string());
                            break;
                        }
                    }
                }

                // Implicitly await a top-level Future/Promise so the user sees
                // the resolved value rather than an opaque future wrapper.
                let final_val = if error.is_none() {
                    match await_value(last_val).await {
                        Ok(v) => v,
                        Err(e) => {
                            error = Some(e.to_string());
                            Value::Nil
                        }
                    }
                } else {
                    last_val
                };

                let output = pop_output_capture().unwrap_or_default();

                match error {
                    Some(msg) => EvalResult {
                        output,
                        result: msg,
                        is_error: true,
                    },
                    None => EvalResult {
                        output,
                        result: pr_str(&globals, final_val),
                        is_error: false,
                    },
                }
            })
            .await
    }
}

fn pr_str(globals: &Arc<GlobalEnv>, val: Value) -> String {
    if let Some(Value::NativeFunction(f)) = globals.lookup_in_ns("clojure.core", "pr-str")
        && let Ok(Value::Str(s)) = (f.get().func)(std::slice::from_ref(&val))
    {
        return s.get().clone();
    }
    format!("{val:?}")
}
