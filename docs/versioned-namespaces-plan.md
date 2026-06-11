# Versioned Namespaces/Symbols for AOT, JIT, and Native (Rust-Interop) Packages

> **Status:** Implemented (Phases 0–6).  The AOT *static-link* variant for
> pinned native deps (end of Phase 5) is deferred — pinned native packages
> use the dlopen path; see `crates/cljrs-dylib/README.md` and TODO.md
> Phase 9 for the open `#[export]`-inventory-collision problem.

## Context

Versioned symbols (`foo@abc1234`, `my.ns/foo@abc1234`) and versioned requires
(`(require '[my.lib@abc1234 :as v1])`) currently work **only in the tree-walking
interpreter** (Tier 3). The resolution logic lives in
`crates/cljrs-interp/src/versioned.rs`: fetch source from git at the commit, evaluate,
cache in `GlobalEnv::version_cache`.

The other execution paths have **zero version awareness** (verified):

- **Tier 2 (IR interpreter)** — `lower_symbol`/`split_sym`
  (`crates/cljrs-ir/src/lower/anf.rs:2356-2381`) split only on `/`, so `foo@hash`
  lowers to `Inst::LoadGlobal(ns, "foo@hash")`; `load_global_value`
  (`crates/cljrs-eval/src/ir_interp.rs:749`) then fails `lookup_var_in_ns` — a hard
  error, not a graceful Tier-3 fallback.
- **Tier 1 (JIT) and AOT** — `LoadGlobal` compiles to `rt_load_global`
  (`crates/cljrs-compiler/src/rt_abi.rs`), same string lookup, same failure. AOT
  additionally never bundles pinned sources, so even versioned requires that worked
  at compile time would hit git again at runtime.
- **Native functions** — versioned lookup of a Rust-native fn silently falls back to
  the HEAD implementation (`crates/cljrs-interp/src/versioned.rs:117-129`).

**User decisions (locked in):**
1. **AOT = snapshot at compile time.** Pinned sources are fetched and embedded in the
   binary; the binary is self-contained (no git/network at runtime). A versioned
   symbol not embedded at compile time errors clearly at runtime.
2. **Native fns = hybrid.** Default "verified HEAD binding": native fns come from the
   current binary, with provenance recorded at registration and a warning (error under
   a strict flag) on pinned-commit mismatch. Opt-in: build the crate at the pinned
   commit and dlopen it (or statically link it for AOT).

**Two latent defects found during exploration (fixed by this design; write repro tests first):**
- (a) Per-symbol resolution (`eval_in_snapshot`, `cljrs-interp/src/versioned.rs:162-165`)
  evaluates the historical `(def foo …)` with `current_ns` = the *base* (HEAD)
  namespace — interning the historical value into the live namespace, clobbering HEAD.
- (b) `trace_globals` (`crates/cljrs-env/src/gc_roots.rs:312-318`) traces only
  `globals.namespaces`; values cached *only* in `version_cache` are not GC-rooted.

## Core design: normalize everything onto versioned namespaces

Make `"ns@commit"` namespaces the single runtime concept for all tiers:

> Resolving `ns/name@commit` (any tier) = ensure the versioned namespace `ns@commit`
> is loaded (lazily, from embedded source or git), then a plain
> `lookup_in_ns("ns@commit", name)` — with the native-provenance fallback when no
> Clojure source defines the symbol.

This replaces the find-one-def-form snapshot path with whole-file load (already the
behavior for versioned requires; cached once per `ns@commit`). Consequences:

- Fixes defect (a): historical defs intern into the immutable `ns@commit` namespace.
  Fixes defect (b): resolved values live in `globals.namespaces` and get traced.
- **Same-ns commit inheritance becomes structural** for IR/JIT/AOT: fns defined while
  loading `ns@commit` get `defining_ns = "ns@commit"`; unqualified refs lower to
  `LoadGlobal("ns@commit", helper)` — a plain namespace hit. No commit threading
  through IR, `EvalContext`, or codegen. The tree-walker's `versioned_eval_commit`
  stays for compatibility.
- Accepted trade-off: per-symbol resolution evaluates the whole file at that commit
  (top-level side effects run once). Document in VERSIONING.md.

