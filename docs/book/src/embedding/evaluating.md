# Evaluating code

Evaluation is two steps the host drives itself: parse source text into forms,
then evaluate each form in an `Env`. There is no single `eval_string` entry
point, because hosts differ in how they want to handle allocation frames,
errors, and partial results — but the loop is a dozen lines.

## Parse and evaluate

```rust
use cljrs_reader::Parser;
use cljrs_runtime::tiered::{Env, EvalError, eval};
use cljrs_value::Value;

fn eval_str(env: &mut Env, src: &str, origin: &str) -> Result<Value, EvalError> {
    let mut parser = Parser::new(src.to_string(), origin.to_string());
    let forms = parser.parse_all().map_err(EvalError::Read)?;

    let mut result = Value::Nil;
    for form in &forms {
        let _frame = cljrs_gc::push_alloc_frame();
        result = eval(form, env)?;
    }
    Ok(result)
}
```

`origin` is the filename the reader attaches to source spans — it shows up in
error messages, so give it something meaningful (`"<config>"`, the real path, a
REPL input counter).

`parse_all` reads **every** form up front, so a syntax error anywhere means
nothing is evaluated. To get REPL-style behaviour (evaluate what parses, report
the rest), parse and evaluate one form at a time.

An `Env` is a lexical scope chained to the shared `GlobalEnv`. Make one with
`runtime.env("user")` or `Env::new(globals.clone(), "user")`. It is cheap: one
per request, per session, or per task is normal. Re-using an `Env` across calls
preserves whatever `def`s the guest code made — those live in the namespace, not
the `Env` — while giving you a fresh set of local bindings.

## Getting values out

`Value` is the runtime's tagged union. Three ways to read one:

```rust
// 1. Display — prints readably, the way `pr-str` does.
assert_eq!(value.to_string(), "\"hello, world\"");   // note the quotes

// 2. Marshalling, for the common scalar types.
use cljrs_interop::FromValue;
let s = String::from_value(&value)?;                  // "hello, world"
let n = i64::from_value(&count)?;

// 3. Matching, when you need the exact shape.
match &value {
    Value::Str(s) => println!("{}", s.get()),
    Value::Long(n) => println!("{n}"),
    Value::Nil => println!("nothing"),
    other => return Err(format!("unexpected: {other}")),
}
```

`FromValue`/`IntoValue` are implemented for `()`, `bool`, `i64`, `f64`,
`String`, `&str`, `BigInt`, `Option<T>`, `Vec<Value>`, and `Value`. Collections
come back as `Value::Vector` / `Value::Map` / `Value::Set` and are read through
their persistent-collection APIs.

Remember that `Display` is the *readable* representation: strings keep their
quotes, characters print as `\a`. Use `FromValue` or a match when you want the
underlying data rather than something to show a user.

## Calling Clojure from Rust

Look the function up in its namespace, then apply it:

```rust
use cljrs_runtime::env::apply::apply_value;
use cljrs_runtime::env::gc_roots::root_value;

eval_str(&mut env, "(defn greet [who] (str \"hello, \" who))", "<host>")?;

let f = globals.lookup_in_ns("user", "greet").expect("greet is defined");
let _root = root_value(&f);                    // f is on the Rust stack — root it

let arg = Value::Str(cljrs_gc::GcPtr::new("world".to_string()));
let out = apply_value(&f, vec![arg], &mut env)?;
```

`apply_value` handles every callee shape Clojure allows — functions, keywords,
maps, sets, vars, protocol methods, multimethods — and roots the callee and
arguments across the safepoint it takes on entry.

Inside a **native function** you usually do not have an `Env` in hand. Use
`invoke` instead, which picks up the eval context that the interpreter pushes
around every native call:

```rust
use cljrs_runtime::tiered::invoke;

// e.g. a comparator handed to your native sort
let ordering = invoke(&user_fn, vec![a.clone(), b.clone()])?;
```

