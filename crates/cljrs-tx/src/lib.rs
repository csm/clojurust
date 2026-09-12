//! Tree-walker-only runtime for pure, isolated transaction functions.

#![cfg(feature = "no-gc")]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use cljrs_reader::{Form, Parser};
use cljrs_runtime::env::env::Env;
use cljrs_runtime::env::error::EvalError;
use cljrs_value::clone::{CloneError, SerializedValue, deserialize, serialize};
use cljrs_value::{Arity, NativeFn, Value, ValueError};

/// A parsed transaction function expression.
///
/// The expression must evaluate to a callable value, normally `(fn [db arg]
/// ...)`. Parsing is intentionally outside the invocation arena: installed
/// transaction code is treated as immutable program data rather than working
/// memory charged on every call.
#[derive(Clone, Debug)]
pub struct TxProgram {
    form: Form,
}

impl TxProgram {
    pub fn parse(source: &str) -> Result<Self, TxError> {
        let mut parser = Parser::new(source.to_string(), "<transaction-function>".to_string());
        let form = parser
            .parse_one()
            .map_err(|error| TxError::Read(Box::new(error)))?
            .ok_or(TxError::EmptyProgram)?;
        if parser
            .parse_one()
            .map_err(|error| TxError::Read(Box::new(error)))?
            .is_some()
        {
            return Err(TxError::MultipleForms);
        }
        Ok(Self { form })
    }

    pub fn form(&self) -> &Form {
        &self.form
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TxLimits {
    /// Managed allocation budget for the invocation arena.
    pub memory_bytes: usize,
    /// Cooperative tree-walker execution credits.
    pub gas: u64,
    /// Nested function applications the invocation may stack. The
    /// tree-walking interpreter consumes real Rust stack per frame, so the
    /// hosting thread's stack must comfortably cover this depth — an
    /// overflow is not catchable.
    pub call_depth: u64,
}

impl Default for TxLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 16 * 1024 * 1024,
            gas: 1_000_000,
            call_depth: 2_000,
        }
    }
}

/// A host-provided function callable from transaction code.
///
/// Arguments and results cross the isolation boundary as pure boundary data:
/// the runtime serializes the cljrs argument values out of the invocation
/// arena, validates them, and validates + deserializes the result back in
/// (charging the invocation's managed-memory budget). Host functions should
/// be deterministic, side-effect-free reads — they run inside the
/// transaction's evaluation and their results become part of it.
///
/// A returned `Err` surfaces inside the transaction as an ordinary
/// evaluation error carrying the message.
pub trait HostFn: Send + Sync {
    /// Invoke the host function with pure boundary-data arguments.
    fn call(&self, args: &[SerializedValue]) -> Result<SerializedValue, String>;
}

impl<F> HostFn for F
where
    F: Fn(&[SerializedValue]) -> Result<SerializedValue, String> + Send + Sync,
{
    fn call(&self, args: &[SerializedValue]) -> Result<SerializedValue, String> {
        self(args)
    }
}

/// A registry of [`HostFn`]s exposed to transaction code as namespaced
/// native functions (e.g. `my.host/lookup`).
///
/// The registry is installed fresh into every invocation's environment by
/// [`execute_with_host`]; the `Arc`ed functions themselves live outside the
/// arena and survive across invocations.
#[derive(Clone, Default)]
pub struct HostApi {
    entries: Vec<HostEntry>,
}

#[derive(Clone)]
struct HostEntry {
    namespace: Arc<str>,
    name: Arc<str>,
    function: Arc<dyn HostFn>,
}

impl HostApi {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `function` as `namespace/name`. Later definitions of the
    /// same name shadow earlier ones.
    pub fn define(
        &mut self,
        namespace: impl Into<Arc<str>>,
        name: impl Into<Arc<str>>,
        function: impl HostFn + 'static,
    ) {
        self.entries.push(HostEntry {
            namespace: namespace.into(),
            name: name.into(),
            function: Arc::new(function),
        });
    }

