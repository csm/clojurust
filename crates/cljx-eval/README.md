# cljx-eval

Tree-walking interpreter for clojurust.  Evaluates `Form` AST nodes produced
by `cljx-reader` within a namespace-aware lexical environment.

**Phase:** 4 — implemented.

---

## Purpose

Implements a complete Clojure-compatible tree-walking evaluator including all
special forms, macro expansion, tail-call optimization, sequential
destructuring, syntax-quote with gensyms, and a full `clojure.core` bootstrap.

---

## File layout

```
src/
  lib.rs          — module declarations, re-exports, standard_env()
  error.rs        — EvalError enum, EvalResult<T> alias
  env.rs          — Frame, GlobalEnv (namespace registry), Env (lexical scope)
  eval.rs         — top-level eval dispatcher, form_to_value, inline tests
  apply.rs        — eval_call, apply_value, call_cljx_fn, select_arity
  special.rs      — all special form handlers, SPECIAL_FORMS list
  macros.rs       — macroexpand_1, macroexpand, value_to_form
  destructure.rs  — sequential destructuring (bind_pattern, bind_sequential)
  syntax_quote.rs — syntax-quote expander with gensym counter
  builtins.rs     — native built-in functions + BOOTSTRAP_SOURCE Clojure string
```

---

## Public API

```rust
/// Evaluate a Form in env and return the resulting Value.
pub fn eval(form: &Form, env: &mut Env) -> EvalResult;

/// Create a GlobalEnv pre-populated with clojure.core builtins
/// (native functions + bootstrap HOFs) and a user namespace
/// that refers everything from clojure.core.
pub fn standard_env() -> Arc<GlobalEnv>;

pub struct GlobalEnv { /* namespace registry */ }
impl GlobalEnv {
    pub fn new() -> Arc<Self>
    pub fn get_or_create_ns(&self, name: &str) -> GcPtr<Namespace>
    pub fn intern(&self, ns: &str, name: Arc<str>, val: Value) -> GcPtr<Var>
    pub fn lookup_var(&self, ns: &str, name: &str) -> Option<GcPtr<Var>>
    pub fn lookup_in_ns(&self, ns: &str, name: &str) -> Option<Value>
    pub fn refer_all(&self, target_ns: &str, source_ns: &str)
}

pub struct Env { /* frames + current_ns + globals */ }
impl Env {
    pub fn new(globals: Arc<GlobalEnv>, ns: &str) -> Self
    pub fn with_closure(globals: Arc<GlobalEnv>, ns: &str, f: &CljxFn) -> Self
    pub fn push_frame(&mut self)
    pub fn pop_frame(&mut self)
    pub fn bind(&mut self, name: Arc<str>, val: Value)
    pub fn lookup(&self, name: &str) -> Option<Value>
}

#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    Runtime(String),
    UnboundSymbol(String),
    Arity { name: String, expected: String, got: usize },
    NotCallable(String),
    Thrown(Value),         // ex-info / throw
    Read(CljxError),       // from reader
    // internal only:
    Recur(Vec<Value>),     // tail-call signal
}

pub type EvalResult<T = Value> = Result<T, EvalError>;
```

---

## Special forms

| Form | Notes |
|------|-------|
| `def` / `defn` / `defmacro` / `defonce` | Intern values in current namespace |
| `fn` / `fn*` | Multi-arity; rest args; closure capture |
| `let` / `let*` | Sequential binding with destructuring |
| `loop` / `loop*` + `recur` | Tail-call trampoline |
| `if` / `do` / `and` / `or` | Control flow |
| `quote` | Return unevaluated form |
| `var` | Return `Value::Var` for a namespace var |
| `set!` | Mutate a var's root binding |
| `throw` / `try` / `catch` / `finally` | Exception handling |
| `ns` | Switch current namespace (basic) |
| `.` | Stub — interop not yet implemented |

---

## Built-in functions (`clojure.core`)

All built-ins have signature `fn(&[Value]) -> ValueResult<Value>`.

**Arithmetic:** `+` `-` `*` `/` `mod` `rem` `quot` `inc` `dec` `max` `min` `abs`

**Comparison:** `=` `not=` `<` `>` `<=` `>=` `identical?`

**Predicates:** `nil?` `zero?` `pos?` `neg?` `not` `true?` `false?` `number?`
`integer?` `float?` `string?` `keyword?` `symbol?` `fn?` `seq?` `map?`
`vector?` `set?` `coll?` `boolean?` `char?` `var?` `atom?` `empty?`

**Collections:** `list` `vector` `hash-map` `hash-set` `conj` `assoc` `dissoc`
`get` `get-in` `count` `seq` `first` `rest` `next` `cons` `nth` `last`
`reverse` `concat` `keys` `vals` `contains?` `merge` `into` `empty` `vec` `set`

**Higher-order (bootstrap):** `map` `filter` `reduce` `keep` `remove` `mapcat`
`take` `drop` `take-while` `drop-while` `comp` `partial` `identity`
`constantly` `complement` `apply`

**Atoms:** `atom` `deref` `reset!` `swap!`

**I/O:** `print` `println` `prn` `pr` `pr-str` `str` `read-string`

**Misc:** `gensym` `type` `hash` `name` `namespace` `ex-info` `ex-data`
`ex-message` `ex-cause`

---

## Bootstrap HOFs

Higher-order functions that need to call back into the evaluator are defined in
`BOOTSTRAP_SOURCE` — a Clojure string eval'd at startup in `clojure.core`:
`when` `when-not` `cond` `->` `->>` `reduce` `map` `filter` `remove` `keep`
`mapcat` `mapv` `filterv` `take` `drop` `take-while` `drop-while` `some`
`every?` `doseq` `dotimes` `for` `comp` `partial` `complement` etc.

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljx-types` | `Span`, `CljxError` |
| `cljx-gc` | `GcPtr<T>` smart pointer |
| `cljx-reader` | `Form` AST input |
| `cljx-value` | `Value`, `CljxFn`, `Namespace`, all collections |
| `thiserror` | `EvalError` derive |

---

## Deferred to later phases

- Map / nested destructuring — Phase 5
- `letfn` — Phase 5
- Lazy sequences (`range`, `lazy-seq`) — Phase 5
- Full `ns` / `require` / `use` / `alias` / `refer` — Phase 5
- `.` interop, `new` — Phase 9
- `monitor-enter` / `monitor-exit` — Phase 7
