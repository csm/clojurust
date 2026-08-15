//! Lexical environment: local frames, global namespace table, and current Env.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};

use crate::env::async_hook::AsyncRuntime;

use crate::env::error::EvalResult;
use crate::mode::{ExecutionMode, TierState};
use cljrs_gc::{GcConfig, GcPtr};
use cljrs_logging::feat_trace;
use cljrs_reader::Form;
use cljrs_value::{CljxFn, Namespace, Value, Var};
// ── RequireSpec / RequireRefer ─────────────────────────────────────────────────

/// How symbols should be referred into the requiring namespace.
#[derive(Debug, Clone)]
pub enum RequireRefer {
    None,
    All,
    Named(Vec<Arc<str>>),
}

/// A parsed `require` specification.
#[derive(Debug, Clone)]
pub struct RequireSpec {
    pub ns: Arc<str>,
    /// Present when the namespace symbol carried a `@<hash>` version suffix.
    pub version: Option<Arc<str>>,
    pub alias: Option<Arc<str>>,
    pub refer: RequireRefer,
}

// ── Frame ─────────────────────────────────────────────────────────────────────

/// One stack frame of local bindings (a single `let*`, `fn`, or `loop*` scope).
pub struct Frame {
    pub bindings: Vec<(Arc<str>, Value)>,
}

impl Default for Frame {
    fn default() -> Self {
        Self::new()
    }
}

impl Frame {
    pub fn new() -> Self {
        Self {
            bindings: Vec::new(),
        }
    }

    pub fn bind(&mut self, name: Arc<str>, val: Value) {
        // Shadow: push new binding; lookup searches from the end.
        self.bindings.push((name, val));
    }

    pub fn lookup(&self, name: &str) -> Option<&Value> {
        // Search in reverse order so later bindings shadow earlier ones.
        feat_trace!("env", "lookup {}", name);
        for (n, v) in self.bindings.iter().rev() {
            if n.as_ref() == name {
                return Some(v);
            }
        }
        None
    }
}

// ── GlobalEnv ─────────────────────────────────────────────────────────────────

