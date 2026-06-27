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
    RtImport {
        name: "rt_deopt",
        params: &[],
        results: &[I32],
    },
    // ic_slot, key_ptr, key_len → fills a per-call-site keyword cache
    RtImport {
        name: "rt_kw_ic_fill",
        params: &[I32, I32, I64],
        results: &[I32],
    },
    // ic_slot, callee, args_ptr, argc
    RtImport {
        name: "rt_call_ic",
        params: &[I32, I32, I32, I64],
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
    fn no_duplicate_imports() {
        for (i, a) in RT_IMPORTS.iter().enumerate() {
            for b in &RT_IMPORTS[i + 1..] {
                assert_ne!(a.name, b.name, "duplicate rt import {}", a.name);
            }
        }
    }
}
