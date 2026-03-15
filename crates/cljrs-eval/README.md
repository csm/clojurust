# cljrs-eval

Tree-walking interpreter for clojurust.  Evaluates `Form` AST nodes produced
by `cljrs-reader` within a namespace-aware lexical environment.

**Phase:** 5–8 + defrecord/reify/built-in protocols + source-path management/require + dynamic variables (`binding`) — implemented.

---

## Purpose

Implements a complete Clojure-compatible tree-walking evaluator including all
special forms, macro expansion, tail-call optimization, sequential and
associative destructuring, syntax-quote with gensyms, lazy sequences, and a
full `clojure.core` bootstrap.

---

## File layout

```
src/
  lib.rs          — module declarations, re-exports, standard_env(), standard_env_with_paths()
  error.rs        — EvalError enum, EvalResult<T> alias
  env.rs          — Frame, GlobalEnv (namespace registry + source paths + loaded set), Env (lexical scope), RequireSpec, RequireRefer
  eval.rs         — top-level eval dispatcher, form_to_value, inline tests
  dynamics.rs     — thread-local dynamic binding stack: BINDING_STACK, BindingGuard, push_frame/pop_frame, deref_var, set_thread_local, capture_current/install_frames, trace_current
  callback.rs     — thread-local eval context for Rust→Clojure callbacks; invoke(f, args) lets builtins call Clojure functions
  apply.rs        — eval_call, apply_value, call_cljrs_fn, ClosureThunk, handle_make_lazy_seq, handle_make_delay, handle_send, handle_vswap, handle_with_bindings, handle_alter_var_root, handle_vary_meta, type_tag_of, resolve_type_tag
  special.rs      — all special form handlers, SPECIAL_FORMS list; includes binding, extract_def_name, compile_meta_form
  loader.rs       — load_ns: resolves namespace names to files, evaluates them, applies alias/refer
  macros.rs       — macroexpand_1, macroexpand, value_to_form
  destructure.rs  — sequential + associative destructuring (bind_pattern, bind_sequential, bind_associative)
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

/// Like standard_env() but also configures the source search paths.
pub fn standard_env_with_paths(source_paths: Vec<PathBuf>) -> Arc<GlobalEnv>;

/// Load a namespace from source and wire alias/refer into current_ns.
pub fn load_ns(globals: Arc<GlobalEnv>, spec: &RequireSpec, current_ns: &str) -> Result<(), String>;

pub enum RequireRefer { None, All, Named(Vec<Arc<str>>) }
pub struct RequireSpec { pub ns: Arc<str>, pub alias: Option<Arc<str>>, pub refer: RequireRefer }

pub struct GlobalEnv { /* namespace registry + source_paths + loaded/loading sets */ }
impl GlobalEnv {
    pub fn new() -> Arc<Self>
    pub fn set_source_paths(&self, paths: Vec<PathBuf>)
    pub fn mark_loaded(&self, ns: &str)
    pub fn is_loaded(&self, ns: &str) -> bool
    pub fn resolve_alias(&self, current_ns: &str, alias: &str) -> Option<Arc<str>>
    pub fn get_or_create_ns(&self, name: &str) -> GcPtr<Namespace>
    pub fn intern(&self, ns: &str, name: Arc<str>, val: Value) -> GcPtr<Var>
    pub fn lookup_var(&self, ns: &str, name: &str) -> Option<GcPtr<Var>>
    pub fn lookup_in_ns(&self, ns: &str, name: &str) -> Option<Value>
    pub fn refer_all(&self, target_ns: &str, source_ns: &str)
    pub fn refer_named(&self, target_ns: &str, source_ns: &str, names: &[Arc<str>])
    pub fn add_alias(&self, current_ns: &str, alias: &str, full_ns: &str)
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
| `letfn` | Mutually recursive local function bindings |
| `loop` / `loop*` + `recur` | Tail-call trampoline |
| `if` / `do` / `and` / `or` | Control flow |
| `quote` | Return unevaluated form |
| `var` | Return `Value::Var` for a namespace var |
| `set!` | Set thread-local binding if inside `binding`; else mutate var's root |
| `throw` / `try` / `catch` / `finally` | Exception handling |
| `ns` | Switch current namespace; processes `:require` clauses; auto-refers `clojure.core` |
| `require` | Load and wire namespaces; supports `:as` alias and `:refer [...]`/`:refer :all` |
| `load-file` | Evaluate a `.cljrs`/`.cljc` file by absolute path |
| `in-ns` | Switch to (or create) a namespace by quoted symbol |
| `alias` | Add a namespace alias to the current namespace |
| `.` | Stub — interop not yet implemented |
| `defprotocol` | Define a protocol with named method signatures |
| `extend-type` | Implement protocol methods for a type |
| `extend-protocol` | Protocol-first sugar for `extend-type` |
| `defmulti` | Define a multimethod with a dispatch function |
| `defmethod` | Add an implementation for one dispatch value |
| `future` | Evaluate body on a new thread; return `Value::Future`; conveys dynamic bindings |
| `defrecord` | Define a named record type; generates `->Name`/`map->Name` constructors; supports inline protocol impls |
| `reify` | Create an anonymous protocol-implementing instance with a gensym'd type tag |
| `binding` | Establish thread-local dynamic var bindings for body (RAII `BindingGuard`) |

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

**Lazy sequences (bootstrap):** `lazy-seq` (macro) `range` `iterate` `repeat`
`repeatedly` `cycle`

**Atoms:** `atom` `deref` `reset!` `swap!` `compare-and-set!`

**Volatiles:** `volatile!` `vreset!` `vswap!` `volatile?`

**Delays:** `delay` (macro) `force` `realized?`

**Promises:** `promise` `deliver`

**Futures:** `future` (special form) `future-done?` `future-cancelled?` `future-cancel`

**Agents:** `agent` `send` `send-off` `await` `agent-error` `restart-agent`

**I/O:** `print` `println` `prn` `pr` `pr-str` `str` `read-string` `slurp` `spit`

**Misc:** `gensym` `type` `hash` `name` `namespace` `ex-info` `ex-data`
`ex-message` `ex-cause`

**Protocols:** `satisfies?` `extends?`

**Multimethods:** `prefer-method` `remove-method` `methods` `isa?`

**Records/reify:** `make-type-instance` `record?` `instance?`

**Dynamic vars:** `var-get` `var-set!` `bound?` `thread-bound?` `meta` `with-meta`
`alter-var-root` `vary-meta` `with-bindings*` (intercepted in `eval_call`)

---

## Lazy sequences

Lazy sequences use a `Thunk` trait (defined in `cljrs-value`) and two new
`Value` variants:

- `Value::LazySeq(GcPtr<LazySeq>)` — a deferred sequence cell; forced at most
  once and cached.
- `Value::Cons(GcPtr<CljxCons>)` — a cons cell with `head: Value` and
  `tail: Value`, used when `cons` is called with a lazy tail.

`make-lazy-seq` (special-cased in `eval_call`) wraps a zero-arg `CljxFn` in a
`ClosureThunk` and returns a `Value::LazySeq`.  The `lazy-seq` macro expands to
`(make-lazy-seq (fn [] ...))`.

---

## Destructuring

Both sequential and associative destructuring are supported in `let`, `fn`,
and `loop` binding positions.

**Sequential** (`[a b & rest :as whole]`): positional bindings, rest, `:as`.

**Associative** (`{:keys [a b] :strs [c] :syms [d] :as m :or {b 99}}`):
keyword, string, and symbol key extraction; entire-value alias; defaults.

---

## Bootstrap HOFs

Higher-order functions that need to call back into the evaluator are defined in
`BOOTSTRAP_SOURCE` — a Clojure string eval'd at startup in `clojure.core`:
`when` `when-not` `cond` `->` `->>` `reduce` `map` `filter` `remove` `keep`
`mapcat` `mapv` `filterv` `take` `drop` `take-while` `drop-while` `some`
`every?` `doseq` `dotimes` `for` `comp` `partial` `complement` `range`
`iterate` `repeat` `repeatedly` `cycle` `update-in` `map-keys` `map-vals` etc.

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-types` | `Span`, `CljxError` |
| `cljrs-gc` | `GcPtr<T>` smart pointer |
| `cljrs-reader` | `Form` AST input |
| `cljrs-value` | `Value`, `CljxFn`, `Namespace`, `LazySeq`, `CljxCons`, all collections |
| `thiserror` | `EvalError` derive |

---

## Deferred to later phases

- `deftype` — blocked by `.` interop (field access via `(.field obj)`); Phase 9
- `.` interop, `new` — Phase 9
- `ref` / STM (`dosync`, `alter`, `commute`, `ensure`) — deferred
- `locking` macro — deferred
- `derive` / full `isa?` hierarchy — deferred