/// The global mutable store of all namespaces.
pub struct GlobalEnv {
    /// Process-unique identity of this runtime instance.
    ///
    /// Allocated from a counter, not derived from the `Arc`'s address: an
    /// address is only unique while the allocation is live, so a dropped
    /// runtime could hand its key to the next one and let it inherit stale
    /// cross-defn IR.  Used to scope per-instance registries.
    id: u64,
    pub namespaces: RwLock<HashMap<Arc<str>, GcPtr<Namespace>>>,
    /// Directories to search when resolving namespace names to files.
    pub source_paths: RwLock<Vec<std::path::PathBuf>>,
    /// Namespaces that have been fully loaded from a file (idempotent guard).
    pub loaded: Mutex<std::collections::HashSet<Arc<str>>>,
    /// Namespaces currently being loaded, mapped to the thread loading them.
    /// Used to detect true circular requires (same thread) vs concurrent loads
    /// (different thread — those wait on `loading_done` instead of erroring).
    pub loading: Mutex<HashMap<Arc<str>, std::thread::ThreadId>>,
    /// Signalled whenever a namespace finishes loading (or fails).
    pub loading_done: Condvar,
    /// Built-in namespace sources embedded in the binary.
    /// Checked by `load_ns` before falling back to source-path search.
    pub builtin_sources: RwLock<HashMap<Arc<str>, &'static str>>,
    /// GC configuration for automatic collection based on memory pressure.
    pub gc_config: RwLock<Option<Arc<GcConfig>>>,
    /// How this runtime executes function calls.  Fixed when the runtime is
    /// built; see [`crate::RuntimeBuilder::execution_mode`].
    execution_mode: ExecutionMode,
    /// Which tiers are live right now (see [`TierState`]).  Starts at
    /// [`TierState::TreeWalk`] — nothing can be lowered until `clojure.core`
    /// exists — and is raised once to `execution_mode.target_tier()` when the
    /// builder finishes bootstrapping.
    tier_state: AtomicU8,
    /// This runtime's Tier-1 and Tier-2 state: the lowered-IR cache, the JIT
    /// counters and native-code tables, and the JIT backend attached to this
    /// runtime.  Instance state: two runtimes in one process never read,
    /// evict, or invalidate each other's entries, and everything dies with
    /// the runtime.
    tiers: Arc<crate::tiered::tiers::Tiers>,
    /// Optional async runtime registered by `cljrs-async`.
    /// `None` when the library is not linked; `Some` after `cljrs_async::init`.
    pub async_rt: RwLock<Option<Arc<dyn AsyncRuntime>>>,
    /// Cache of values resolved at a specific commit.
    /// Key format: `"<ns>/<name>@<commit>"` for individual vars,
    /// or `"<ns>@<commit>"` for whole versioned namespaces.
    pub version_cache: Mutex<HashMap<Arc<str>, Value>>,
    /// Parsed `cljrs.edn` config, loaded once at startup.
    pub deps_config: RwLock<Option<Arc<cljrs_deps::DepsConfig>>>,
    /// When true, every versioned-symbol or versioned-namespace resolution must
    /// carry a valid commit signature (verified natively against `trusted_keys`)
    /// before the historical code is executed.  Off by default; enabled via
    /// `--verify-commit-signatures` CLI flag or `:verify-commit-signatures true`
    /// in `cljrs.edn`.
    pub verify_commit_signatures: AtomicBool,
    /// Public keys trusted to sign versioned dependency commits, built from
    /// the `:trusted-signers` config.  Consulted by `check_commit_signature`
    /// when `verify_commit_signatures` is on.  (Not built on wasm, where
    /// `cljrs-vcs` is unavailable and signature checks are no-ops.)
    #[cfg(not(target_arch = "wasm32"))]
    pub trusted_keys: RwLock<Arc<cljrs_vcs::TrustedKeys>>,
    /// Session-scoped cache of commits that have already passed signature
    /// verification this run, keyed by `(repo_root, commit_hash)`.
    pub sig_verify_cache: Mutex<HashSet<(Arc<str>, Arc<str>)>>,
    /// Pinned source texts fetched from git this session, keyed by
    /// `"<ns>@<commit>"`.  The AOT compiler embeds these in the produced
    /// binary so versioned namespaces resolve without git at runtime.
    pub versioned_sources: RwLock<HashMap<Arc<str>, Arc<str>>>,
    /// When true (set by AOT harness main), versioned namespaces resolve
    /// only from embedded builtin sources — never from git.  A versioned
    /// namespace that was not embedded at compile time fails with a clear
    /// error instead of attempting a fetch.
    pub versioned_offline: AtomicBool,
    /// Provenance of native (Rust-backed) packages recorded at registration:
    /// namespace → the git commit the package was built from.  Consulted by
    /// the versioned resolver's native HEAD fallback to detect pinned-commit
    /// mismatches.
    pub native_provenance: RwLock<HashMap<Arc<str>, Arc<str>>>,
    /// When true, a pinned lookup of a native function whose recorded
    /// provenance does not match the requested commit is an error instead of
    /// a once-per-pin warning.  CLI: `--enforce-native-versions`; cljrs.edn:
    /// `:enforce-native-versions true`.
    pub enforce_native_versions: AtomicBool,
    /// Pinned-native mismatches already warned about this session
    /// (key: `"<ns>@<commit>"`), so each pin warns at most once.
    pub provenance_warned: Mutex<HashSet<Arc<str>>>,
    /// Optional loader for **pinned native packages** (`:rust/load :dylib`),
    /// installed by `cljrs-dylib`.  Called by the versioned resolver with
    /// `(globals, base_ns, commit)` before falling back to the HEAD native
    /// binding; returns `Ok(true)` when it registered the package's pinned
    /// implementations into the `"<base_ns>@<commit>"` namespace.
    #[allow(clippy::type_complexity)]
    pub pinned_native_loader: RwLock<Option<PinnedNativeLoader>>,
    /// Optional loader for **native dependencies on the plain `require` path**
    /// (`:rust/load :dylib`), installed by `cljrs-dylib`.  Called by the
    /// unversioned namespace loader with `(globals, ns)` when a `require`d
    /// namespace has no Clojure source on the source path; returns `Ok(true)`
    /// when it built the dep's crate at the pinned `:git/sha` and registered
    /// the package's exports into the **unversioned** namespace, so a plain
    /// `(require '[my.native.lib :as lib])` brings the native code in.
    #[allow(clippy::type_complexity)]
    pub native_require_loader: RwLock<Option<NativeRequireLoader>>,
    /// Loaders for **AOT-compiled namespaces**, installed by the binary
    /// produced by `cljrs compile`.  Keyed by namespace name.  When a plain
    /// `require` resolves a namespace that has a registered loader, `load_ns`
    /// invokes the loader instead of interpreting Clojure source: the loader
    /// evaluates the namespace's small interpreted preamble (its `ns`/`require`
    /// and macro definitions) and then calls the namespace's natively compiled
    /// initializer, so the bulk of the namespace runs as machine code rather
    /// than being tree-walked at startup.
    #[allow(clippy::type_complexity)]
    pub compiled_ns_loaders: RwLock<HashMap<Arc<str>, CompiledNsLoader>>,
}

