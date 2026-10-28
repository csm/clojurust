//! Every `thread_local!` in the workspace is classified as per-mutator
//! (swapped with each fiber) or per-thread / per-isolate (shared by every
//! fiber on the thread).  See docs/virtual-threads-plan.md, "Per-mutator
//! thread-local state".
//!
//! A thread-local that holds the state of *a running computation* must be a
//! `cljrs_gc::mutator_slot!`, or fibers sharing the thread corrupt each
//! other's state.  This test fails when a new thread-local appears without a
//! row below, so whoever adds one has to decide which kind it is.

// Slots are collected at link time, so every crate that declares one must be
// linked into this test even though nothing here calls into it.
extern crate cljrs_compiler;
extern crate cljrs_runtime;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

enum Class {
    /// Per-mutator: registered as this `MutatorSlot` name.
    Fiber(&'static str),
    /// Per-mutator, but only compiled in some configurations (the slot is
    /// checked only when it exists).
    FiberWhen(&'static str, bool),
    /// Shared by every fiber on the thread; the reason it is safe.
    Thread(&'static str),
}
use Class::*;

const NO_GC: bool = cfg!(feature = "no-gc");

/// `(file relative to crates/, static name, class)`.
const CLASSIFICATION: &[(&str, &str, Class)] = &[
    // ── cljrs-gc ────────────────────────────────────────────────────────────
    (
        "cljrs-gc/src/lib.rs",
        "ALLOC_ROOTS",
        FiberWhen("cljrs-gc/ALLOC_ROOTS", !NO_GC),
    ),
    (
        "cljrs-gc/src/lib.rs",
        "ISOLATE_HEAP",
        Thread("the isolate's heap"),
    ),
    (
        "cljrs-gc/src/region.rs",
        "REGION_STACK",
        Fiber("cljrs-gc/REGION_STACK"),
    ),
    (
        "cljrs-gc/src/region.rs",
        "RETIRED_REGIONS",
        Thread("isolate-wide root set"),
    ),
    (
        "cljrs-gc/src/region.rs",
        "POISON_WATERMARK",
        FiberWhen("cljrs-gc/POISON_WATERMARK", !NO_GC),
    ),
    (
        "cljrs-gc/src/alloc_ctx.rs",
        "ALLOC_CTX",
        FiberWhen("cljrs-gc/ALLOC_CTX", NO_GC),
    ),
    (
        "cljrs-gc/src/governor.rs",
        "CRITICAL_DEPTH",
        Thread("critical sections never park (mutator::assert_parkable)"),
    ),
    (
        "cljrs-gc/src/governor.rs",
        "FAULT",
        Thread("isolate-wide memory fault"),
    ),
    (
        "cljrs-gc/src/governor.rs",
        "CURRENT",
        Thread("the isolate's memory account"),
    ),
    (
        "cljrs-gc/src/config.rs",
        "ISOLATE_CANCELLATION",
        Thread("isolate-wide"),
    ),
    (
        "cljrs-gc/src/mutator.rs",
        "GUARDS_HELD",
        Thread("zero at every park (mutator::assert_parkable)"),
    ),
    (
        "cljrs-gc/src/mutator.rs",
        "TEST_STACK",
        Thread("unit-test fixture"),
    ),
    (
        "cljrs-gc/src/mutator.rs",
        "TEST_COUNTER",
        Thread("unit-test fixture"),
    ),
    // ── cljrs-runtime ───────────────────────────────────────────────────────
    (
        "cljrs-runtime/src/env/gc_roots.rs",
        "ENV_ROOTS",
        Fiber("cljrs-runtime/ENV_ROOTS"),
    ),
    (
        "cljrs-runtime/src/env/gc_roots.rs",
        "VALUE_ROOTS",
        Fiber("cljrs-runtime/VALUE_ROOTS"),
    ),
    (
        "cljrs-runtime/src/env/gc_roots.rs",
        "OPTION_VALUE_ROOTS",
        Fiber("cljrs-runtime/OPTION_VALUE_ROOTS"),
    ),
    (
        "cljrs-runtime/src/env/dynamics.rs",
        "BINDING_STACK",
        Fiber("cljrs-runtime/BINDING_STACK"),
    ),
    (
        "cljrs-runtime/src/env/callback.rs",
        "EVAL_CONTEXT",
        Fiber("cljrs-runtime/EVAL_CONTEXT"),
    ),
    (
        "cljrs-runtime/src/env/depth.rs",
        "CALL_DEPTH",
        Fiber("cljrs-runtime/CALL_DEPTH"),
    ),
    (
        "cljrs-runtime/src/env/gas.rs",
        "ACTIVE",
        Fiber("cljrs-runtime/GAS_ACTIVE"),
    ),
    (
        "cljrs-runtime/src/env/gas.rs",
        "EXHAUSTED",
        Fiber("cljrs-runtime/GAS_EXHAUSTED"),
    ),
    (
        "cljrs-runtime/src/env/gas.rs",
        "INTERRUPTS",
        Fiber("cljrs-runtime/GAS_INTERRUPTS"),
    ),
    (
        "cljrs-runtime/src/env/policy.rs",
        "TRANSACTION_DEPTH",
        Fiber("cljrs-runtime/TRANSACTION_DEPTH"),
    ),
    (
        "cljrs-runtime/src/env/policy.rs",
        "TRANSACTION_GENSYM",
        Fiber("cljrs-runtime/TRANSACTION_GENSYM"),
    ),
    (
        "cljrs-runtime/src/env/taps.rs",
        "TAP",
        Thread("isolate-wide tap set"),
    ),
    (
        "cljrs-runtime/src/builtins/builtins.rs",
        "OUTPUT_CAPTURE",
        Fiber("cljrs-runtime/OUTPUT_CAPTURE"),
    ),
    (
        "cljrs-runtime/src/builtins/builtins.rs",
        "BIG_DECIMAL_SCALE",
        Fiber("cljrs-runtime/BIG_DECIMAL_SCALE"),
    ),
    (
        "cljrs-runtime/src/interp/apply.rs",
        "WATCH_ERROR",
        Fiber("cljrs-runtime/WATCH_ERROR"),
    ),
    (
        "cljrs-runtime/src/tiered/apply.rs",
        "IR_LOWERING_ACTIVE",
        Fiber("cljrs-runtime/IR_LOWERING_ACTIVE"),
    ),
    (
        "cljrs-runtime/src/tiered/jit_state.rs",
        "MY_FRAMES",
        Fiber("cljrs-runtime/MY_FRAMES"),
    ),
    // ── cljrs-compiler ──────────────────────────────────────────────────────
    (
        "cljrs-compiler/src/rt_abi.rs",
        "RT_REGION_STACK",
        Fiber("cljrs-compiler/RT_REGION_STACK"),
    ),
    (
        "cljrs-compiler/src/rt_abi.rs",
        "RT_SCRATCH",
        Thread("only live between two adjacent rt_abi calls, which never park"),
    ),
    (
        "cljrs-compiler/src/rt_abi.rs",
        "PENDING_EXCEPTION",
        Fiber("cljrs-compiler/PENDING_EXCEPTION"),
    ),
    (
        "cljrs-compiler/src/rt_abi.rs",
        "IC_TRACER_REGISTERED",
        Thread("process-wide flag"),
    ),
    // ── other crates ────────────────────────────────────────────────────────
    (
        "cljrs/src/session.rs",
        "ASYNC_DRIVER",
        Thread("the isolate's async driver"),
    ),
    (
        "cljrs-value/src/symbol.rs",
        "INTERNED",
        Thread("interning cache"),
    ),
    (
        "cljrs-ir/src/lib.rs",
        "INDENT",
        Thread("debug-print indentation"),
    ),
    (
        "cljrs-wasm/src/dom/events.rs",
        "ACTIVE_EVENT",
        Thread("wasm32: no fibers"),
    ),
    (
        "cljrs-wasm/src/dom/mod.rs",
        "DOM_GLOBALS",
        Thread("wasm32: no fibers"),
    ),
    (
        "cljrs-wasm/src/dom/fns.rs",
        "MEMORY",
        Thread("wasm32: no fibers"),
    ),
    (
        "cljrs-wasm/src/dom/fns.rs",
        "MEMORY_NEXT_ID",
        Thread("wasm32: no fibers"),
    ),
];

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `src` with every comment and the contents of every string and char
/// literal blanked to spaces (newlines kept), so a `thread_local!` in a doc
/// example or a brace inside a string cannot be mistaken for code.
fn code_only(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let blank = |c: char| if c == '\n' { '\n' } else { ' ' };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            // Block comments nest in Rust.
            let mut depth = 0usize;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(blank(chars[i]));
                    i += 1;
                }
            }
        } else if c == 'r' && matches!(next, Some('"') | Some('#')) && {
            let hashes = chars[i + 1..].iter().take_while(|&&h| h == '#').count();
            chars.get(i + 1 + hashes) == Some(&'"')
        } {
            // Raw string: r"…", r#"…"#, …
            let hashes = chars[i + 1..].iter().take_while(|&&h| h == '#').count();
            let open = 2 + hashes;
            out.extend(std::iter::repeat_n(' ', open));
            i += open;
            while i < chars.len() {
                if chars[i] == '"'
                    && chars[i + 1..].iter().take_while(|&&h| h == '#').count() >= hashes
                {
                    out.extend(std::iter::repeat_n(' ', 1 + hashes));
                    i += 1 + hashes;
                    break;
                }
                out.push(blank(chars[i]));
                i += 1;
            }
        } else if c == '"' {
            out.push(' ');
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    out.push(' ');
                    i += 1;
                }
                if i < chars.len() {
                    out.push(blank(chars[i]));
                    i += 1;
                }
            }
            out.push(' ');
            i += 1;
        } else if c == '\'' {
            // A char literal ('x', '\n', '\u{…}'), not a lifetime ('a).
            let len = if next == Some('\\') {
                chars[i + 2..]
                    .iter()
                    .position(|&ch| ch == '\'')
                    .map(|p| p + 3)
            } else if chars.get(i + 2) == Some(&'\'') {
                Some(3)
            } else {
                None
            };
            match len {
                Some(len) => {
                    out.extend(std::iter::repeat_n(' ', len));
                    i += len;
                }
                None => {
                    out.push(c);
                    i += 1;
                }
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Names of the statics declared inside each `thread_local! { … }` block.
fn thread_local_statics(src: &str) -> Vec<String> {
    let code = code_only(src);
    let mut names = Vec::new();
    let mut rest = code.as_str();
    while let Some(start) = rest.find("thread_local!") {
        let after = &rest[start + "thread_local!".len()..];
        let Some(open) = after.find(['{', '(']) else {
            break;
        };
        let (open_ch, close_ch) = if after.as_bytes()[open] == b'{' {
            ('{', '}')
        } else {
            ('(', ')')
        };
        let mut depth = 0usize;
        let mut end = after.len();
        for (i, ch) in after[open..].char_indices() {
            if ch == open_ch {
                depth += 1;
            } else if ch == close_ch {
                depth -= 1;
                if depth == 0 {
                    end = open + i;
                    break;
                }
            }
        }
        let body = &after[open..end];
        let mut words = body.split(|c: char| !(c.is_alphanumeric() || c == '_'));
        while let Some(word) = words.next() {
            if word == "static"
                && let Some(ident) = words.find(|w| !w.is_empty())
            {
                names.push(ident.to_string());
            }
        }
        rest = &after[end..];
    }
    names
}

#[test]
fn scanner_ignores_comments_and_literals() {
    let src = r##"
//! ```ignore
//! thread_local! {
//!     static IN_DOC: RefCell<u8> = RefCell::new(0);
//! }
//! ```
/* thread_local! { static IN_BLOCK: u8 = 0; } /* nested */ */
const S: &str = "thread_local! { static IN_STRING: u8 = 0; }";
const R: &str = r#"thread_local! { static IN_RAW: u8 = 0; }"#;
const C: char = '{';
fn f<'a>(_: &'a str) {}
thread_local! {
    // static IN_LINE_COMMENT: u8 = 0;
    static REAL_ONE: Cell<u8> = const { Cell::new(0) }; // trailing
    /// docs
    pub(crate) static REAL_TWO: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}
"##;
    assert_eq!(thread_local_statics(src), vec!["REAL_ONE", "REAL_TWO"]);
}

#[test]
fn every_thread_local_is_classified() {
    let crates = crates_dir();
    let mut found = BTreeSet::new();
    let mut entries = std::fs::read_dir(&crates)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect::<Vec<_>>();
    entries.sort();
    for krate in entries {
        let mut files = Vec::new();
        rust_files(&krate.join("src"), &mut files);
        for file in files {
            let src = std::fs::read_to_string(&file).unwrap();
            let rel = file
                .strip_prefix(&crates)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            for name in thread_local_statics(&src) {
                found.insert((rel.clone(), name));
            }
        }
    }

    let classified: BTreeSet<(String, String)> = CLASSIFICATION
        .iter()
        .map(|(file, name, _)| (file.to_string(), name.to_string()))
        .collect();

    let unclassified: Vec<_> = found.difference(&classified).collect();
    assert!(
        unclassified.is_empty(),
        "unclassified thread-locals {unclassified:?}: add a row to CLASSIFICATION — \
         per-mutator state must also be registered with cljrs_gc::mutator_slot!"
    );
    let stale: Vec<_> = classified.difference(&found).collect();
    assert!(
        stale.is_empty(),
        "CLASSIFICATION rows for thread-locals that no longer exist: {stale:?}"
    );
}

#[test]
fn per_mutator_thread_locals_are_registered_slots() {
    let registered: BTreeSet<&str> = cljrs_gc::mutator::slot_names().into_iter().collect();
    let mut expected = BTreeSet::new();
    for (file, name, class) in CLASSIFICATION {
        let slot = match class {
            Fiber(slot) => slot,
            FiberWhen(slot, true) => slot,
            FiberWhen(_, false) => continue,
            Thread(reason) => {
                assert!(!reason.is_empty(), "{file}: {name} needs a reason");
                continue;
            }
        };
        // Slots in crates this binary doesn't link can't be checked here.
        if file.starts_with("cljrs-wasm/") {
            continue;
        }
        assert!(
            registered.contains(slot),
            "{file}: {name} is classified per-mutator but slot `{slot}` is not registered"
        );
        expected.insert(*slot);
    }
    let unexpected: Vec<_> = registered.difference(&expected).collect();
    assert!(
        unexpected.is_empty(),
        "registered mutator slots with no CLASSIFICATION row: {unexpected:?}"
    );
}
