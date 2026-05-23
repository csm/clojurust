use std::sync::Arc;
use wasm_bindgen::prelude::*;

use cljrs_builtins::builtins::{pop_output_capture, push_output_capture};
use cljrs_env::env::{Env, GlobalEnv};
use cljrs_env::error::EvalError;
use cljrs_interp::eval::eval;
use cljrs_reader::Parser;
use cljrs_value::Value;

/// A stateful Clojure REPL.  The `GlobalEnv` holds all defined vars across
/// calls; a fresh `Env` (local frame stack) is created per `eval` call.
#[wasm_bindgen]
pub struct Repl {
    globals: Arc<GlobalEnv>,
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

#[wasm_bindgen]
impl Repl {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Repl {
        console_error_panic_hook::set_once();
        let globals = cljrs_interp::standard_env_minimal(None, None, None);
        Repl { globals }
    }

    /// Evaluate one or more Clojure forms.  Returns captured print output and
    /// the `pr-str` of the last form's value (or an error message).
    pub fn eval(&self, input: &str) -> EvalResult {
        let mut env = Env::new(self.globals.clone(), "user");

        let forms = match Parser::new(input.to_string(), "<repl>".to_string()).parse_all() {
            Ok(f) => f,
            Err(e) => {
                return EvalResult {
                    output: String::new(),
                    result: format!("Read error: {e}"),
                    is_error: true,
                }
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

        let mut last_val = Value::Nil;
        let mut error: Option<String> = None;

        for form in &forms {
            let _frame = cljrs_gc::push_alloc_frame();
            match eval(form, &mut env) {
                Ok(val) => last_val = val,
                Err(e) => {
                    error = Some(format_error(e));
                    break;
                }
            }
        }

        let output = pop_output_capture().unwrap_or_default();

        match error {
            Some(msg) => EvalResult {
                output,
                result: msg,
                is_error: true,
            },
            None => EvalResult {
                output,
                result: pr_str(&self.globals, last_val),
                is_error: false,
            },
        }
    }
}

fn format_error(e: EvalError) -> String {
    e.to_string()
}

fn pr_str(globals: &Arc<GlobalEnv>, val: Value) -> String {
    if let Some(Value::NativeFunction(f)) = globals.lookup_in_ns("clojure.core", "pr-str") {
        if let Ok(Value::Str(s)) = (f.get().func)(&[val.clone()]) {
            return s.get().clone();
        }
    }
    format!("{val:?}")
}