/// Loader callback for an AOT-compiled namespace (see
/// `GlobalEnv::compiled_ns_loaders`).  Given the global env, it loads the
/// namespace by running its interpreted preamble and its compiled initializer.
pub type CompiledNsLoader = Arc<dyn Fn(&Arc<GlobalEnv>) -> EvalResult<()> + Send + Sync>;

/// Loader callback for pinned native packages (see
/// `GlobalEnv::pinned_native_loader`).
pub type PinnedNativeLoader =
    Arc<dyn Fn(&Arc<GlobalEnv>, &str, &str) -> EvalResult<bool> + Send + Sync>;

/// Loader callback for native dependencies reached through a plain `require`
/// (see `GlobalEnv::native_require_loader`).
pub type NativeRequireLoader = Arc<dyn Fn(&Arc<GlobalEnv>, &str) -> EvalResult<bool> + Send + Sync>;

/// Source of [`GlobalEnv::id`] values.
static NEXT_GLOBAL_ENV_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl std::fmt::Debug for GlobalEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GlobalEnv {{ ... }}")
    }
}

impl GlobalEnv {
    /// Create an empty global environment for the given execution mode.
    ///
    /// This is the *raw* constructor: no builtins, no bootstrap, no source
    /// paths.  Use [`crate::Runtime::builder`] unless you are the builder.
    pub fn new(execution_mode: ExecutionMode) -> Arc<Self> {
        let id = NEXT_GLOBAL_ENV_ID.fetch_add(1, Ordering::Relaxed);
        Arc::new(Self {
            id,
            namespaces: RwLock::new(HashMap::new()),
            source_paths: RwLock::new(Vec::new()),
            loaded: Mutex::new(std::collections::HashSet::new()),
            loading: Mutex::new(HashMap::new()),
            loading_done: Condvar::new(),
            builtin_sources: RwLock::new(HashMap::new()),
            gc_config: RwLock::new(None),
            execution_mode,
            tier_state: AtomicU8::new(TierState::TreeWalk as u8),
            tiers: crate::tiered::tiers::Tiers::new(id),
            async_rt: RwLock::new(None),
            version_cache: Mutex::new(HashMap::new()),
            deps_config: RwLock::new(None),
            verify_commit_signatures: AtomicBool::new(false),
            #[cfg(not(target_arch = "wasm32"))]
            trusted_keys: RwLock::new(Arc::new(cljrs_vcs::TrustedKeys::new())),
            sig_verify_cache: Mutex::new(HashSet::new()),
            versioned_sources: RwLock::new(HashMap::new()),
            versioned_offline: AtomicBool::new(false),
            native_provenance: RwLock::new(HashMap::new()),
            enforce_native_versions: AtomicBool::new(false),
            provenance_warned: Mutex::new(HashSet::new()),
            pinned_native_loader: RwLock::new(None),
            native_require_loader: RwLock::new(None),
            compiled_ns_loaders: RwLock::new(HashMap::new()),
        })
    }

    /// Replace the source path list.
    pub fn set_source_paths(&self, paths: Vec<std::path::PathBuf>) {
        *self.source_paths.write().unwrap() = paths;
    }

    /// Register an embedded namespace source (called by cljrs-stdlib at startup).
    pub fn register_builtin_source(&self, ns: &str, src: &'static str) {
        self.builtin_sources
            .write()
            .unwrap()
            .insert(Arc::from(ns), src);
    }