`invoke` fails with "invoke called outside eval context" if there is no active
evaluation on the thread. If you spawn a thread and want to call Clojure from
it — which only makes sense for a runtime confined to that thread — install a
context first with
`cljrs_runtime::env::callback::install_eval_context_guard(globals, ns)`.

## Errors

```rust
pub enum EvalError {
    Thrown(Value),               // (throw ...) — the thrown value, often an error map
    UnboundSymbol(String),
    Arity { name: String, expected: String, got: usize },
    NotCallable(String),
    Runtime(String),
    Read(CljxError),             // reader/parse failure, carries file/line/col
    GasExhausted,                // the execution-credit budget ran out
    ForbiddenEffect(String),     // a capability denied by the transaction policy
    Recur(Vec<Value>),           // `recur` outside a loop — never escapes normal code
    CommitSignatureVerificationFailed { commit: String, reason: String },
}
```

Guest exceptions arrive as `Thrown`, carrying the thrown value; the rest are the
evaluator's own failures. A host that reports to users will want to map these to
its own error type — `EvalError::to_error_value` converts one into a Clojure
error *value* if you would rather hand the failure back to guest code than
propagate it into Rust.

Reader errors keep their source spans, so `miette` renders them with the
offending line highlighted if your host uses it. The clojurust library crates
depend on miette for its *types* only — `Diagnostic`, `SourceSpan`,
`NamedSource` — and deliberately do not enable its `fancy` feature, so
embedding the runtime does not drag in a terminal renderer (`owo-colors`,
`supports-color`, `terminal_size`, `textwrap`, `backtrace`). If you want that
rendering, enable it on your own dependency:

```toml
miette = { version = "7", features = ["fancy"] }
```

## GC discipline

The collector traces from a fixed set of roots. Three cases cover a host:

**Values reachable from a namespace are safe.** Anything `def`'d, and anything
you intern yourself, is traced through the namespace table (assuming
[`register_gc_roots`](runtime-builder.md#register_gc_roots) is on, which is the
default). This is the right home for anything the host holds for a long time:

```rust
globals.intern("acme.host", "session-id".into(), session_id_value);
```

**Values you allocate while evaluating need an allocation frame.** That is what
`push_alloc_frame()` in the eval loop is for: everything allocated inside the
frame is rooted until the returned guard drops. One frame per top-level form is
the convention the CLI and the bootstrap both use — it keeps a long-running
session from accumulating roots while still protecting the form being evaluated.

**Values living on the Rust stack across a safepoint need an explicit root.**

```rust
use cljrs_runtime::env::gc_roots::{root_value, root_values};

let _r1 = root_value(&callee);     // one Value
let _r2 = root_values(&args);      // a slice
```

Both return RAII guards that unregister on drop. Forgetting one is the classic
embedding bug: it works until a collection happens to land inside the call.

`cljrs_runtime::tiered::force_collect(&env)` triggers an immediate collection,
which is worth doing after you tear down namespaces so their closures and form
trees are released before the next load.

## Threads and runtimes

`Runtime`, `GlobalEnv`, `Env`, and `Value` are all `!Send`, and the compiler
enforces it. The rules that fall out:

- **Build the runtime on the thread that will use it**, and evaluate only on
  that thread. Each such thread owns an independent GC heap.
- **To use several cores, run several runtimes** — one per thread — and move
  data between them with isolate channels (a deep copy) or `shared-atom` (a
  refcounted shared cell). Closures and stateful objects cannot cross. See
  [Worker isolation](../async-io/isolation.md).
- **Send work to the interpreter thread as plain data.** This is exactly what
  the nREPL server does: its network thread packages each request as `Send`
  strings plus a reply channel and hands it to the interpreter thread over an
  mpsc channel.
- **Byte-level work does not need a runtime.** Compression, hashing, TLS, socket
  traffic — anything touching no Clojure values — can run on an ordinary thread
  pool and hand results back as `Vec<u8>`.
