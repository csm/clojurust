# Plan: clojure.spec Integration into IR Lowering

## Background and Goals

The IR lowering pipeline (`cljrs-ir` + `cljrs-eval/src/lower.rs` + `anf.cljrs`) currently knows nothing about the types or shapes of values flowing through a function. `clojure.spec` registrations are declarations of exactly this information — `:args`/`:ret` specs on `s/fdef` tell us what a function accepts and produces; `s/keys` tells us a map has specific required keys; `s/tuple` tells us a vector has a fixed arity and positional types.

The plan has five phases:
- **Phase 1** — implement `clojure.spec.alpha` as a Clojure library
- **Phase 2** — extend the IR types and serialized format with spec annotations
- **Phase 3** — integrate spec queries into the ANF lowering pass
- **Phase 4** — a new spec-propagation optimization pass (Rust side)
- **Phase 5** — optional runtime spec-check instructions for unresolvable specs

---

## Phase 1 — Implement `clojure.spec.alpha`

### 1.1 Spec Data Model

Specs are first-class values stored in a global registry. Represent every spec as a tagged map so the Clojure compiler can inspect them structurally.

```clojure
;; Every spec is one of these map shapes:
{:spec/type :pred   :pred fn?}
{:spec/type :and    :specs [...]}
{:spec/type :or     :keys [...] :specs [...]}
{:spec/type :keys   :req [...] :opt [...] :req-un [...] :opt-un [...]}
{:spec/type :coll-of :spec ... :kind nil :min-count nil :max-count nil}
{:spec/type :map-of  :kspec ... :vspec ...}
{:spec/type :tuple   :specs [...]}
{:spec/type :cat     :keys [...] :specs [...]}
{:spec/type :alt     :keys [...] :specs [...]}
{:spec/type :merge   :specs [...]}
{:spec/type :nilable :spec ...}
{:spec/type :every   :spec ... :opts {...}}
{:spec/type :fspec   :args ... :ret ... :fn ...}
{:spec/type :multi   :dispatch-fn ...}
{:spec/type :ref     :name kw}
```

This is the canonical internal form. The public `s/def` macro normalizes any input (predicate fn, set, regex op, etc.) into one of these before storing it.

### 1.2 Registry

```clojure
;; In clojure.spec.alpha
(def ^:private registry (atom {}))       ; keyword → spec-map
(def ^:private fdef-registry (atom {}))  ; qualified-symbol → {:args :ret :fn}

(defn registry [] @registry)
(defn get-spec [kw] (get @registry kw))
(defn def* [kw spec] (swap! registry assoc kw (normalize spec)) kw)
```

`normalize` converts a predicate function to `{:spec/type :pred :pred f}`, a set to a membership predicate, a keyword to a `{:spec/type :ref :name kw}`, or passes through an already-normalized map.

### 1.3 Public API

Implement in `cljrs-stdlib/src/clojure/spec/alpha.cljrs`:

| Form | Description |
|---|---|
| `s/def` | Register a spec for a keyword name |
| `s/fdef` | Register `:args`/`:ret`/`:fn` specs for a function var |
| `s/valid?` | Boolean: does value conform? |
| `s/conform` | Conform and return tagged value, or `:clojure.spec.alpha/invalid` |
| `s/explain` | Human-readable explanation of failure |
| `s/explain-data` | Machine-readable failure map `{:clojure.spec.alpha/problems [...]}` |
| `s/assert` | Conform or throw `ex-info` |
| `s/instrument` | Wrap a var's fn to check `:args` on every call |
| `s/unstrument` | Remove instrumentation |
| `s/and`, `s/or` | Combinators |
| `s/keys`, `s/map-of` | Map specs |
| `s/coll-of`, `s/every` | Collection specs |
| `s/tuple` | Fixed-arity vector spec |
| `s/cat`, `s/alt` | Sequential regex specs |
| `s/nilable` | Nil-or-spec |
| `s/merge` | Merge map specs |
| `s/get-spec` | Look up by keyword |

Conformance is a recursive walk of the spec map against the value. The interpreter for `s/cat`/`s/alt` handles sequential patterns and is essentially a small regex NFA.

### 1.4 `s/fdef` Schema

```clojure
(s/fdef clojure.core/+
  :args (s/cat :nums (s/* number?))
  :ret  number?)

;; Stored as:
{:args {:spec/type :cat :keys [:nums]
        :specs [{:spec/type :coll-of :spec {:spec/type :pred :pred number?}}]}
 :ret  {:spec/type :pred :pred number?}
 :fn   nil}
```