**IR representation: `Inst::LoadGlobal` unchanged; the version rides in the name
string** (`"foo@abc1234"`), which the lexer already produces. Rationale: `IrFunction`
is postcard-serialized (format stability), ~12 passes pattern-match `LoadGlobal`
untouched, and alias-qualified versioned symbols can't be fully resolved at lowering
time anyway. Runtime lookup points detect a valid `@<7-40 hex>` suffix (reuse
`cljrs_value::symbol::is_commit_hash`), split, resolve alias → base ns, and call the
shared resolver. Codegen statically detects the same pattern to emit an inline-cache
load.

**Feasibility keystone (verified):** `cljrs-env` does not depend on `cljrs-interp`;
versioned-namespace loading already lives in `cljrs-env/src/loader.rs:173-261` and
snapshot eval goes through the injected `GlobalEnv::eval_fn` callback (`env.rs:97`).
The per-symbol resolver can move down into `cljrs-env` with no dependency cycle.

---

## Phase 0 — Move and unify the resolver in `cljrs-env`

- **New `crates/cljrs-env/src/versioned.rs`** (non-wasm, cfg-gated like the loader):
  - `ensure_versioned_ns_loaded(globals, base_ns, commit) -> EvalResult<Arc<str>>` —
    returns the `ns@commit` name; refactor `loader::load_versioned_ns` to delegate;
    idempotent.
  - `resolve_versioned_value(globals, defining_ns, ns_part, name, commit) -> EvalResult<Value>`
    — alias resolution, ensure-load, `lookup_in_ns`, then native fallback (moved from
    `cljrs-interp/src/versioned.rs:117-129`).
- **`GlobalEnv` additions** (`crates/cljrs-env/src/env.rs`):
  `versioned_sources: RwLock<HashMap<Arc<str>, Arc<str>>>` (key `"ns@commit"`,
  value = fetched source text) — populated whenever the loader fetches from git;
  feeds AOT embedding in Phase 3.
- **`crates/cljrs-env/src/loader.rs`**: versioned loader checks builtin/embedded
  sources (`register_builtin_source` registry) **before** the git path — embedded
  sources skip git, repo-root discovery, and runtime signature checks.
- **`crates/cljrs-env/src/gc_roots.rs`**: trace `version_cache` values in
  `trace_globals` (defect (b) fix, landed regardless).
- **`crates/cljrs-interp/src/versioned.rs`** becomes a thin shim delegating to
  `cljrs_env::versioned`; call sites in `eval.rs:170-230` unchanged.

**Tests:** existing interp versioned tests + `crates/cljrs-vcs/tests/versioning_harness.rs`
pass unchanged; new regression test that per-symbol `foo@hash` does **not** clobber the
HEAD binding of `foo`; GC stress test that a resolved versioned value survives collection.

## Phase 1 — Tier 2 (IR interpreter)

- **`crates/cljrs-eval/src/ir_interp.rs`** `load_global_value` (~line 749): if `name`
  carries a valid hash suffix → `cljrs_env::versioned::resolve_versioned_value`; if
  `ns` itself is an `@`-name not yet loaded → lazily `ensure_versioned_ns_loaded`.
- **`crates/cljrs-ir/src/lower/anf.rs`** (small, optional but recommended): when
  `ctx.ns()` is `"base@hash"` and a qualified symbol's ns part equals `base`, rewrite
  to `ctx.ns()` (qualified self-references resolve at the pinned commit, not HEAD).
  Mirror in the tree-walker for parity; verify current behavior with a test first.

**Tests** (`crates/cljrs-eval/tests/`): temp git repo fixture (reuse `git_cmd`/`git_sha`
helper patterns from `versioning_harness.rs`); a fn whose body references `lib/f@<sha1>`;
force the IR path with `CLJRS_EAGER_LOWER=1`; assert pinned result and that the IR cache
was used.

## Phase 2 — Tier 1 (JIT) + AOT codegen

- **`crates/cljrs-compiler/src/rt_abi.rs`**:
  - Make `rt_load_global` version-aware as the slow path: detect `@hash`, resolve via
    `with_eval_context`; report errors through `stash_pending_exception` (the
    established channel), never silently nil.
  - New `rt_load_global_versioned_ic(ns_ptr, ns_len, name_ptr, name_len, slot) -> *const Value`,
    modeled on `rt_kw_ic_fill` (rt_abi.rs:3915): resolve once, box the value,
    **register it as a permanent GC root**, store the pointer in the per-call-site
    slot. Check how `rt_kw_ic_fill`'s interned keywords are rooted; if keyword-specific,
    add `permanent_value_roots: Mutex<Vec<Value>>` to `GlobalEnv`, traced by
    `trace_globals`. Anchor both in `anchor_rt_symbols`.
