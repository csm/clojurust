//! Lexical environment: local frames, global namespace table, and current Env.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use cljx_gc::GcPtr;
use cljx_value::{CljxFn, Namespace, Value, Var};

// ── Frame ─────────────────────────────────────────────────────────────────────

/// One stack frame of local bindings (a single `let*`, `fn`, or `loop*` scope).
pub struct Frame {
    bindings: Vec<(Arc<str>, Value)>,
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
}

impl GlobalEnv {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            namespaces: RwLock::new(HashMap::new()),
        })
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
    pub fn lookup_in_ns(&self, ns_name: &str, sym_name: &str) -> Option<Value> {
        let map = self.namespaces.read().unwrap();
        let ns = map.get(ns_name)?;
        let ns_ref = ns.get();
        // Check interns first.
        {
            let interns = ns_ref.interns.lock().unwrap();
            if let Some(var) = interns.get(sym_name) {
                return var.get().deref();
            }
        }
        // Then refers.
        {
            let refers = ns_ref.refers.lock().unwrap();
            if let Some(var) = refers.get(sym_name) {
                return var.get().deref();
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
            dst_refers
                .entry(name.clone())
                .or_insert_with(|| var.clone());
        }
    }
}

// ── Env ───────────────────────────────────────────────────────────────────────

/// The full execution environment: a stack of local frames plus the global env.
pub struct Env {
    frames: Vec<Frame>,
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
}