The fdef registry is keyed by the fully-qualified symbol (not keyword), to allow lookup during lowering given a call target.

---

## Phase 2 — IR Format Extensions

### 2.1 New Rust Types in `cljrs-ir`

Add a new file `cljrs-ir/src/spec.rs`:

```rust
/// Static type constraint derived from spec analysis.
/// Stored on VarIds in the IR; no runtime cost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SpecConstraint {
    Top,                                   // unconstrained (default)
    Bottom,                                // unreachable / contradiction
    Prim(PrimType),                        // concrete Clojure type
    Nullable(Box<SpecConstraint>),         // nil | inner
    MapShape(MapShapeConstraint),          // known key presence
    VecShape(VecShapeConstraint),          // fixed arity or uniform element type
    Intersection(Vec<SpecConstraint>),     // s/and (meet)
    Union(Vec<SpecConstraint>),            // s/or (join)
    UserSpec(Arc<str>),                    // opaque registered spec name
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrimType {
    Nil, Bool, Long, Double, Str,
    Keyword, Symbol, List, Vector,
    Map, Set, Seq, Fn, Any,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapShapeConstraint {
    pub required_keys: Vec<Arc<str>>,
    pub optional_keys: Vec<Arc<str>>,
    pub value_spec:    Option<Box<SpecConstraint>>,  // for map-of
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecShapeConstraint {
    pub min_count:  Option<u32>,
    pub max_count:  Option<u32>,
    pub positional: Vec<SpecConstraint>,              // s/tuple positional types
    pub uniform:    Option<Box<SpecConstraint>>,      // s/coll-of element type
}

/// Spec for a function's args / return value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FnSpec {
    pub args: Option<Arc<str>>,  // registered name of :args spec
    pub ret:  Option<Arc<str>>,  // registered name of :ret spec
    pub fn_:  Option<Arc<str>>,  // registered name of :fn spec
}

/// What the runtime should do when a SpecCheck fails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpecFailMode {
    Throw,  // throw ex-info (s/assert semantics)
    Warn,   // emit warning, continue (instrument default)
}
```

### 2.2 New `Inst` Variants

Add to the `Inst` enum in `cljrs-ir/src/lib.rs`:

```rust
/// Pure annotation: records a statically-derived spec constraint on a var.
/// No runtime effect; used by optimization passes and the JIT.
SpecAnnotate {
    var:        VarId,
    constraint: SpecConstraint,
},

/// Runtime spec check. `result` receives the conformed value (or the
/// original if the spec has no conforming transform). Throws / warns on
/// failure according to `fail_mode`.
SpecCheck {
    result:    VarId,
    val:       VarId,
    spec_name: Arc<str>,
    fail_mode: SpecFailMode,
},
```

`SpecAnnotate` is zero-cost at runtime — the interpreter skips it, the JIT reads it as metadata. `SpecCheck` is only emitted when a runtime check is actually needed.

### 2.3 `IrFunction` Extensions

```rust
pub struct IrFunction {
    // ... existing fields ...

    /// Spec constraints inferred or declared for each VarId.
    /// Sparse: only vars with non-Top constraints appear here.
    pub spec_annotations: HashMap<u32, SpecConstraint>,

    /// The fdef spec for this function, if one was registered.
    pub fn_spec: Option<FnSpec>,
}
```

`spec_annotations` is populated by both the lowering pass (from fdef / literal type info) and the propagation pass (from flow analysis). It is the authoritative source of constraint information for later JIT passes.

### 2.4 Serialization Versioning

The IR format is `postcard`-based. Add a `FORMAT_VERSION: u32` constant to `IrBundle`. Bump the version whenever new variants are added. On deserialization, check the version and either:

- Accept and deserialize (current or known-compatible version)
- Return a `BundleError::VersionMismatch { found, expected }` so the caller can re-lower from source instead of crashing

Because `postcard` uses compact integer encoding for enum discriminants, adding new variants at the end of `Inst` is backward-compatible for reading old bundles; new bundles are not readable by old binaries (which is fine — old binaries re-lower).

---

## Phase 3 — Spec-Aware ANF Lowering

The Clojure-side lowering pass (`cljrs-ir/src/clojure/compiler/anf.cljrs`) is extended to query the spec registry at lowering time. The Rust orchestrator (`cljrs-eval/src/lower.rs`) needs no structural changes.

### 3.1 Spec Query Helpers

Add a new namespace `cljrs.compiler.spec-query` loaded alongside `cljrs.compiler.anf`:

