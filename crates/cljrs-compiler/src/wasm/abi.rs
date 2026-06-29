//! The wasm backend's ABI contract: how IR values, the `rt_abi` runtime bridge,
//! and regions map onto WebAssembly types.
//!
//! This is the canonical, written-down contract that [`super::emit`] consumes.
//! It is deliberately data-only (no `wasm-encoder` dependency) so the contract
//! can be reviewed and tested independently of the encoder.
//!
//! # Value representation
//!
//! The native backend passes Clojure values as `*const Value` (a host pointer)
//! and unboxed scalars as raw `i64`/`f64`.  Under `wasm32` the mapping is:
//!
//! | IR / `rt_abi` Rust type         | wasm type        | Notes                                   |
//! |---------------------------------|------------------|-----------------------------------------|
//! | `*const Value` (`Repr::Boxed`)  | [`WasmValType::I32`] | linear-memory offset of the boxed value |
//! | `*mut Region` (region handle)   | [`WasmValType::I32`] | linear-memory offset of the `Region`    |
//! | `*const *const Value` (slices)  | [`WasmValType::I32`] | offset of a contiguous pointer array    |
//! | `i64` (`Repr::Long`)            | [`WasmValType::I64`] | unboxed long payload                    |
//! | `f64` (`Repr::Double`)         | [`WasmValType::F64`] | unboxed double payload                  |
//! | `u8`/`u32` (`Repr::Bool`, tags) | [`WasmValType::I32`] | small integers                          |
//! | `u64` (lengths/counts)          | [`WasmValType::I64`] | matches the native `extern "C"` shape   |
//!
//! Because every pointer collapses to an `i32`, the entire `rt_abi` surface
//! ([`crate::rt_abi`], ~165 `extern "C"` functions) is expressible as wasm
//! imports with no marshalling beyond width changes.  The GC heap and all
//! regions live in the module's single linear memory.
//!
//! # Region ABI (bump allocation)
//!
//! Region inference runs host-side at lowering time and is identical for both
//! backends.  The runtime mechanism in wasm:
//!
//! - A **region handle** is an `i32` — the linear-memory offset of a `Region`
//!   returned by [`RT_REGION_START`].  A `Region` is an arena: a `(base, bump,
//!   limit)` triple in linear memory; allocation bumps `bump`, and
//!   [`RT_REGION_END`] resets/frees it.
//! - [`Inst::RegionStart`](crate::ir::Inst::RegionStart) → call `rt_region_start`, keep the `i32` handle.
//! - [`Inst::RegionAlloc`](crate::ir::Inst::RegionAlloc) → call the matching
//!   `rt_region_alloc_*` with the handle as the leading `i32` arg.
//! - [`Inst::RegionEnd`](crate::ir::Inst::RegionEnd) → call `rt_region_end` with the handle.
//! - [`Inst::RegionParam`](crate::ir::Inst::RegionParam) → bind the function's **hidden trailing `i32`
//!   parameter** (present iff [`IrFunction::takes_region_param`](crate::ir::IrFunction::takes_region_param)).
//! - [`Inst::CallWithRegion`](crate::ir::Inst::CallWithRegion) → ordinary direct call, passing the caller's
//!   region handle as the trailing `i32` argument.
//!
//! So the compiled signature of a region-parameterised variant is its visible
//! params plus one trailing `i32` — exactly mirroring
//! [`IrFunction::abi_param_count`](crate::ir::IrFunction::abi_param_count) on the native side.

/// A WebAssembly value type, restricted to the four the backend uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WasmValType {
    /// 32-bit integer — every pointer/handle and small integer.
    I32,
    /// 64-bit integer — unboxed `Repr::Long` and `u64` lengths.
    I64,
    /// 64-bit float — unboxed `Repr::Double`.
    F64,
}

use crate::ir::Repr;

impl WasmValType {
    /// The wasm type carrying an IR value of the given machine representation.
    pub fn for_repr(repr: Repr) -> WasmValType {
        match repr {
            Repr::Long => WasmValType::I64,
            Repr::Double => WasmValType::F64,
            // Boxed values, booleans (i8 payload), and array handles are all
            // carried as i32 (a linear-memory offset or small integer).
            Repr::Boxed | Repr::Bool | Repr::LongArray | Repr::DoubleArray => WasmValType::I32,
        }
    }
}

