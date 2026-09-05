//! The one construction path for a clojurust runtime.
//!
//! Before this stage every layer had its own "standard environment"
//! constructor — `cljrs_runtime::interp::standard_env{,_minimal,_with_paths}`,
//! `cljrs_runtime::tiered::standard_env{,_minimal,_minimal_no_ir,_with_paths}`, and
//! `cljrs_stdlib::standard_env{,_no_ir,_with_paths,_with_paths_and_config}`.
//! They differed in which `fn` pointers they installed, whether they enabled
//! IR lowering, and which of GC config / root tracer / source paths they
//! remembered to set, and callers had to know which one matched their needs.
//!
//! There is now one: [`Runtime::builder`].  Execution mode, source paths, GC
//! configuration, embedded namespace sources, and tier enablement are all
//! builder inputs.  Extensions install themselves into a finished runtime —
//! `cljrs_stdlib::install(&runtime)` and friends.
//!
//! ```no_run
//! use cljrs_runtime::{ExecutionMode, Runtime};
//!
//! let runtime = Runtime::builder()
//!     .execution_mode(ExecutionMode::Tiered)
//!     .source_paths(vec!["src".into()])
//!     .build()
//!     .expect("bootstrap");
//!
//! // Extensions install into the finished runtime; this package cannot name
//! // them (they depend on it), so the call is shown rather than compiled:
//! //     cljrs_stdlib::install(&runtime);
//! let mut env = runtime.env("user");
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use cljrs_gc::GcConfig;

use crate::builtins::builtins;
use crate::env::env::{Env, GlobalEnv};
use crate::interp::{eval, special};
use crate::mode::{ExecutionMode, TierState};

/// Why a runtime could not be built.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// An embedded source that the builder evaluates could not be parsed.
    /// This means the binary's own bootstrap text is broken.
    #[error("failed to parse embedded source {origin}: {message}")]
    EmbeddedSource { origin: String, message: String },
}

/// A constructed runtime: an environment plus the execution mode that decides
/// how its code runs.
///
/// Cheap to clone — every runtime instance's state lives in the shared
/// [`GlobalEnv`], so clones name the same runtime rather than a new one.
#[derive(Clone, Debug)]
pub struct Runtime {
    globals: Arc<GlobalEnv>,
}

impl Runtime {
    /// Start configuring a runtime.
    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder::new()
    }

    /// Adopt an already-constructed environment.
    ///
    /// For code that is handed an `Arc<GlobalEnv>` (an AOT harness, an
    /// embedding host, a native package loader) and needs a [`Runtime`] to
    /// pass to an extension's `install`.
    pub fn from_globals(globals: Arc<GlobalEnv>) -> Self {
        Self { globals }
    }

    /// The environment this runtime evaluates in.
    pub fn globals(&self) -> &Arc<GlobalEnv> {
        &self.globals
    }

    /// Take ownership of the environment handle.
    pub fn into_globals(self) -> Arc<GlobalEnv> {
        self.globals
    }

    /// A fresh evaluation context in namespace `ns`.
    pub fn env(&self, ns: &str) -> Env {
        Env::new(self.globals.clone(), ns)
    }

    /// How this runtime executes function calls.
    pub fn execution_mode(&self) -> ExecutionMode {
        self.globals.execution_mode()
    }

    /// Which tiers are live right now.
    pub fn tier_state(&self) -> TierState {
        self.globals.tier_state()
    }
}