- **`crates/cljrs-compiler/src/codegen.rs`**: in `Inst::LoadGlobal` handling, if the
  name has a valid version suffix, emit the IC pattern (clone of `emit_keyword_ic`,
  line 1197, with string args + slot). Register the new FuncId in `RtFuncs`.
  **No invalidation machinery needed** — versioned bindings are immutable; the slot is
  fill-once-forever (assert/ignore `@` namespaces in the `on_var_rebind` JIT hook for
  defense). This is the immutability win.

**Tests** (`crates/cljrs-jit/tests/`): hot loop calling a versioned fn with a low
`CLJRS_JIT_THRESHOLD` (`set_jit_threshold`); assert pinned result and
`jit_state::get_native_fn(...).is_some()` after warmup; tier-consistency test (JIT
result == tree-walker result).

## Phase 3 — AOT snapshot

Files: `crates/cljrs-compiler/src/aot.rs`, small changes in `cljrs-env`
(`loader.rs`, `env.rs`).

1. **Compile-time verification**: propagate `verify-commit-signatures` from
   cljrs.edn/CLI onto the macroexpansion `GlobalEnv` in `compile_file` before any
   require runs — signature checks happen at compile time; embedded sources are then
   trusted as part of the binary.
2. **Discovery**: versioned requires already execute during expansion (aot.rs:432-437)
   and now record into `versioned_sources` (Phase 0). Additionally, walk the final
   `IrFunction` tree (reuse the `referenced_globals` walker pattern,
   `cljrs-eval/src/lower.rs:208`) plus interpreted-preamble forms for `@hash` names,
   forcing `ensure_versioned_ns_loaded` at compile time — catches bare `foo@hash`
   symbols no require mentions. Transitive versioned requires inside pinned sources
   are caught automatically (loading a pinned file executes its own requires).
3. **Embedding**: extend `discover_bundled_sources` (aot.rs:656-681) to append every
   `versioned_sources` entry as `(ns@commit, source)` — the existing
   `bundled_N.cljrs` + `register_builtin_source("my.lib@abc1234", include_str!(…))`
   harness machinery (aot.rs:776-789) needs no structural change. At runtime the
   loader's builtin-source-first path (Phase 0) means no git, no network.
4. **Offline mode**: generated harness `main.rs` calls a new
   `globals.set_versioned_offline(true)` (AtomicBool). When set,
   `load_versioned_ns` with no builtin source fails immediately:
   `"versioned namespace my.lib@abc1234 was not embedded at compile time; AOT binaries cannot fetch from git at runtime"`.

**Tests** (extend `crates/cljrs-compiler/tests/aot_e2e.rs`): two-repo fixture; compile
an app pinning `lib@sha1` (both require-style and bare-symbol-style); run the binary
from an empty cwd with `HOME` pointed at an empty dir and the lib repo deleted —
assert pinned output; negative test that a non-embedded `other@sha` produces the
clear offline error.

## Phase 4 — Native provenance (default hybrid)

- **`GlobalEnv`** (`cljrs-env/src/env.rs`):
  `native_provenance: RwLock<HashMap<Arc<str> /*ns*/, Arc<str> /*commit*/>>`,
  `enforce_native_versions: AtomicBool`, `provenance_warned: Mutex<HashSet<Arc<str>>>`
  (warn once per `ns@commit`). Lives in cljrs-env so the resolver checks it without
  an interop dependency.