/// One imported `rt_abi` runtime function, described as a wasm function type.
///
/// `name` is the `#[no_mangle]` symbol from [`crate::rt_abi`]; the emitter
/// imports it from the `"rt"` module so the runtime (compiled to the same
/// linear memory) satisfies it at instantiation.
#[derive(Debug, Clone, Copy)]
pub struct RtImport {
    /// The `rt_abi` symbol name.
    pub name: &'static str,
    /// Parameter types, in order.
    pub params: &'static [WasmValType],
    /// Result types (wasm permits multiple; `rt_abi` uses 0 or 1).
    pub results: &'static [WasmValType],
}

use WasmValType::{F64, I32, I64};

// Region-handle name constants so call sites read clearly.
pub const RT_REGION_START: &str = "rt_region_start";
pub const RT_REGION_END: &str = "rt_region_end";

/// The shared indirect function table the AOT module imports from the runtime.
///
/// A closure's `fn_ptr` is, under `wasm32`, a table index; the runtime's dynamic
/// dispatch (`rt_call` → `call_indirect`) calls through that same table, so both
/// modules must share one.  The emitter imports it here, mirroring the imported
/// `"rt" "memory"`, and installs its defined functions into it with an active
/// `funcref` element segment.
pub const FUNC_TABLE_NAME: &str = "__indirect_function_table";

/// Table slot at which the AOT module installs its defined functions.
///
/// The element segment is placed at this base, so the function pointer (table
/// index) for the defined function at bundle position `p` is
/// `FUNC_TABLE_BASE + p`.  The runtime must reserve `[FUNC_TABLE_BASE, …)` of its
/// table for the AOT functions — the table analogue of the rodata coordination
/// the string constants need; the concrete base is finalized in the CLI/bundling
/// step (the CLI/bundling item).  `0` is the validation-time placeholder.
pub const FUNC_TABLE_BASE: u32 = 0;

/// Linear-memory offset at which the AOT module installs its read-only data
/// pool (string / keyword / symbol constant bytes).
///
/// The emitter accumulates every `Const::Str` / `Const::Keyword` /
/// `Const::Symbol`'s UTF-8 bytes into one deduplicated pool and emits it as a
/// single active data segment at this base, so a constant at pool offset `o`
/// resolves to the `(ptr, len)` pair `(RODATA_BASE + o, len)` passed to
/// `rt_const_string` / `_keyword` / `_symbol`.  This is the linear-memory
/// analogue of [`FUNC_TABLE_BASE`]: the runtime must reserve `[RODATA_BASE, …)`
/// for the AOT data, and the concrete base is finalized against the runtime's
/// actual memory layout in the CLI/bundling step.  `0` is the
/// validation-time placeholder.
pub const RODATA_BASE: u32 = 0;