    /// Whether no host functions are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl std::fmt::Debug for HostApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<String> = self
            .entries
            .iter()
            .map(|entry| format!("{}/{}", entry.namespace, entry.name))
            .collect();
        f.debug_struct("HostApi").field("fns", &names).finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TxError {
    #[error("transaction function source is empty")]
    EmptyProgram,
    #[error("transaction function source must contain exactly one form")]
    MultipleForms,
    #[error("transaction memory limit must be at least 64 bytes")]
    InvalidMemoryLimit,
    #[error("transaction function read error: {0}")]
    Read(Box<cljrs_types::error::CljxError>),
    #[error("transaction input is not isolated pure data: {0}")]
    InvalidInput(&'static str),
    #[error("transaction result is not isolated pure data: {0}")]
    InvalidResult(&'static str),
    #[error("transaction evaluation failed: {0}")]
    Evaluation(String),
    #[error("transaction gas exhausted")]
    GasExhausted,
    #[error("transaction call depth exceeded")]
    DepthExceeded,
    #[error("transaction attempted a forbidden effect: {0}")]
    ForbiddenEffect(String),
    #[error(
        "transaction managed-memory budget exhausted (limit {limit} bytes, used {used}, requested {requested})"
    )]
    MemoryExhausted {
        limit: usize,
        used: usize,
        requested: usize,
    },
    #[error("transaction result cannot cross the isolation boundary: {0}")]
    Clone(#[from] CloneError),
    #[error("transaction runtime panicked")]
    Panic,
}

/// Execute `program` in a fresh GC-less arena and return an owned data value.
///
/// Inputs are structured-cloned into the invocation. The environment, inputs,
/// closures, lazy sequences, local mutable cells, and all intermediate values
/// are destroyed together when execution finishes. The result is cloned out
/// before the arena is freed.
pub fn execute(
    program: &TxProgram,
    args: Vec<SerializedValue>,
    limits: TxLimits,
) -> Result<SerializedValue, TxError> {
    execute_with_host(program, args, limits, &HostApi::default())
}

/// Like [`execute`], but installs the [`HostApi`]'s functions into the
/// invocation environment before evaluating `program`.
///
/// Host functions appear as namespaced natives (`namespace/name`); every
/// value crossing between the arena and a host function is serialized and
/// validated as pure data in both directions.
pub fn execute_with_host(
    program: &TxProgram,
    args: Vec<SerializedValue>,
    limits: TxLimits,
    host: &HostApi,
) -> Result<SerializedValue, TxError> {
    if limits.memory_bytes < 64 {
        return Err(TxError::InvalidMemoryLimit);
    }
    for arg in &args {
        validate_pure_data(arg).map_err(TxError::InvalidInput)?;
    }

    match catch_unwind(AssertUnwindSafe(|| {
        execute_inner(program, args, limits, host)
    })) {
        Ok(result) => result,
        Err(payload) => match payload.downcast_ref::<cljrs_gc::region::RegionLimitExceeded>() {
            Some(exhausted) => Err(TxError::MemoryExhausted {
                limit: exhausted.limit,
                used: exhausted.used,
                requested: exhausted.requested,
            }),
            None => Err(TxError::Panic),
        },
    }
}

fn execute_inner(
    program: &TxProgram,
    args: Vec<SerializedValue>,
    limits: TxLimits,
    host: &HostApi,
) -> Result<SerializedValue, TxError> {
    let invocation = cljrs_gc::alloc_ctx::InvocationGuard::new(limits.memory_bytes);

    // Bootstrap is trusted but is deliberately constructed inside the arena so
    // the complete namespace/Var/function environment dies with the call.
    // `ExecutionMode::NoGcTransaction` enforces the call-depth cap, protecting
    // the hosting thread's Rust stack from unbounded interpreted recursion.
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::NoGcTransaction)
        .build()
        .map_err(|_| TxError::Panic)?
        .into_globals();
    for entry in &host.entries {
        globals.get_or_create_ns(&entry.namespace);
        globals.intern(
            &entry.namespace,
            Arc::clone(&entry.name),
            host_native(entry),
        );
    }
    let mut env = Env::new(globals, "user");
    let args: Vec<_> = args.into_iter().map(deserialize).collect();

    let _policy = cljrs_runtime::env::policy::TransactionPolicyGuard::install();
    let meter = cljrs_runtime::env::gas::GasMeter::new(limits.gas);
    let _gas = cljrs_runtime::env::gas::GasGuard::install(meter);
    let _depth = cljrs_runtime::env::depth::DepthGuard::install(limits.call_depth);

    let function = env.eval(program.form()).map_err(map_eval_error)?;
    let result = cljrs_runtime::env::apply::apply_value(&function, args, &mut env)
        .map_err(map_eval_error)?;
    let result = serialize(&result)?;
    validate_pure_data(&result).map_err(TxError::InvalidResult)?;

    // Make the lifetime ordering explicit: all GcPtrs disappear before the
    // invocation arena is reset by its guard.
    drop(env);
    drop(function);
    let _managed_bytes = invocation.accounted_bytes();
    Ok(result)
}

/// Wraps a host function as an arena-allocated native. The wrapper is the
/// isolation seam: arguments are serialized out of the arena and validated,
/// the result is validated and deserialized back in (arena allocation, so
/// the managed-memory budget covers it).
fn host_native(entry: &HostEntry) -> Value {
    let function = Arc::clone(&entry.function);
    let native = NativeFn::with_closure(
        Arc::clone(&entry.name),
        Arity::Variadic { min: 0 },
        move |args| {
            let args: Vec<SerializedValue> =
                args.iter()
                    .map(serialize)
                    .collect::<Result<_, _>>()
                    .map_err(|error| ValueError::Other(format!("host call argument: {error}")))?;
            for arg in &args {
                validate_pure_data(arg).map_err(|what| {
                    ValueError::Other(format!("host call argument is not pure data: {what}"))
                })?;
            }
            let result = function.call(&args).map_err(ValueError::Other)?;
            validate_pure_data(&result).map_err(|what| {
                ValueError::Other(format!("host result is not pure data: {what}"))
            })?;
            Ok(deserialize(result))
        },
    );
    Value::NativeFunction(cljrs_gc::GcPtr::new(native))
}

fn map_eval_error(error: EvalError) -> TxError {
    match error {
        EvalError::GasExhausted => TxError::GasExhausted,
        EvalError::ForbiddenEffect(operation) => TxError::ForbiddenEffect(operation),
        other => {
            let text = other.to_string();
            // The depth cap surfaces as a runtime error carrying a fixed
            // marker: the interpreter's call path has one error type, so
            // `ExecutionMode::NoGcTransaction` reports the overrun in its text.
            if text.contains(cljrs_runtime::env::depth::DEPTH_EXCEEDED_MSG) {
                TxError::DepthExceeded
            } else {
                TxError::Evaluation(text)
            }
        }
    }
}

fn validate_pure_data(value: &SerializedValue) -> Result<(), &'static str> {
    use SerializedValue::*;
    match value {
        SharedAtom(_) => Err("shared atom"),
        ByteBlob(_) => Err("shared byte blob"),
        Var { .. } => Err("Var"),
        Error(_) => Err("error object"),
        List(items) | Vector(items) | HashSet(items) | SortedSet(items) | Queue(items)
        | ObjectArray(items) => items.iter().try_for_each(validate_pure_data),
        ArrayMap(pairs) | HashMap(pairs) | SortedMap(pairs) => {
            pairs.iter().try_for_each(|(key, value)| {
                validate_pure_data(key).and_then(|()| validate_pure_data(value))
            })
        }
        Cons { head, tail } => validate_pure_data(head).and_then(|()| validate_pure_data(tail)),
        TypeInstance { fields, .. } => fields.iter().try_for_each(|(key, value)| {
            validate_pure_data(key).and_then(|()| validate_pure_data(value))
        }),
        WithMeta { value, meta } => {
            validate_pure_data(value).and_then(|()| validate_pure_data(meta))
        }
        Reduced(inner) => validate_pure_data(inner),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executes_pure_tree_walked_function() {
        let program = TxProgram::parse("(fn [x] (assoc x :seen true))").unwrap();
        let input = SerializedValue::ArrayMap(vec![(
            SerializedValue::Keyword {
                namespace: None,
                name: "id".into(),
            },
            SerializedValue::Long(7),
        )]);
        let result = execute(&program, vec![input], TxLimits::default()).unwrap();
        assert!(matches!(result, SerializedValue::ArrayMap(_)));
    }

    #[test]
    fn permits_internal_closure_that_does_not_escape() {
        let program = TxProgram::parse("(fn [x] (let [f (fn [y] (+ x y))] (f 2)))").unwrap();
        let result = execute(
            &program,
            vec![SerializedValue::Long(40)],
            TxLimits::default(),
        )
        .unwrap();
        assert!(matches!(result, SerializedValue::Long(42)));
    }

    #[test]
    fn rejects_effectful_builtin() {
        let program = TxProgram::parse("(fn [] (spit \"/tmp/cljrs-tx-test\" \"no\"))").unwrap();
        let error = execute(&program, vec![], TxLimits::default()).unwrap_err();
        assert!(matches!(error, TxError::ForbiddenEffect(name) if name == "spit"));
    }

    /// The environment is process-global state the transaction was not handed,
    /// and it can change under a retry, so reading it is denied like any other
    /// ambient effect.
    #[test]
    fn rejects_reading_the_environment() {
        let program = TxProgram::parse("(fn [] (System/getenv \"HOME\"))").unwrap();
        let error = execute(&program, vec![], TxLimits::default()).unwrap_err();
        assert!(matches!(error, TxError::ForbiddenEffect(name) if name == "System/getenv"));
    }

    #[test]
    fn enforces_gas_limit() {
        let program = TxProgram::parse("(fn [] (loop [x 0] (recur (inc x))))").unwrap();
        let error = execute(
            &program,
            vec![],
            TxLimits {
                gas: 100,
                ..TxLimits::default()
            },
        )
        .unwrap_err();
        assert!(matches!(error, TxError::GasExhausted));
    }

    #[test]
    fn enforces_call_depth_limit() {
        let program = TxProgram::parse("(fn [] ((fn boom [n] (boom (inc n))) 0))").unwrap();
        let error = execute(
            &program,
            vec![],
            TxLimits {
                call_depth: 64,
                ..TxLimits::default()
            },
        )
        .unwrap_err();
        assert!(matches!(error, TxError::DepthExceeded), "got {error:?}");
    }

    #[test]
    fn enforces_managed_memory_limit() {
        let program = TxProgram::parse("(fn [] (vec (range 100000)))").unwrap();
        let error = execute(
            &program,
            vec![],
            TxLimits {
                memory_bytes: 256 * 1024,
                gas: 1_000_000,
                ..TxLimits::default()
            },
        )
        .unwrap_err();
        assert!(matches!(error, TxError::MemoryExhausted { .. }));
    }

    #[test]
    fn host_function_is_callable_from_transaction_code() {
        let mut host = HostApi::new();
        host.define(
            "test.host",
            "double",
            |args: &[SerializedValue]| match args {
                [SerializedValue::Long(n)] => Ok(SerializedValue::Long(n * 2)),
                _ => Err("double takes one long".into()),
            },
        );
        let program = TxProgram::parse("(fn [x] (+ 1 (test.host/double x)))").unwrap();
        let result = execute_with_host(
            &program,
            vec![SerializedValue::Long(20)],
            TxLimits::default(),
            &host,
        )
        .unwrap();
        assert!(matches!(result, SerializedValue::Long(41)));
    }

    #[test]
    fn host_function_receives_structured_arguments() {
        let mut host = HostApi::new();
        host.define("test.host", "lookup", |args: &[SerializedValue]| {
            let [SerializedValue::ArrayMap(pairs) | SerializedValue::HashMap(pairs)] = args else {
                return Err("lookup takes one map".into());
            };
            pairs
                .iter()
                .find(|(key, _)| matches!(key, SerializedValue::Keyword { name, .. } if &**name == "id"))
                .map(|(_, value)| value.clone())
                .ok_or_else(|| "missing :id".into())
        });
        let program = TxProgram::parse("(fn [] (test.host/lookup {:id 7}))").unwrap();
        let result = execute_with_host(&program, vec![], TxLimits::default(), &host).unwrap();
        assert!(matches!(result, SerializedValue::Long(7)));
    }

    #[test]
    fn host_function_error_surfaces_as_evaluation_error() {
        let mut host = HostApi::new();
        host.define("test.host", "boom", |_: &[SerializedValue]| {
            Err::<SerializedValue, _>("host exploded".into())
        });
        let program = TxProgram::parse("(fn [] (test.host/boom))").unwrap();
        let error = execute_with_host(&program, vec![], TxLimits::default(), &host).unwrap_err();
        assert!(
            matches!(&error, TxError::Evaluation(text) if text.contains("host exploded")),
            "got {error:?}"
        );
    }

    #[test]
    fn impure_host_result_is_rejected() {
        let mut host = HostApi::new();
        host.define("test.host", "leak", |_: &[SerializedValue]| {
            Ok(SerializedValue::ByteBlob(Arc::from(&b"shared"[..])))
        });
        let program = TxProgram::parse("(fn [] (test.host/leak))").unwrap();
        let error = execute_with_host(&program, vec![], TxLimits::default(), &host).unwrap_err();
        assert!(
            matches!(&error, TxError::Evaluation(text) if text.contains("not pure data")),
            "got {error:?}"
        );
    }

    #[test]
    fn oversized_host_result_hits_memory_budget() {
        let mut host = HostApi::new();
        host.define("test.host", "flood", |_: &[SerializedValue]| {
            Ok(SerializedValue::Vector(
                (0..100_000).map(SerializedValue::Long).collect(),
            ))
        });
        let program = TxProgram::parse("(fn [] (count (test.host/flood)))").unwrap();
        let error = execute_with_host(
            &program,
            vec![],
            TxLimits {
                memory_bytes: 64 * 1024,
                gas: 10_000_000,
                ..TxLimits::default()
            },
            &host,
        )
        .unwrap_err();
        assert!(
            matches!(error, TxError::MemoryExhausted { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn host_function_result_flows_into_transaction_data() {
        let mut host = HostApi::new();
        host.define(
            "test.host",
            "balance",
            |args: &[SerializedValue]| match args {
                [SerializedValue::Str(name)] if name == "alice" => Ok(SerializedValue::Long(100)),
                _ => Err("unknown account".into()),
            },
        );
        let program = TxProgram::parse(
            "(fn [acct amount] [[:db/add acct :balance (+ (test.host/balance acct) amount)]])",
        )
        .unwrap();
        let result = execute_with_host(
            &program,
            vec![
                SerializedValue::Str("alice".into()),
                SerializedValue::Long(5),
            ],
            TxLimits::default(),
            &host,
        )
        .unwrap();
        let SerializedValue::Vector(forms) = &result else {
            panic!("expected tx data vector, got {result:?}");
        };
        let SerializedValue::Vector(form) = &forms[0] else {
            panic!("expected inner vector");
        };
        assert!(matches!(&form[0], SerializedValue::Keyword { name, .. } if &**name == "add"));
        assert!(matches!(form[3], SerializedValue::Long(105)));
    }

    #[test]
    fn syntax_quote_gensyms_are_deterministic_per_invocation() {
        let program = TxProgram::parse("(fn [] `local#)").unwrap();
        let first = execute(&program, vec![], TxLimits::default()).unwrap();
        let second = execute(&program, vec![], TxLimits::default()).unwrap();
        let symbol_name = |value: &SerializedValue| match value {
            SerializedValue::Symbol { name, .. } => name.to_string(),
            other => panic!("expected symbol, got {other:?}"),
        };
        assert_eq!(symbol_name(&first), symbol_name(&second));
    }
}