    /// Look up an embedded source for a namespace, if one has been registered.
    pub fn builtin_source(&self, ns: &str) -> Option<&'static str> {
        self.builtin_sources.read().unwrap().get(ns).copied()
    }

    /// Register a loader for an AOT-compiled namespace (called by the harness
    /// `main` of a binary produced by `cljrs compile`).
    pub fn register_compiled_ns_loader(&self, ns: &str, loader: CompiledNsLoader) {
        self.compiled_ns_loaders
            .write()
            .unwrap()
            .insert(Arc::from(ns), loader);
    }

    /// Look up the loader for an AOT-compiled namespace, if one is registered.
    pub fn compiled_ns_loader(&self, ns: &str) -> Option<CompiledNsLoader> {
        self.compiled_ns_loaders.read().unwrap().get(ns).cloned()
    }

    /// Mark a namespace as fully loaded from a file.
    pub fn mark_loaded(&self, ns: &str) {
        self.loaded.lock().unwrap().insert(Arc::from(ns));
    }

    /// True if the namespace has already been loaded from a file.
    pub fn is_loaded(&self, ns: &str) -> bool {
        self.loaded.lock().unwrap().contains(ns)
    }

    /// Set the GC configuration for automatic memory pressure management.
    pub fn set_gc_config(&self, config: Arc<GcConfig>) {
        *self.gc_config.write().unwrap() = Some(config);
    }

    /// Get the GC configuration, if one has been set.
    pub fn gc_config(&self) -> Option<Arc<GcConfig>> {
        self.gc_config.read().unwrap().clone()
    }

    /// Resolve a short alias to a full namespace name in `current_ns`.
    pub fn resolve_alias(&self, current_ns: &str, alias: &str) -> Option<Arc<str>> {
        let map = self.namespaces.read().unwrap();
        let ns = map.get(current_ns)?;
        let aliases = ns.get().aliases.lock().unwrap();
        aliases.get(alias).cloned()
    }

    /// Resolve an auto-resolved keyword name (the text after `::`) to its
    /// fully-qualified `ns/name` form.
    ///
    /// `::kw` qualifies with `current_ns` directly; `::alias/kw` looks
    /// `alias` up in `current_ns`'s alias table (populated by `(require
    /// '[... :as alias])`) and qualifies with the resolved namespace.
    pub fn resolve_auto_keyword(&self, current_ns: &str, name: &str) -> Result<String, String> {
        match name.split_once('/') {
            Some((alias, kw_name)) => match self.resolve_alias(current_ns, alias) {
                Some(ns) => Ok(format!("{ns}/{kw_name}")),
                None => Err(format!(
                    "invalid token: ::{name} (no such namespace alias: {alias})"
                )),
            },
            None => Ok(format!("{current_ns}/{name}")),
        }
    }

    /// Return the namespace with this name, creating it if it doesn't exist.
    pub fn get_or_create_ns(&self, name: &str) -> GcPtr<Namespace> {
        // Fast path: already exists.
        {
            let map = self.namespaces.read().unwrap();
            if let Some(ns) = map.get(name) {
                return ns.clone();
            }
        }
        // Slow path: insert.
        let mut map = self.namespaces.write().unwrap();
        // Re-check after acquiring write lock.
        if let Some(ns) = map.get(name) {
            return ns.clone();
        }
        let ns = GcPtr::new(Namespace::new(name));
        map.insert(Arc::from(name), ns.clone());
        ns
    }

    /// Intern `name` with `val` in the given namespace, returning the Var.
    pub fn intern(&self, ns_name: &str, name: Arc<str>, val: Value) -> GcPtr<Var> {
        let ns = self.get_or_create_ns(ns_name);
        let mut interns = ns.get().interns.lock().unwrap();
        if let Some(var) = interns.get(&name) {
            // Update existing var.
            var.get().bind(val);
            return var.clone();
        }
        let var = GcPtr::new(Var::new(ns_name, name.as_ref()));
        var.get().bind(val);
        interns.insert(name, var.clone());
        var
    }

    /// Look up a Var in the named namespace (interns only).
    pub fn lookup_var(&self, ns_name: &str, sym_name: &str) -> Option<GcPtr<Var>> {
        let map = self.namespaces.read().unwrap();
        let ns = map.get(ns_name)?;
        let interns = ns.get().interns.lock().unwrap();
        interns.get(sym_name).cloned()
    }

    /// Look up a value in `ns_name`: checks interns then refers.
    /// Routes through the dynamic binding stack so `binding` overrides work.
    pub fn lookup_in_ns(&self, ns_name: &str, sym_name: &str) -> Option<Value> {
        let map = self.namespaces.read().unwrap();
        let ns = map.get(ns_name)?;
        let ns_ref = ns.get();
        // Check interns first.
        {
            let interns = ns_ref.interns.lock().unwrap();
            if let Some(var) = interns.get(sym_name) {
                return crate::env::dynamics::deref_var(var);
            }
        }
        // Then refers.
        {
            let refers = ns_ref.refers.lock().unwrap();
            if let Some(var) = refers.get(sym_name) {
                return crate::env::dynamics::deref_var(var);
            }
        }
        None
    }

    /// Look up the raw Var (not its value) in `ns_name`: interns then refers.
    pub fn lookup_var_in_ns(&self, ns_name: &str, sym_name: &str) -> Option<GcPtr<Var>> {
        let map = self.namespaces.read().unwrap();
        let ns = map.get(ns_name)?;
        let ns_ref = ns.get();
        {
            let interns = ns_ref.interns.lock().unwrap();
            if let Some(var) = interns.get(sym_name) {
                return Some(var.clone());
            }
        }
        {
            let refers = ns_ref.refers.lock().unwrap();
            if let Some(var) = refers.get(sym_name) {
                return Some(var.clone());
            }
        }
        None
    }

    /// Copy all interns from `src_ns` into `dst_ns` as refers.
    pub fn refer_all(&self, dst_ns: &str, src_ns: &str) {
        let map = self.namespaces.read().unwrap();
        let src = match map.get(src_ns) {
            Some(ns) => ns.clone(),
            None => return,
        };
        let dst = match map.get(dst_ns) {
            Some(ns) => ns.clone(),
            None => return,
        };
        let src_interns = src.get().interns.lock().unwrap();
        let mut dst_refers = dst.get().refers.lock().unwrap();
        for (name, var) in src_interns.iter() {
            dst_refers.insert(name.clone(), var.clone());
        }
    }

    /// Copy selected interns from `src_ns` into `dst_ns` as refers.
    pub fn refer_named(&self, dst_ns: &str, src_ns: &str, names: &[Arc<str>]) {
        let map = self.namespaces.read().unwrap();
        let src = match map.get(src_ns) {
            Some(ns) => ns.clone(),
            None => return,
        };
        let dst = match map.get(dst_ns) {
            Some(ns) => ns.clone(),
            None => return,
        };
        let src_interns = src.get().interns.lock().unwrap();
        let mut dst_refers = dst.get().refers.lock().unwrap();
        for name in names {
            if let Some(var) = src_interns.get(name) {
                // Use insert (not or_insert_with) so that an explicit
                // `require :refer [name]` always overrides a previous refer
                // (e.g. one inherited from clojure.core via refer-all).
                // clojure.core.async's `into` intentionally shadows clojure.core/into;
                // or_insert_with would silently drop the override.
                dst_refers.insert(name.clone(), var.clone());
            }
        }
    }

    /// Register `alias` → `full_ns` in `current_ns`'s alias table.
    pub fn add_alias(&self, current_ns: &str, alias: &str, full_ns: &str) {
        let ns_ptr = self.get_or_create_ns(current_ns);
        let mut aliases = ns_ptr.get().aliases.lock().unwrap();
        aliases.insert(Arc::from(alias), Arc::from(full_ns));
    }

    /// Process-unique identity of this runtime instance.
    #[inline(always)]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// This runtime's Tier-1/Tier-2 state.
    #[inline(always)]
    pub fn tiers(&self) -> &Arc<crate::tiered::tiers::Tiers> {
        &self.tiers
    }

    /// This runtime's cache of lowered IR.
    #[inline(always)]
    pub fn ir_cache(&self) -> &crate::tiered::ir_cache::IrCache {
        self.tiers.ir_cache()
    }

    /// This runtime's JIT counters, profiles, and native-code tables.
    #[inline(always)]
    pub fn jit(&self) -> &crate::tiered::jit_state::JitState {
        self.tiers.jit()
    }

    /// The JIT compiler attached to this runtime, if any.
    ///
    /// `None` when no JIT is linked or installed; callers then keep to the
    /// interpreter tiers.  Installed by `cljrs_compiler::jit::install`.
    #[inline(always)]
    pub fn jit_backend(&self) -> Option<Arc<dyn crate::tiered::backend::JitBackend>> {
        self.tiers.jit().backend().cloned()
    }

    // ── Execution mode and tier state ────────────────────────────────────

    /// How this runtime executes function calls.
    #[inline(always)]
    pub fn execution_mode(&self) -> ExecutionMode {
        self.execution_mode
    }

    /// Which tiers are live right now.
    #[inline(always)]
    pub fn tier_state(&self) -> TierState {
        TierState::from_u8(self.tier_state.load(Ordering::Acquire))
    }

    /// Raise the live tier state.  Called once by the runtime builder after
    /// the bootstrap completes; lowering the tier is not supported, so a
    /// request below the current state is ignored.
    pub fn set_tier_state(&self, tier: TierState) {
        let _ = self.tier_state.fetch_max(tier as u8, Ordering::AcqRel);
    }

    /// True when IR may be lowered, cached, and interpreted.  This is the
    /// gate the old `compiler_ready` flag served.
    #[inline(always)]
    pub fn ir_enabled(&self) -> bool {
        self.tier_state().ir_enabled()
    }

    // ── Evaluation entry points ──────────────────────────────────────────

    /// Evaluate `form` in `env`.
    #[inline(always)]
    pub fn eval(&self, form: &Form, env: &mut Env) -> EvalResult {
        crate::interp::eval::eval(form, env)
    }

    /// Call a Clojure function, taking the path this runtime's
    /// [`ExecutionMode`] selects.
    ///
    /// This is the single function-call dispatch point: tree walk, tier-1 IR,
    /// and JIT-native execution are all reached from here.
    #[inline(always)]
    pub fn call_cljrs_fn(&self, func: &CljxFn, args: &[Value], env: &mut Env) -> EvalResult {
        match self.execution_mode {
            ExecutionMode::TreeWalk => crate::interp::apply::call_cljrs_fn(func, args, env),
            ExecutionMode::Tiered | ExecutionMode::TieredNoJit => {
                crate::tiered::apply::call_cljrs_fn(func, args, env)
            }
            ExecutionMode::NoGcTransaction => crate::env::depth::call_cljrs_fn(func, args, env),
        }
    }

    /// Notify the active tier that a new `fn*` was defined.
    ///
    /// In a tiered runtime with IR enabled this eagerly lowers the function
    /// (when eager lowering is on); in every other mode it does nothing.
    #[inline(always)]
    pub fn on_fn_defined(&self, f: &CljxFn, env: &mut Env) {
        if self.execution_mode.is_tiered() && self.ir_enabled() {
            crate::tiered::ir_interp::eager_lower_fn(f, env);
        }
    }

    /// Install an async runtime. Called once by `cljrs_async::init`.
    /// Subsequent calls are silently ignored (first writer wins).
    pub fn set_async_runtime(&self, rt: Arc<dyn AsyncRuntime>) {
        let mut guard = self.async_rt.write().unwrap();
        if guard.is_none() {
            *guard = Some(rt);
        }
    }

    /// Return the async runtime, if one has been registered.
    pub fn async_runtime(&self) -> Option<Arc<dyn AsyncRuntime>> {
        self.async_rt.read().unwrap().clone()
    }

    /// Return `(source_file, git_repo_root)` for the named namespace, if
    /// both have been populated by the loader.
    pub fn get_ns_git_context(&self, ns_name: &str) -> Option<(Arc<str>, Arc<str>)> {
        let map = self.namespaces.read().unwrap();
        let ns = map.get(ns_name)?;
        let ns_ref = ns.get();
        let file = ns_ref.source_file.lock().unwrap().clone()?;
        let repo = ns_ref.git_repo_root.lock().unwrap().clone()?;
        Some((file, repo))
    }

    /// Store a resolved versioned value in the cache.
    /// Key: `"<ns>/<name>@<commit>"`.
    pub fn cache_versioned(&self, ns: &str, name: &str, commit: &str, val: Value) {
        let key: Arc<str> = Arc::from(format!("{ns}/{name}@{commit}"));
        self.version_cache.lock().unwrap().insert(key, val);
    }

    /// Retrieve a previously resolved versioned value, if cached.
    pub fn get_cached_versioned(&self, ns: &str, name: &str, commit: &str) -> Option<Value> {
        let key = format!("{ns}/{name}@{commit}");
        self.version_cache
            .lock()
            .unwrap()
            .get(key.as_str())
            .cloned()
    }

    /// Mark namespace `name@commit` as loaded in the standard loaded set.
    pub fn cache_versioned_ns(&self, ns: &str, commit: &str) {
        let key: Arc<str> = Arc::from(format!("{ns}@{commit}"));
        self.version_cache.lock().unwrap().insert(key, Value::Nil);
    }

    /// Record the source text of a versioned namespace fetched from git.
    /// Key: `"<ns>@<commit>"`.  Consumed by the AOT compiler for embedding.
    pub fn record_versioned_source(&self, versioned_ns: &str, src: &str) {
        self.versioned_sources
            .write()
            .unwrap()
            .insert(Arc::from(versioned_ns), Arc::from(src));
    }

    /// Snapshot of all versioned sources fetched this session, sorted by key.
    pub fn versioned_sources_snapshot(&self) -> Vec<(Arc<str>, Arc<str>)> {
        let map = self.versioned_sources.read().unwrap();
        let mut entries: Vec<_> = map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }

    /// Restrict versioned-namespace resolution to embedded builtin sources
    /// (no git).  Called by AOT harness binaries, which embed every pinned
    /// source discovered at compile time.
    pub fn set_versioned_offline(&self, offline: bool) {
        self.versioned_offline.store(offline, Ordering::Relaxed);
    }

    /// True when versioned namespaces may only come from embedded sources.
    pub fn versioned_offline(&self) -> bool {
        self.versioned_offline.load(Ordering::Relaxed)
    }

    /// Record the git commit a native (Rust-backed) package was built from.
    /// Called at registration time (`Registry::set_provenance` or the
    /// `register_provenance!` inventory entry in cljrs-interop).
    pub fn set_native_provenance(&self, ns: &str, commit: &str) {
        self.native_provenance
            .write()
            .unwrap()
            .insert(Arc::from(ns), Arc::from(commit));
    }

    /// The recorded provenance commit for a native package's namespace.
    pub fn native_provenance_for(&self, ns: &str) -> Option<Arc<str>> {
        self.native_provenance.read().unwrap().get(ns).cloned()
    }

    /// Make pinned-native provenance mismatches hard errors.
    pub fn set_enforce_native_versions(&self, enforce: bool) {
        self.enforce_native_versions
            .store(enforce, Ordering::Relaxed);
    }

    /// True when pinned-native provenance mismatches are errors.
    pub fn enforce_native_versions(&self) -> bool {
        self.enforce_native_versions.load(Ordering::Relaxed)
    }

    /// Install the pinned-native package loader (called once by
    /// `cljrs_dylib::install`; first writer wins).
    pub fn set_pinned_native_loader(&self, loader: PinnedNativeLoader) {
        let mut guard = self.pinned_native_loader.write().unwrap();
        if guard.is_none() {
            *guard = Some(loader);
        }
    }

    /// Install the native-dependency `require` loader (called once by
    /// `cljrs_dylib::install`; first writer wins).
    pub fn set_native_require_loader(&self, loader: NativeRequireLoader) {
        let mut guard = self.native_require_loader.write().unwrap();
        if guard.is_none() {
            *guard = Some(loader);
        }
    }

    /// If `:verify-commit-signatures` is enabled, verify that `commit` inside
    /// `repo_root` carries a valid GPG or SSH signature.
    ///
    /// Returns `Ok(())` immediately when the feature is off.  On the happy
    /// path the result is cached per `(repo_root, commit)` so each commit is
    /// only verified once per session.  On failure returns
    /// `EvalError::CommitSignatureVerificationFailed`.
    pub fn check_commit_signature(&self, repo_root: &str, commit: &str) -> EvalResult<()> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if !self.verify_commit_signatures.load(Ordering::Relaxed) {
                return Ok(());
            }
            let key = (Arc::<str>::from(repo_root), Arc::<str>::from(commit));
            if self.sig_verify_cache.lock().unwrap().contains(&key) {
                return Ok(());
            }
            let trusted = self.trusted_keys.read().unwrap().clone();
            cljrs_vcs::verify_commit_signature(std::path::Path::new(repo_root), commit, &trusted)
                .map_err(|e| match e {
                cljrs_vcs::VcsError::SignatureVerificationFailed { commit: c, reason } => {
                    crate::env::error::EvalError::CommitSignatureVerificationFailed {
                        commit: c,
                        reason,
                    }
                }
                other => crate::env::error::EvalError::Runtime(format!("{other}")),
            })?;
            self.sig_verify_cache.lock().unwrap().insert(key);
        }
        let _ = (repo_root, commit);
        Ok(())
    }

    /// Build the trusted-signer key set from a parsed `cljrs.edn` config and
    /// install it, so subsequent `check_commit_signature` calls verify against
    /// it.  Inline keys are parsed directly; `File` entries are read from disk.
    /// Returns the number of keys loaded; warns (to stderr) on any key that
    /// fails to load rather than aborting.  (Not available on wasm.)
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_trusted_signers(&self, config: &cljrs_deps::DepsConfig) -> usize {
        let mut keys = cljrs_vcs::TrustedKeys::new();
        let mut loaded = 0usize;
        for signer in &config.trusted_signers {
            let result = match signer {
                cljrs_deps::TrustedSigner::Inline(text) => keys.add_key_text(text),
                cljrs_deps::TrustedSigner::File(path) => match std::fs::read_to_string(path) {
                    Ok(text) => keys.add_key_text(&text),
                    Err(e) => {
                        eprintln!(
                            "cljrs: warning: could not read trusted signer key {}: {e}",
                            path.display()
                        );
                        continue;
                    }
                },
            };
            match result {
                Ok(()) => loaded += 1,
                Err(e) => eprintln!("cljrs: warning: invalid trusted signer key: {e}"),
            }
        }
        *self.trusted_keys.write().unwrap() = Arc::new(keys);
        loaded
    }
}