```clojure
(ns cljrs.compiler.spec-query
  (:require [clojure.spec.alpha :as s]))

(defn fdef-for
  "Returns the fdef map {:args :ret :fn} for a fully-qualified symbol, or nil."
  [qualified-sym]
  (get @s/fdef-registry qualified-sym))

(defn constraint-for-spec
  "Converts a normalized spec map to an IR SpecConstraint keyword map.
   Returns nil if the spec cannot be statically represented."
  [spec]
  (when spec
    (case (:spec/type spec)
      :pred    (pred->constraint (:pred spec))
      :and     {:constraint/type :intersection
                :specs (mapv constraint-for-spec (:specs spec))}
      :or      {:constraint/type :union
                :specs (mapv constraint-for-spec (:specs spec))}
      :keys    {:constraint/type :map-shape
                :required-keys (mapv name (concat (:req spec) (:req-un spec)))
                :optional-keys (mapv name (concat (:opt spec) (:opt-un spec)))}
      :tuple   {:constraint/type :vec-shape
                :positional (mapv constraint-for-spec (:specs spec))
                :min-count  (count (:specs spec))
                :max-count  (count (:specs spec))}
      :coll-of {:constraint/type :vec-shape
                :uniform (constraint-for-spec (:spec spec))}
      :nilable {:constraint/type :nullable
                :inner (constraint-for-spec (:spec spec))}
      :ref     (constraint-for-spec (s/get-spec (:name spec)))
      {:constraint/type :user-spec :name (str (:spec/type spec))})))

(defn- pred->constraint [pred-fn]
  (condp = pred-fn
    int?      {:constraint/type :prim :prim :long}
    integer?  {:constraint/type :prim :prim :long}
    float?    {:constraint/type :prim :prim :double}
    number?   {:constraint/type :union
               :specs [{:constraint/type :prim :prim :long}
                       {:constraint/type :prim :prim :double}]}
    string?   {:constraint/type :prim :prim :str}
    keyword?  {:constraint/type :prim :prim :keyword}
    symbol?   {:constraint/type :prim :prim :symbol}
    boolean?  {:constraint/type :prim :prim :bool}
    nil?      {:constraint/type :prim :prim :nil}
    map?      {:constraint/type :prim :prim :map}
    vector?   {:constraint/type :prim :prim :vector}
    seq?      {:constraint/type :prim :prim :seq}
    fn?       {:constraint/type :prim :prim :fn}
    nil))
```

### 3.2 Call-Site Annotation

When lowering a `Call` to a known symbol, annotate the result var using the fdef `:ret` spec:

```clojure
(defn- lower-call [ctx callee-sym args]
  (let [fdef    (spec-query/fdef-for callee-sym)
        ret-var (fresh-var ctx)]
    (emit ctx (call-inst callee-sym args ret-var))
    (when-let [ret-constraint (some-> fdef :ret spec-query/constraint-for-spec)]
      (emit ctx {:inst/type  :spec-annotate
                 :var        ret-var
                 :constraint ret-constraint}))
    ret-var))
```

### 3.3 Parameter Annotation at Function Entry

When lowering a `fn*` body, if the enclosing function has a registered fdef with an `:args` spec, annotate each parameter VarId immediately after binding it:

```clojure
(defn lower-fn-body [fname ns params body-forms]
  (let [ctx      (fresh-ctx)
        fdef     (spec-query/fdef-for (qualify ns fname))
        arg-spec (when fdef (s/get-spec (:args fdef)))]
    (doseq [[i param] (map-indexed vector params)]
      (let [var-id (bind-param ctx param)]
        (when-let [constraint (some-> arg-spec
                                      (nth-cat-spec i)
                                      spec-query/constraint-for-spec)]
          (emit ctx {:inst/type  :spec-annotate
                     :var        var-id
                     :constraint constraint}))))
    ...))
```

`nth-cat-spec` extracts the i-th positional spec from a `s/cat` sequence spec.

### 3.4 Literal Value Annotation

When lowering a literal constant, emit a `SpecAnnotate` immediately after:

