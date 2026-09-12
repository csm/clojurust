//! Dynamic execution policy for restricted evaluator profiles.

use std::cell::Cell;

use crate::env::error::{EvalError, EvalResult};

thread_local! {
    static TRANSACTION_DEPTH: Cell<usize> = const { Cell::new(0) };
    static TRANSACTION_GENSYM: Cell<u64> = const { Cell::new(0) };
}

/// Installs the side-effect-free transaction policy for its dynamic extent.
#[must_use = "dropping the guard removes the transaction execution policy"]
pub struct TransactionPolicyGuard;

impl TransactionPolicyGuard {
    pub fn install() -> Self {
        TRANSACTION_DEPTH.with(|depth| {
            if depth.get() == 0 {
                TRANSACTION_GENSYM.with(|counter| counter.set(0));
            }
            depth.set(depth.get() + 1);
        });
        Self
    }
}

impl Drop for TransactionPolicyGuard {
    fn drop(&mut self) {
        TRANSACTION_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

pub fn transaction_policy_active() -> bool {
    TRANSACTION_DEPTH.with(|depth| depth.get() != 0)
}

/// Return an invocation-local deterministic gensym sequence number when the
/// transaction policy is active.
pub fn next_transaction_gensym() -> Option<u64> {
    if !transaction_policy_active() {
        return None;
    }
    Some(TRANSACTION_GENSYM.with(|counter| {
        let current = counter.get();
        counter.set(current.wrapping_add(1));
        current
    }))
}

fn forbidden(operation: &str) -> EvalError {
    EvalError::ForbiddenEffect(operation.to_string())
}

/// Check a native builtin at its final call boundary.
///
/// The transaction environment registers only clojurust's builtins, so this
/// denylist is the capability surface: filesystem, output, clocks, randomness,
/// process-global state, blocking/concurrency, and Rust object construction.
pub fn check_native(name: &str) -> EvalResult<()> {
    if !transaction_policy_active() {
        return Ok(());
    }
    const DENIED: &[&str] = &[
        "print",
        "println",
        "pr",
        "prn",
        "printf",
        "newline",
        "flush",
        "spit",
        "slurp",
        "close",
        "nanotime",
        "sleep",
        "rand",
        "rand-int",
        "random-sample",
        "shuffle",
        "random-uuid",
        "gensym",
        "add-tap",
        "remove-tap",
        "tap>",
        "shared-atom",
        "promise",
        "deliver",
        "send",
        "send-off",
        "new",
        "Exception.",
        "push-precision!",
        "pop-precision!",
        // Reads process-global state the transaction did not receive as an
        // argument, and which can change under it between calls.
        "System/getenv",
    ];
    if DENIED.contains(&name) {
        Err(forbidden(name))
    } else {
        Ok(())
    }
}

/// Check special forms that can reach outside the invocation environment.
pub fn check_special(name: &str) -> EvalResult<()> {
    if !transaction_policy_active() {
        return Ok(());
    }
    const DENIED: &[&str] = &[
        ".",
        "ns",
        "require",
        "in-ns",
        "alias",
        "load-file",
        "with-out-str",
        "await",
    ];
    if DENIED.contains(&name) {
        Err(forbidden(name))
    } else {
        Ok(())
    }
}

pub fn check_versioned_lookup() -> EvalResult<()> {
    if transaction_policy_active() {
        Err(forbidden("versioned namespace lookup"))
    } else {
        Ok(())
    }
}