// ── Env ───────────────────────────────────────────────────────────────────────

/// The full execution environment: a stack of local frames plus the global env.
pub struct Env {
    pub frames: Vec<Frame>,
    pub current_ns: Arc<str>,
    pub globals: Arc<GlobalEnv>,
    /// When set, unversioned same-namespace symbol lookups implicitly resolve
    /// at this commit hash instead of HEAD.  Set by the versioned resolver when
    /// evaluating a function body fetched from git history.
    pub versioned_eval_commit: Option<Arc<str>>,
    /// True when evaluating the body of an `^:async` function.
    /// Set by `cljrs-async`; allows the `await` special form to know whether
    /// to yield (async context) or block the OS thread (sync context).
    pub is_async: bool,
}

impl Env {
    pub fn new(globals: Arc<GlobalEnv>, ns: &str) -> Self {
        Self {
            frames: Vec::new(),
            current_ns: Arc::from(ns),
            globals,
            versioned_eval_commit: None,
            is_async: false,
        }
    }

    /// Create an Env for evaluating source at a specific commit.
    pub fn new_versioned(globals: Arc<GlobalEnv>, ns: &str, commit: &str) -> Self {
        Self {
            versioned_eval_commit: Some(Arc::from(commit)),
            ..Self::new(globals, ns)
        }
    }