```clojure
(defn lower-const [ctx form]
  (let [var-id     (fresh-var ctx)
        constraint (literal->constraint form)]
    (emit ctx {:inst/type :const :var var-id :value form})
    (when constraint
      (emit ctx {:inst/type :spec-annotate :var var-id :constraint constraint}))
    var-id))

(defn- literal->constraint [form]
  (cond
    (int? form)     {:constraint/type :prim :prim :long}
    (float? form)   {:constraint/type :prim :prim :double}
    (string? form)  {:constraint/type :prim :prim :str}
    (keyword? form) {:constraint/type :prim :prim :keyword}
    (nil? form)     {:constraint/type :prim :prim :nil}
    (boolean? form) {:constraint/type :prim :prim :bool}
    (vector? form)  {:constraint/type :vec-shape
                     :min-count (count form) :max-count (count form)}
    (map? form)     {:constraint/type :map-shape
                     :required-keys (mapv str (keys form))
                     :optional-keys []}
    :else nil))
```

### 3.5 Conditional Narrowing

When lowering `(if test then else)`, if `test` is a call to a known predicate applied to a var, annotate that var with the narrowed constraint in the `then` branch and the negated constraint in the `else` branch:

```clojure
(defn lower-if [ctx test-form then-form else-form]
  (let [test-var              (lower-expr ctx test-form)
        [narrowed negated]    (infer-narrowing ctx test-form)]
    (in-block ctx :then-block
      (when narrowed
        (emit ctx {:inst/type  :spec-annotate
                   :var        (:narrowed-var narrowed)
                   :constraint (:constraint narrowed)}))
      (lower-expr ctx then-form))
    (in-block ctx :else-block
      (when negated
        (emit ctx {:inst/type  :spec-annotate
                   :var        (:narrowed-var negated)
                   :constraint (:constraint negated)}))
      (lower-expr ctx else-form))
    ...))
```

---

## Phase 4 — Spec Propagation Optimization Pass (Rust Side)

A new optimization pass in `cljrs-compiler` (or a new sub-crate `cljrs-opt`), run on an `IrFunction` after lowering.

### 4.1 Constraint Lattice

```
        Top (unknown)
       / | \ ...
    Long Double Str  Map  Vec  ...   (concrete prims)
       \   |   /
        Nullable(X)    Union([X,Y])    Intersection([X,Y])
              \          /
              Bottom (contradiction)
```

Operations:
- `meet(a, b)` — greatest lower bound (used at branch join / phi): `Intersection` if neither subsumes the other
- `join(a, b)` — least upper bound (used for union types): `Union` if neither subsumes the other
- `subsumes(a, b)` — true if every value satisfying `b` also satisfies `a`

### 4.2 Dataflow Analysis

Run forward dataflow over the CFG in reverse-postorder:

```
entry: collect SpecAnnotate instructions from params → initial constraint map
for each block (RPO order):
  for each phi:
    constraint[result] = meet(constraint[each incoming var])
  for each inst:
    SpecAnnotate { var, constraint } →
      constraint[var] = meet(constraint[var], constraint)
    CallKnown { result, fn, args } →
      constraint[result] = known_return_type(fn, arg_constraints)
    Const { result, val } →
      constraint[result] = literal_constraint(val)
  for branch terminator:
    if constraint of condition var is statically known → fold to Jump
```

### 4.3 Optimizations Enabled by Constraints

| Optimization | Trigger condition |
|---|---|
| Dead branch elimination | Condition var has a statically-known boolean value |
| Predicate elimination | `(int? x)` where `x: Prim(Long)` → replace with `Const(true)` |
| Nil-check elimination | `x` has non-nullable constraint → skip nil guard |
| Call specialization | `(+ x y)` where both are `Prim(Long)` → emit `CallKnown(Add)` with `UnboxHint` |
| Map get optimization | `(get x :foo)` where `x` has `MapShape { required: ["foo"] }` → result is non-nil |
| Count folding | `(count x)` where `x` has `VecShape { min: 3, max: 3 }` → `Const(3)` |

### 4.4 New JIT Hint on `CallKnown`

Add an optional `unbox_hint: Option<UnboxHint>` to `CallKnown`:

```rust
pub struct UnboxHint {
    pub arg_types:   Vec<Option<PrimType>>,
    pub result_type: Option<PrimType>,
}
```

The JIT (Cranelift backend) uses this to emit unboxed arithmetic directly instead of boxing/unboxing through `Value::Long`.

---

## Phase 5 — Runtime Spec Checks

For specs containing user-defined predicates, `s/multi-spec`, or complex `s/fn` constraints that cannot be resolved statically, we insert `SpecCheck` instructions. These are guarded by a dynamic variable so they can be disabled in production.

### 5.1 Control Variable

```clojure
(def ^:dynamic *spec-checking-enabled* false)  ; in clojure.spec.alpha
```

`s/instrument` binds this true for instrumented vars. Users can also enable it globally during development via `alter-var-root`.

### 5.2 When to Emit `SpecCheck`