/// Configuration for [`Runtime::builder`].
pub struct RuntimeBuilder {
    execution_mode: ExecutionMode,
    source_paths: Vec<PathBuf>,
    gc_config: Option<Arc<GcConfig>>,
    gc_config_from_env: bool,
    register_gc_roots: bool,
    builtin_sources: Vec<(String, &'static str)>,
    eager_clojure_test: bool,
}

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeBuilder {
    pub fn new() -> Self {
        Self {
            execution_mode: ExecutionMode::default(),
            source_paths: Vec::new(),
            gc_config: None,
            gc_config_from_env: true,
            register_gc_roots: true,
            builtin_sources: Vec::new(),
            eager_clojure_test: false,
        }
    }

    /// Select how the runtime executes function calls.  Defaults to
    /// [`ExecutionMode::Tiered`].
    pub fn execution_mode(mut self, mode: ExecutionMode) -> Self {
        self.execution_mode = mode;
        self
    }

    /// Directories searched when `require` resolves a namespace to a file.
    pub fn source_paths(mut self, paths: Vec<PathBuf>) -> Self {
        self.source_paths = paths;
        self
    }

    /// Explicit GC limits.  Without this the heap is configured from the
    /// environment (`CLJRS_GC_*`), unless [`Self::gc_config_from_env`] is off.
    pub fn gc_config(mut self, config: Arc<GcConfig>) -> Self {
        self.gc_config = Some(config);
        self
    }

    /// Whether to apply `CLJRS_GC_*` environment settings to the heap.
    /// On by default; an explicit [`Self::gc_config`] is applied after it.
    pub fn gc_config_from_env(mut self, enabled: bool) -> Self {
        self.gc_config_from_env = enabled;
        self
    }

    /// Whether to register this runtime's namespace table as a GC root set.
    /// On by default.  The tracer holds a weak handle, so the runtime is
    /// still collected when the last [`Runtime`] handle drops.
    pub fn register_gc_roots(mut self, enabled: bool) -> Self {
        self.register_gc_roots = enabled;
        self
    }

    /// Embed a namespace's source in the runtime, so `require` resolves it
    /// without a file on the source path.
    pub fn builtin_source(mut self, ns: impl Into<String>, src: &'static str) -> Self {
        self.builtin_sources.push((ns.into(), src));
        self
    }

    /// Evaluate `clojure.test` during construction instead of leaving it to
    /// the first `require`.
    ///
    /// Only useful without an extension that embeds `clojure.test` lazily
    /// (`cljrs-stdlib` does); tests inside this package rely on it.
    pub fn eager_clojure_test(mut self, enabled: bool) -> Self {
        self.eager_clojure_test = enabled;
        self
    }

    /// Bootstrap the runtime.
    ///
    /// Registers native `clojure.core`, evaluates the bootstrap Clojure
    /// source, applies GC and source-path configuration, and finally raises
    /// the tier state to what the execution mode targets — the bootstrap
    /// itself always tree-walks, because nothing can be lowered before
    /// `clojure.core` exists.
    pub fn build(self) -> Result<Runtime, BuildError> {
        let globals = GlobalEnv::new(self.execution_mode);

        // Native clojure.core, then a `user` namespace referring it.
        builtins::register_all(&globals, "clojure.core");
        globals.get_or_create_ns("user");
        globals.refer_core("user");

        // Bootstrap Clojure source (higher-order fns defined in Clojure).
        eval_embedded(&globals, builtins::BOOTSTRAP_SOURCE, "<bootstrap>")?;

        // Re-refer clojure.core now that the bootstrap has defined its HOFs.
        globals.refer_core("user");
        globals.mark_loaded("clojure.core");

        // Experimental namespace: natives first, then the Clojure source that
        // builds on them. Not referred into `user` — an explicit `require` is
        // the only way in, so a feature probe for `clojure.core/future` and
        // friends still (correctly) finds nothing.
        builtins::register_experimental(&globals, builtins::EXPERIMENTAL_NS);
        eval_embedded(
            &globals,
            builtins::EXPERIMENTAL_SOURCE,
            "<cljrs.core.experimental>",
        )?;
        globals.mark_loaded(builtins::EXPERIMENTAL_NS);

        for (ns, src) in &self.builtin_sources {
            globals.register_builtin_source(ns, src);
        }

        if self.eager_clojure_test {
            eval_embedded(&globals, builtins::CLOJURE_TEST_SOURCE, "<clojure.test>")?;
            globals.mark_loaded("clojure.test");
        }

        if !self.source_paths.is_empty() {
            globals.set_source_paths(self.source_paths);
        }

        if self.gc_config_from_env {
            cljrs_gc::HEAP.set_config_from_env();
        }
        if let Some(config) = self.gc_config {
            globals.set_gc_config(config.clone());
            cljrs_gc::HEAP.set_config(config);
        }
        if self.register_gc_roots {
            register_namespace_roots(&globals);
        }

        // `*ns*` is `user` — loading above may have moved it.
        special::sync_star_ns(&mut Env::new(globals.clone(), "user"));

        // Bootstrap is over: raise the tiers this mode targets.  `CLJRS_NO_IR`
        // pins the runtime at tree-walk regardless of mode.
        if std::env::var("CLJRS_NO_IR").is_err() {
            let target = self.execution_mode.target_tier();
            if target.ir_enabled() {
                // Functions defined before this point (the clojure.core
                // bootstrap) stay excluded from background lowering.
                globals
                    .jit()
                    .set_bootstrap_watermark(crate::interp::arity::next_arity_id());
            }
            globals.set_tier_state(target);
        }

        Ok(Runtime { globals })
    }
}

/// Parse and evaluate an embedded source text in `clojure.core`.
///
/// A parse failure is fatal (the binary's own embedded text is broken); an
/// individual form failing to evaluate is reported and skipped, which is the
/// long-standing behavior of the bootstrap.
fn eval_embedded(globals: &Arc<GlobalEnv>, src: &str, origin: &str) -> Result<(), BuildError> {
    let mut parser = cljrs_reader::Parser::new(src.to_string(), origin.to_string());
    let forms = parser.parse_all().map_err(|e| BuildError::EmbeddedSource {
        origin: origin.to_string(),
        message: format!("{e:?}"),
    })?;
    let mut env = Env::new(globals.clone(), "clojure.core");
    for form in forms {
        let _alloc_frame = cljrs_gc::push_alloc_frame();
        if let Err(e) = eval::eval(&form, &mut env) {
            eprintln!("[{origin} warning] {}: {:?}", form.span.start, e);
        }
    }
    Ok(())
}

/// Register the runtime's namespace table as a GC root set.
///
/// The tracer holds a `Weak` handle: a runtime that is dropped stops being a
/// root instead of keeping itself alive forever through the heap's tracer
/// list, which matters now that a process can build several runtimes.
fn register_namespace_roots(globals: &Arc<GlobalEnv>) {
    let weak = Arc::downgrade(globals);
    cljrs_gc::HEAP.register_root_tracer(move |visitor| {
        use cljrs_gc::GcVisitor as _;
        let Some(globals) = weak.upgrade() else {
            return;
        };
        let namespaces = globals.namespaces.read().unwrap();
        for ns_ptr in namespaces.values() {
            visitor.visit(ns_ptr);
        }
    });
}