- **`crates/cljrs-interop`**: `Registry::set_provenance(ns, commit)`; inventory-collected
  `ProvenanceEntry { ns, commit }` + a `register_provenance!(ns, commit)` macro
  (typically `commit = env!("CLJRS_PKG_COMMIT")` emitted by the package's build.rs);
  registered alongside exports.
- **Resolver hook** (in the Phase-0 native fallback): look up provenance for the base
  ns; prefix-match in either direction (requested hash may be abbreviated). Match →
  silent. Mismatch/missing → warn once (default), `EvalError` under the strict flag.
- **Flags**, mirroring `verify_commit_signatures` style: CLI
  `--enforce-native-versions`, cljrs.edn `:enforce-native-versions true` (parse in
  `cljrs-deps`, wire in `cljrs/src/main.rs` next to the existing flag).

**Tests:** matching provenance → silent; mismatch → value returned + warned-set
populated (assert on the set, not stderr); strict flag → error.

## Phase 5 — Opt-in pinned native code (dlopen / static link)

New crate `crates/cljrs-dylib` (non-wasm; deps: cljrs-env, cljrs-vcs, cljrs-interop,
cljrs-deps, libloading).

- **Config**: per-dep in cljrs.edn:
  `{:git/url … :git/sha … :rust/init "my_crate::cljrs_init" :rust/load :dylib}`
  (extend `Dependency` in `cljrs-deps`).
- **Flow (interpreter/JIT)**: native fallback, when the dep is `:rust/load :dylib`:
  `fetch_remote(url, sha)` → generate a thin wrapper cdylib crate in
  `~/.cljrs/cache/dylibs/<pkg>@<commit>/` (reuse the `resolve_harness_deps` /
  `HarnessDeps` pattern from aot.rs:962-1110 to pin identical cljrs crate versions) →
  `cargo build --release` → cache keyed by `(pkg, commit, rustc -V, cljrs version,
  target)` → dlopen.
- **ABI discipline**: two exported symbols. (1) C-ABI handshake
  `cljrs_dylib_abi() -> *const CljrsAbiInfo` (`#[repr(C)]`, carries cljrs-value
  version + `rustc -V` baked in at wrapper build time; host requires exact equality
  or refuses with a clear error). (2) Rust-ABI
  `cljrs_dylib_init(registry: &mut Registry)` — acceptable only because the handshake
  guarantees identical compiler + crate versions. A full C-ABI vtable is the safer
  long-term answer; explicitly deferred and documented as experimental.
- **Versioned registration**: the wrapper's init calls the dep's `cljrs_init` through
  a `Registry::versioned(commit)` view that rewrites `define("ns/name")` →
  `define_in("ns@commit", name)`; `inventory` iteration happens *inside the dylib* so
  its `#[export]` entries don't collide with the host's.
- **AOT variant**: prefer static linking over runtime dlopen — at compile time add the
  pinned crate to the harness Cargo.toml as
  `pkg_abc1234 = { package = "the-crate", path = "<cache checkout>" }` and call its
  init through the versioned Registry view in generated main.rs.
  **Open problem (call out in docs/PR):** two statically linked versions of the same
  crate both submit `#[export]` inventory entries under the unversioned name in
  nondeterministic order — for the static path, require init-fn-based registration
  for pinned native deps (error otherwise), or fall back to dlopen for that dep.

**Tests:** heavyweight integration test gated behind an env var (it runs cargo):
two-commit native crate fixture, pin commit 1, assert the dylib-loaded `pkg@sha1`
impl differs from HEAD; handshake-mismatch negative test.

## Phase 6 — Documentation (CLAUDE.md mandates README currency)

- `VERSIONING.md`: whole-file snapshot semantics, AOT snapshot model, native hybrid +
  flags, dlopen opt-in + ABI requirements.
- `docs/book/src/language/versioned-symbols.md`: same, user-facing.
- Crate READMEs: cljrs-env (new `versioned` module, GlobalEnv fields), cljrs-interp
  (shim note), cljrs-eval, cljrs-compiler (new rt symbols, AOT embedding),
  cljrs-interop (provenance API), new cljrs-dylib.
- `TODO.md`: update Phase 9 dynamic-loading entry.

## Sequencing, verification, risks

**Order:** 0 → 1 → 2 → 3 (strict); 4 depends only on 0; 5 depends on 4 (+3 for the
static-link variant). Each phase lands green independently.

**End-to-end verification:**
- `cargo test` workspace-wide after each phase (notably `versioning_harness.rs`).
- Tier matrix on one fixture: same pinned call evaluated via tree-walker, via
  `CLJRS_EAGER_LOWER=1` (Tier 2), via low `CLJRS_JIT_THRESHOLD` (Tier 1), and via a
  compiled binary — all four must agree and differ from HEAD.
- AOT offline proof: run the compiled binary with the source repo deleted and `HOME`
  redirected (no `~/.cljrs/cache`).

**Risks:**
1. Whole-file snapshot eval changes per-symbol semantics (side effects, load cost) —
   mitigated by per-`ns@commit` caching + docs.
2. Permanent rooting of IC-cached values — verify how `rt_kw_ic_fill` roots interned
   keywords and mirror it before relying on the pattern.
3. The Rust-ABI dylib boundary stays fragile even with the handshake (feature-flag
   skew) — ship as experimental, documented.
4. Inventory collision in static-link AOT native pinning (open question above).
5. The `version_cache` GC-tracing fix may surface previously-masked lifetime
   assumptions — land with its own stress test.
6. Reproduce defect (a) with a test before claiming the fix.
