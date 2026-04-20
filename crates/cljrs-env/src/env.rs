//! Lexical environment: local frames, global namespace table, and current Env.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use cljrs_gc::{GcConfig, GcPtr};
use cljrs_reader::Form;
use cljrs_value::{CljxFn, Namespace, Value, Var};
use cljrs_logging::feat_trace;
use crate::error::EvalResult;
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
    pub namespaces: RwLock<HashMap<Arc<str>, GcPtr<Namespace>>>,
    /// Directories to search when resolving namespace names to files.
    pub source_paths: RwLock<Vec<std::path::PathBuf>>,
    /// Namespaces that have been fully loaded from a file (idempotent guard).
    pub loaded: Mutex<HashSet<Arc<str>>>,
    /// Namespaces currently being loaded (cycle detection).
    pub loading: Mutex<HashSet<Arc<str>>>,
    /// Built-in namespace sources embedded in the binary.
    /// Checked by `load_ns` before falling back to source-path search.
    pub builtin_sources: RwLock<HashMap<Arc<str>, &'static str>>,
    /// GC configuration for automatic collection based on memory pressure.
    pub gc_config: RwLock<Option<Arc<GcConfig>>>,
    /// True once the Clojure compiler namespaces have been loaded and IR
    /// lowering is available.  Before this, all functions use tree-walking.
    pub compiler_ready: std::sync::atomic::AtomicBool,
    /// Evaluator function. Evaluates form given env, produces an EvalResult.
    pub eval_fn: fn(&Form, &mut Env) -> EvalResult,
    /// Call a cljrs function.
    pub call_cljrs_fn: fn(&CljxFn, &[Value], &mut Env) -> EvalResult,
    /// Hook for customization when a new fn* is defined.
    on_fn_defined: Option<fn(&CljxFn, &mut Env)>,
}

impl std::fmt::Debug for GlobalEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GlobalEnv {{ ... }}")
    }
}

impl GlobalEnv {
    pub fn new(
        eval_fn: fn(&Form, &mut Env) -> EvalResult,
        call_cljrs_fn: fn(&CljxFn, &[Value], &mut Env) -> EvalResult,
        on_fn_defined: Option<fn(&CljxFn, &mut Env)>,
    ) -> Arc<Self> {
        Arc::new(Self {
            namespaces: RwLock::new(HashMap::new()),
            source_paths: RwLock::new(Vec::new()),
            loaded: Mutex::new(HashSet::new()),
            loading: Mutex::new(HashSet::new()),
            builtin_sources: RwLock::new(HashMap::new()),
            gc_config: RwLock::new(None),
            compiler_ready: std::sync::atomic::AtomicBool::new(false),
            eval_fn,
            call_cljrs_fn,
            on_fn_defined,
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
                return crate::dynamics::deref_var(var);
            }
        }
        // Then refers.
        {
            let refers = ns_ref.refers.lock().unwrap();
            if let Some(var) = refers.get(sym_name) {
                return crate::dynamics::deref_var(var);
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
                dst_refers
                    .entry(name.clone())
                    .or_insert_with(|| var.clone());
            }
        }
    }

    /// Register `alias` → `full_ns` in `current_ns`'s alias table.
    pub fn add_alias(&self, current_ns: &str, alias: &str, full_ns: &str) {
        let ns_ptr = self.get_or_create_ns(current_ns);
        let mut aliases = ns_ptr.get().aliases.lock().unwrap();
        aliases.insert(Arc::from(alias), Arc::from(full_ns));
    }

    /// Evaluate form given env.
    #[inline(always)]
    pub fn eval(&self, form: &Form, env: &mut Env) -> EvalResult {
        (self.eval_fn)(form, env)
    }

    /// Call the given cljrs function.
    #[inline(always)]
    pub fn call_cljrs_fn(&self, func: &CljxFn, args: &[Value], env: &mut Env) -> EvalResult {
        (self.call_cljrs_fn)(func, args, env)
    }

    /// Callback hook for new functions defined.
    #[inline(always)]
    pub fn on_fn_defined(&self, f: &CljxFn, env: &mut Env) {
        if let Some(hook) = self.on_fn_defined {
            hook(f, env);
        }
    }

    /// Sets the on-new-function-defined hook.
    pub fn set_on_fn_defined(&mut self, hook: fn(&CljxFn, &mut Env)) {
        self.on_fn_defined = Some(hook);
    }
}

// ── Env ───────────────────────────────────────────────────────────────────────

/// The full execution environment: a stack of local frames plus the global env.
pub struct Env {
    pub frames: Vec<Frame>,
    pub current_ns: Arc<str>,
    pub globals: Arc<GlobalEnv>,
}

impl Env {
    pub fn new(globals: Arc<GlobalEnv>, ns: &str) -> Self {
        Self {
            frames: Vec::new(),
            current_ns: Arc::from(ns),
            globals,
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
        if !names.is_empty() {
            child.push_frame();
            for (n, v) in names.into_iter().zip(vals.into_iter()) {
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