    /// Create an Env pre-loaded with a function's closed-over bindings.
    pub fn with_closure(globals: Arc<GlobalEnv>, ns: &str, f: &CljxFn) -> Self {
        let mut env = Self::new(globals, ns);
        if !f.closed_over_names.is_empty() {
            env.push_frame();
            for (name, val) in f.closed_over_names.iter().zip(f.closed_over_vals.iter()) {
                env.bind(name.clone(), val.clone());
            }
        }
        env
    }

    pub fn push_frame(&mut self) {
        self.frames.push(Frame::new());
    }

    pub fn pop_frame(&mut self) {
        self.frames.pop();
    }

    /// Bind `name` to `val` in the top frame.
    pub fn bind(&mut self, name: Arc<str>, val: Value) {
        if let Some(frame) = self.frames.last_mut() {
            frame.bind(name, val);
        }
        // If there are no frames, the binding is silently dropped.
        // Callers must push a frame first.
    }

    /// Look up `name`: local frames (innermost first), then the current namespace.
    pub fn lookup(&self, name: &str) -> Option<Value> {
        feat_trace!("env", "lookup {} in {} frames", name, self.frames.len());
        for frame in self.frames.iter().rev() {
            if let Some(v) = frame.lookup(name) {
                return Some(v.clone());
            }
        }
        self.globals.lookup_in_ns(&self.current_ns, name)
    }