/// The subset of the `rt_abi` bridge the scaffold wires as wasm imports.
///
/// This is the contract, not the whole table: it covers safepoints, constant
/// materialization, unboxed arithmetic/comparison bridges, GC-heap and region
/// allocation, and the inline-cache / deopt bridges.  Completing it to all of
/// [`crate::rt_abi`] is mechanical — each `extern "C"` signature maps to one
/// `RtImport` by the rules in the module docs.
pub const RT_IMPORTS: &[RtImport] = &[
    // ── Safepoint ──────────────────────────────────────────────────────────
    RtImport {
        name: "rt_safepoint",
        params: &[],
        results: &[],
    },
    // ── Constant materialization ─────────────────────────────────────────────
    RtImport {
        name: "rt_const_nil",
        params: &[],
        results: &[I32],
    },
    RtImport {
        name: "rt_const_true",
        params: &[],
        results: &[I32],
    },
    RtImport {
        name: "rt_const_false",
        params: &[],
        results: &[I32],
    },
    RtImport {
        name: "rt_const_long",
        params: &[I64],
        results: &[I32],
    },
    RtImport {
        name: "rt_const_double",
        params: &[F64],
        results: &[I32],
    },
    RtImport {
        name: "rt_const_char",
        params: &[I32],
        results: &[I32],
    },
    // ptr, len → boxed string/keyword/symbol
    RtImport {
        name: "rt_const_string",
        params: &[I32, I64],
        results: &[I32],
    },
    RtImport {
        name: "rt_const_keyword",
        params: &[I32, I64],
        results: &[I32],
    },
    RtImport {
        name: "rt_const_symbol",
        params: &[I32, I64],
        results: &[I32],
    },
    // ── Truthiness ───────────────────────────────────────────────────────────
    RtImport {
        name: "rt_truthiness",
        params: &[I32],
        results: &[I32],
    },
    // ── Boxed arithmetic / comparison bridges ────────────────────────────────
    RtImport {
        name: "rt_add",
        params: &[I32, I32],
        results: &[I32],
    },
    RtImport {
        name: "rt_sub",
        params: &[I32, I32],
        results: &[I32],
    },
    RtImport {
        name: "rt_mul",
        params: &[I32, I32],
        results: &[I32],
    },
    RtImport {
        name: "rt_div",
        params: &[I32, I32],
        results: &[I32],
    },
    RtImport {
        name: "rt_rem",
        params: &[I32, I32],
        results: &[I32],
    },
    RtImport {
        name: "rt_eq",
        params: &[I32, I32],
        results: &[I32],
    },
    RtImport {
        name: "rt_lt",
        params: &[I32, I32],
        results: &[I32],
    },
    RtImport {
        name: "rt_gt",
        params: &[I32, I32],
        results: &[I32],
    },
    RtImport {
        name: "rt_lte",
        params: &[I32, I32],
        results: &[I32],
    },
    RtImport {
        name: "rt_gte",
        params: &[I32, I32],
        results: &[I32],
    },
    // ── Scratch buffer (marshalling arrays for alloc bridges) ────────────────
    // n_bytes → linear-memory offset of a thread-local scratch buffer at least
    // `n_bytes` wide.  The wasm backend stores a contiguous array of element
    // `*const Value` pointers here before calling the slice-taking `rt_alloc_*`
    // / `rt_region_alloc_*` bridges (the wasm analogue of the native backend's
    // explicit stack slot).
    RtImport {
        name: "rt_scratch_ptr",
        params: &[I32],
        results: &[I32],
    },
    // ── GC-heap allocation (slice ptr, n) ────────────────────────────────────
    RtImport {
        name: "rt_alloc_vector",
        params: &[I32, I64],
        results: &[I32],
    },
    RtImport {
        name: "rt_alloc_map",
        params: &[I32, I64],
        results: &[I32],
    },
    RtImport {
        name: "rt_alloc_set",
        params: &[I32, I64],
        results: &[I32],
    },
    RtImport {
        name: "rt_alloc_list",
        params: &[I32, I64],
        results: &[I32],
    },
    RtImport {
        name: "rt_alloc_cons",
        params: &[I32, I32],
        results: &[I32],
    },
    // ── Region allocation (bump) ─────────────────────────────────────────────
    RtImport {
        name: RT_REGION_START,
        params: &[],
        results: &[I32],
    },
    RtImport {
        name: RT_REGION_END,
        params: &[I32],
        results: &[I32],
    },
    // handle, slice ptr, n
    RtImport {
        name: "rt_region_alloc_vector",
        params: &[I32, I32, I64],
        results: &[I32],
    },
    RtImport {
        name: "rt_region_alloc_map",
        params: &[I32, I32, I64],
        results: &[I32],
    },
    RtImport {
        name: "rt_region_alloc_set",
        params: &[I32, I32, I64],
        results: &[I32],
    },
    RtImport {
        name: "rt_region_alloc_list",
        params: &[I32, I32, I64],
        results: &[I32],
    },
    // handle, head, tail
    RtImport {
        name: "rt_region_alloc_cons",
        params: &[I32, I32, I32],
        results: &[I32],
    },
    // ── Globals / vars ───────────────────────────────────────────────────────
    // All take `(ns_ptr, ns_len, name_ptr, name_len)` with the name bytes living
    // in the rodata pool.  `rt_load_global` resolves a namespaced binding to its
    // value (versioned `name@sha` names are handled inside the bridge, uncached —
    // the per-call-site versioned IC is deferred with `rt_call_ic`);
    // `rt_load_var` returns the Var object itself (for `set!`/`binding`);
    // `rt_def_var` interns the var with the given value; `rt_set_bang` mutates a
    // Var's binding (its `*const Value` result is dropped).
    RtImport {
        name: "rt_load_global",
        params: &[I32, I64, I32, I64],
        results: &[I32],
    },
    RtImport {
        name: "rt_load_var",
        params: &[I32, I64, I32, I64],
        results: &[I32],
    },
    RtImport {
        name: "rt_def_var",
        params: &[I32, I64, I32, I64, I32],
        results: &[I32],
    },
    RtImport {
        name: "rt_set_bang",
        params: &[I32, I32],
        results: &[I32],
    },
    // ── Exceptions (thread-local error path) ─────────────────────────────────
    // `rt_throw(exc)` stashes the exception in a thread-local and returns nil;
    // the throwing block then falls into its `unreachable`/return terminator.
    // `rt_try(body, catch, finally)` invokes the body thunk, routes a pending
    // thread-local exception into the catch thunk, and always runs the finally
    // thunk — all three are boxed closures.  This mirrors the Cranelift
    // backend; the wasm exception-handling proposal (`try`/`catch`/`throw`,
    // gated on `WasmBackend::exceptions`) is a deferred alternative.
    RtImport {
        name: "rt_throw",
        params: &[I32],
        results: &[I32],
    },
    RtImport {
        name: "rt_try",
        params: &[I32, I32, I32],
        results: &[I32],
    },
    // ── A couple of common collection ops ────────────────────────────────────
    RtImport {
        name: "rt_get",
        params: &[I32, I32],
        results: &[I32],
    },
    RtImport {
        name: "rt_count",
        params: &[I32],
        results: &[I32],
    },
    // ── Calls ────────────────────────────────────────────────────────────────
    // rt_call(callee, args_ptr, nargs) → boxed result.  Dynamic dispatch through
    // a boxed callable value; `args_ptr` is a contiguous array of `*const Value`
    // (marshalled through the scratch buffer), `nargs` the element count.  A
    // zero-arg call passes a null `args_ptr` and a zero count.
    RtImport {
        name: "rt_call",
        params: &[I32, I32, I64],
        results: &[I32],
    },
    // ── Closure construction ─────────────────────────────────────────────────
    // rt_make_fn(name_ptr, name_len, fn_ptr, param_count, captures_ptr, ncaptures)
    // `fn_ptr` is a table index (a wasm32 function pointer); `captures_ptr` is a
    // contiguous array of `*const Value` (i32 each), marshalled through scratch.
    RtImport {
        name: "rt_make_fn",
        params: &[I32, I64, I32, I64, I32, I64],
        results: &[I32],
    },
    // Same shape as rt_make_fn; the fixed param count excludes the rest param.
    RtImport {
        name: "rt_make_fn_variadic",
        params: &[I32, I64, I32, I64, I32, I64],
        results: &[I32],
    },
    // rt_make_fn_multi(name_ptr, name_len, fn_ptrs, param_counts, is_variadic,
    //                  n_arities, captures_ptr, ncaptures).  `fn_ptrs` is an array
    // of i32 table indices, `param_counts` an array of u64, `is_variadic` an array
    // of u8 — all marshalled contiguously through the scratch buffer.
    RtImport {
        name: "rt_make_fn_multi",
        params: &[I32, I64, I32, I32, I32, I64, I32, I64],
        results: &[I32],
    },
    // ── Specialization / inline-cache / deopt bridges (Phase 10.6) ───────────
    RtImport {
        name: "rt_value_tag",
        params: &[I32],
        results: &[I32],
    },
    RtImport {
        name: "rt_unbox_long",
        params: &[I32],
        results: &[I64],
    },
    RtImport {
        name: "rt_unbox_double",
        params: &[I32],
        results: &[F64],
    },
    RtImport {
        name: "rt_box_bool",
        params: &[I32],
        results: &[I32],
    },
    // Boxed → unboxed coercion for the typed-parameter ABI's boxed-entry
    // trampoline.  `rt_coerce_long`/`rt_coerce_double` honor a static
    // `^long`/`^double` hint when a boxed caller (dynamic dispatch, the shared
    // table, cross-function `CallDirect`) reaches the trampoline: they coerce a
    // number (or throw via the thread-local pending-exception slot), there being
    // no deopt seam in the sandbox.
    RtImport {
        name: "rt_coerce_long",
        params: &[I32],
        results: &[I64],
    },
    RtImport {
        name: "rt_coerce_double",
        params: &[I32],
        results: &[F64],
    },
    RtImport {
        name: "rt_deopt",
        params: &[],
        results: &[I32],
    },
    // Boxed integer-overflow exception, raised by unboxed checked `+`/`-`/`*`
    // when the `i64` result overflows (Clojure primitive-long semantics) — the
    // wasm analogue of `codegen.rs::emit_long_overflow_check`.
    RtImport {
        name: "rt_overflow_error",
        params: &[],
        results: &[I32],
    },
    // ic_slot, key_ptr, key_len → fills a per-call-site keyword cache
    RtImport {
        name: "rt_kw_ic_fill",
        params: &[I32, I32, I64],
        results: &[I32],
    },
    // rt_call_ic(callee, args_ptr, nargs, ic_slot) — `rt_call` with a per-call-site
    // inline cache for protocol dispatch.  `ic_slot` is a writable 8-byte
    // linear-memory slot (an index into the runtime's IC entry table, never a GC
    // pointer).  Wiring this in wasm needs the emitter to own a writable rodata /
    // IC region coordinated with the runtime's memory layout — the same data-segment
    // work the string/keyword/symbol constants need — so the emitter currently
    // lowers `Inst::Call` through plain `rt_call` and keeps this for that follow-up.
    RtImport {
        name: "rt_call_ic",
        params: &[I32, I32, I64, I32],
        results: &[I32],
    },
];