The lowering pass checks a compile-time flag (`cljrs.compiler.options/instrument?`) rather than the runtime dynamic var, so the decision is made once at lowering time and baked into the IR.

| Situation | Emit condition |
|---|---|
| Function with fdef `:args` | `instrument?` flag set at lowering time, or function is a public API boundary |
| Function with fdef `:ret` | Same as above, on the return value |
| Explicit `s/assert` call | Always |
| `s/conform` call | Always |
| Inline `s/valid?` in `if` | Never — becomes a `Branch` directly |

### 5.3 IR Interpreter Handling

In `cljrs-eval/src/ir_interp.rs`:

```rust
Inst::SpecAnnotate { .. } => { /* metadata only — no-op at runtime */ }

Inst::SpecCheck { result, val, spec_name, fail_mode } => {
    let v = reg(val);
    let conformed = call_spec_conform(globals, spec_name, v.clone())?;
    if conformed == Value::Keyword("clojure.spec.alpha/invalid") {
        match fail_mode {
            SpecFailMode::Throw => {
                return Err(EvalError::SpecFailure {
                    spec: spec_name.clone(),
                    value: v,
                });
            }
            SpecFailMode::Warn => {
                eprintln!("WARNING: spec {} failed for {:?}", spec_name, v);
                set_reg(result, v);
            }
        }
    } else {
        set_reg(result, conformed);
    }
}
```

`call_spec_conform` invokes `clojure.spec.alpha/conform` via the standard callback mechanism.

### 5.4 JIT Codegen for `SpecCheck` (Cranelift)

The common path (checks disabled) is a single load + branch with the slow path out-of-line:

```
load *spec-checking-enabled*
branch-if-false → skip_label
  call spec_conform(spec_name_ptr, val_ptr) → conformed
  cmp conformed, INVALID_SENTINEL
  branch-if-eq → fail_label
  store conformed → result_reg
  jump → done_label
fail_label:
  call throw_spec_failure(spec_name_ptr, val_ptr)
skip_label:
  store val → result_reg
done_label:
```

---

## Summary of File Changes

| File | Change |
|---|---|
| `cljrs-stdlib/src/clojure/spec/alpha.cljrs` | New: full spec library |
| `cljrs-stdlib/src/clojure/spec/alpha/conformer.cljrs` | New: conformance engine (`s/cat` NFA, etc.) |
| `cljrs-ir/src/spec.rs` | New: `SpecConstraint`, `FnSpec`, `SpecFailMode` types |
| `cljrs-ir/src/lib.rs` | Add `SpecAnnotate`/`SpecCheck` to `Inst`; add `spec_annotations` and `fn_spec` to `IrFunction`; bump `FORMAT_VERSION` |
| `cljrs-ir/src/clojure/compiler/spec_query.cljrs` | New: spec registry query helpers for the lowering pass |
| `cljrs-ir/src/clojure/compiler/anf.cljrs` | Extend call lowering, parameter binding, literal lowering, and `if` lowering to emit `SpecAnnotate`/`SpecCheck` |
| `cljrs-eval/src/ir_convert.rs` | Convert new `SpecAnnotate`/`SpecCheck` Clojure data → Rust `Inst` |
| `cljrs-eval/src/ir_interp.rs` | Interpret `SpecCheck` (call into spec runtime), no-op `SpecAnnotate` |
| `cljrs-compiler/src/spec_prop.rs` | New: spec propagation dataflow pass |
| `cljrs-compiler/src/lib.rs` | Wire propagation pass into optimization pipeline after lowering |
| `cljrs-ir-prebuild/src/main.rs` | Rebuild prebuilt bundles after IR format change |

## Implementation Order

1. **`clojure.spec.alpha`** — pure library, no IR dependency, can be tested independently with `s/valid?` / `s/conform` calls
2. **IR type extensions** — add `spec.rs`, extend `Inst`/`IrFunction`, update serialization, bump version, update `ir_convert.rs`
3. **Interpreter no-op and check** — add `SpecAnnotate` (skip) and `SpecCheck` (call runtime) to the IR interpreter so all existing tests still pass
4. **Spec-aware lowering** — add `spec_query.cljrs`, extend `anf.cljrs`; verify that functions with fdefs get annotated vars in the IR
5. **Spec propagation pass** — implement the dataflow analysis and optimization transforms; gate behind a feature flag initially
6. **JIT integration** — extend Cranelift codegen to emit the out-of-line `SpecCheck` pattern and consume `UnboxHint` on `CallKnown` for unboxed arithmetic