    /// Look up `name` in local frames only — does **not** fall back to the
    /// global namespace.  Used by the versioned resolver to check for local
    /// bindings before applying commit inheritance.
    pub fn lookup_local_frames(&self, name: &str) -> Option<Value> {
        for frame in self.frames.iter().rev() {
            if let Some(v) = frame.lookup(name) {
                return Some(v.clone());
            }
        }
        None
    }

    /// Look up the Var object for `name` in the current namespace.
    pub fn lookup_var(&self, name: &str) -> Option<GcPtr<Var>> {
        self.globals.lookup_var_in_ns(&self.current_ns, name)
    }

    /// Collect all current local bindings (all frames, innermost last).
    /// Used for closure capture.
    pub fn all_local_bindings(&self) -> (Vec<Arc<str>>, Vec<Value>) {
        let mut names = Vec::new();
        let mut vals = Vec::new();
        // Outermost first so inner frames override on lookup.
        for frame in &self.frames {
            for (n, v) in &frame.bindings {
                names.push(n.clone());
                vals.push(v.clone());
            }
        }
        (names, vals)
    }

    /// Create a child Env for closure capture (same globals, same ns, captures locals).
    pub fn child(&self) -> Self {
        let (names, vals) = self.all_local_bindings();
        let mut child = Self::new(self.globals.clone(), &self.current_ns);
        child.is_async = self.is_async;
        if !names.is_empty() {
            child.push_frame();
            for (n, v) in names.into_iter().zip(vals) {
                child.bind(n, v);
            }
        }
        child
    }

    #[inline(always)]
    pub fn eval(&mut self, form: &Form) -> EvalResult {
        let globals = self.globals.clone();
        globals.eval(form, self)
    }

    #[inline(always)]
    pub fn call_cljrs_fn(&mut self, func: &CljxFn, args: &[Value]) -> EvalResult {
        let globals = self.globals.clone();
        globals.call_cljrs_fn(func, args, self)
    }

    #[inline(always)]
    pub fn on_fn_defined(&mut self, func: &CljxFn) {
        let globals = self.globals.clone();
        globals.on_fn_defined(func, self);
    }
}