/// Look up an `rt_abi` import descriptor by symbol name.
pub fn lookup(name: &str) -> Option<&'static RtImport> {
    RT_IMPORTS.iter().find(|i| i.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointers_are_i32() {
        assert_eq!(WasmValType::for_repr(Repr::Boxed), WasmValType::I32);
        assert_eq!(WasmValType::for_repr(Repr::Bool), WasmValType::I32);
        assert_eq!(WasmValType::for_repr(Repr::LongArray), WasmValType::I32);
    }

    #[test]
    fn unboxed_scalars_map_to_native_wasm_types() {
        assert_eq!(WasmValType::for_repr(Repr::Long), WasmValType::I64);
        assert_eq!(WasmValType::for_repr(Repr::Double), WasmValType::F64);
    }

    #[test]
    fn region_contract_present_and_well_typed() {
        let start = lookup(RT_REGION_START).expect("rt_region_start in table");
        assert_eq!(start.params, &[] as &[WasmValType]);
        assert_eq!(start.results, &[WasmValType::I32]); // handle is an i32

        let end = lookup(RT_REGION_END).expect("rt_region_end in table");
        assert_eq!(end.params, &[WasmValType::I32]); // takes the handle

        // Region alloc threads the handle as the leading i32 arg.
        let v = lookup("rt_region_alloc_vector").expect("rt_region_alloc_vector in table");
        assert_eq!(v.params.first(), Some(&WasmValType::I32));
        assert_eq!(v.results, &[WasmValType::I32]);
    }

    #[test]
    fn closure_constructors_present_and_well_typed() {
        // rt_make_fn / _variadic share the (name_ptr, name_len, fn_ptr,
        // param_count, captures, ncaptures) shape and return a boxed value.
        for name in ["rt_make_fn", "rt_make_fn_variadic"] {
            let rt = lookup(name).unwrap_or_else(|| panic!("{name} in table"));
            assert_eq!(rt.params, &[I32, I64, I32, I64, I32, I64]);
            assert_eq!(rt.results, &[I32]);
        }
        let multi = lookup("rt_make_fn_multi").expect("rt_make_fn_multi in table");
        assert_eq!(multi.params, &[I32, I64, I32, I32, I32, I64, I32, I64]);
        assert_eq!(multi.results, &[I32]);
    }

    #[test]
    fn no_duplicate_imports() {
        for (i, a) in RT_IMPORTS.iter().enumerate() {
            for b in &RT_IMPORTS[i + 1..] {
                assert_ne!(a.name, b.name, "duplicate rt import {}", a.name);
            }
        }
    }
}
