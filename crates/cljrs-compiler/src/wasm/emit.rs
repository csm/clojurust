//! WebAssembly emitter.
//!
//! Walks a [`reloop::Structured`] tree and lowers each IR [`Inst`] to wasm,
//! producing a single-function module via `wasm-encoder`.  `rt_abi` symbols are
//! declared as imports from the `"rt"` module (see [`super::abi`]); the runtime,
//! compiled to the same linear memory, satisfies them at instantiation.
//!
//! # Value model (boxed-only, for now)
//!
//! Every IR [`VarId`] is a wasm `i32` local holding a boxed `*const Value`
//! linear-memory offset — the universal representation, always correct.  This
//! mirrors the Cranelift backend's boxed fallback (`codegen.rs` materializes
//! unboxed scalars only when `typeinfer` proves a `Long`/`Double`/`Bool` repr;
//! the `_ =>` arm boxes through `rt_const_*`).  Unboxed specialization that
//! aligns the wasm signature with [`function_signature`] is a follow-up.  The
//! hidden trailing region param of a region-parameterised callee variant *is*
//! honored (it is always an `i32`); only the async poll-function ABI
//! (state-machine params) is still rejected with [`WasmError::Unsupported`].
//!
//! # Control flow
//!
//! The relooper already produced structured control flow.  This pass maps it
//! directly: [`Structured::Labeled`] → `block`, [`Structured::Loop`] → `loop`,
//! [`Structured::If`] → `if`/`else`, and [`Structured::Br`] → `br N`, where the
//! depth `N` is resolved from a stack of the enclosing control frames.  A
//! forward `Br` targets an enclosing `block` (break); a backward `Br` targets an
//! enclosing `loop` (continue).
//!
//! # SSA φ resolution
//!
//! No `phi` instruction is emitted.  On each edge into a block, the φ's incoming
//! value for that predecessor is copied into the φ's local.  Copies use the
//! wasm operand stack for parallel-move semantics (all `local.get`s before any
//! `local.set`), so a swapping `recur` cannot clobber.  Copies happen at the
//! `Br` site for merge/loop targets and at block entry for inlined
//! single-predecessor edges.
//!
//! # Multi-function modules
//!
//! [`emit_bundle`] compiles several [`IrFunction`]s into one module so that a
//! [`Inst::CallDirect`]/[`Inst::CallWithRegion`] can resolve its callee to a
//! wasm function index.  In wasm, imported functions occupy the low function-index
//! space (`0..k`) and defined functions follow (`k..k+n`), so the *number of
//! imports* must be settled before any `call` to a defined function can be encoded.
//! The emitter therefore runs **two passes**: pass 1 lowers every body into a
//! throwaway buffer purely to discover the `rt_abi` imports each one needs; pass 2
//! re-lowers with `func_base = imports.len()` known, so `CallDirect` targets
//! resolve to their final absolute indices.  Emission is deterministic, so the
//! import set is identical across passes.
//!
//! # Closures and the function table
//!
//! [`Inst::AllocClosure`] materializes a closure through `rt_make_fn` /
//! `rt_make_fn_variadic` / `rt_make_fn_multi`.  An arity function's pointer is,
//! under `wasm32`, a **table index**, so the module imports the runtime's shared
//! indirect function table (`"rt" "__indirect_function_table"`, mirroring the
//! imported memory) and installs every defined function into it with an active
//! `funcref` element segment at [`abi::FUNC_TABLE_BASE`]; the slot for the
//! function at bundle position `p` is `FUNC_TABLE_BASE + p`.  The closure name
//! bytes, the captured-value pointer array, and (multi-arity) the fn-pointer /
//! param-count / variadic-flag arrays are all marshalled contiguously through a
//! single `rt_scratch_ptr` reservation — sidestepping the data-segment /
//! memory-layout coordination by writing the constant bytes into scratch at
//! call time.
//!
//! # Cross-function tail calls
//!
//! A block whose trailing instruction is a direct call
//! ([`Inst::CallDirect`]/[`Inst::CallWithRegion`]) whose result is the
//! function's return value is lowered to a `return_call` when
//! [`WasmBackend::tail_calls`] is set, reclaiming the caller's frame before the
//! callee runs (the callee's `[i32; abi_param_count] → [i32]` signature matches
//! this function's result).  With `tail_calls` off — or for dynamic `Call`s,
//! which dispatch through the `rt_call` import — the ordinary `call` + `return`
//! is emitted instead (correct, but not constant-stack; a trampoline is the
//! deferred alternative).
//!
//! # String / keyword / symbol constants
//!
//! [`Const::Str`]/[`Const::Keyword`]/[`Const::Symbol`] intern their UTF-8 bytes
//! into a deduplicated read-only data pool ([`ModuleAsm::rodata`]), emitted as a
//! single active data segment at [`abi::RODATA_BASE`] in the runtime's imported
//! memory.  A constant resolves to the `(ptr, len)` pair `(RODATA_BASE + offset,
//! len)` passed to `rt_const_string`/`_keyword`/`_symbol`.  Mirrors
//! `codegen.rs::emit_string_const` (the native backend defines an anonymous data
//! object per string); keywords skip the per-call-site inline cache, which is
//! deferred with the rest of the IC work.  The pool base is the linear-memory
//! analogue of [`abi::FUNC_TABLE_BASE`] and is finalized against the runtime's
//! memory layout in the CLI/bundling step.
//!
//! # Status
//!
//! Emits valid, `wasmparser`-validated modules for the subset: scalar
//! constants, string/keyword/symbol constants (via the rodata data segment),
//! `LoadLocal`, boxed arithmetic (`+ - * / rem`, folded) and binary
//! comparison (`= < > <= >=`) via the `rt_abi` bridges, collection allocation
//! (`AllocVector`/`AllocMap`/`AllocSet`/`AllocList`/`AllocCons` — element arrays
//! marshalled through an imported linear memory and the `rt_scratch_ptr`
//! buffer), region operations (`RegionStart`/`RegionAlloc`/`RegionEnd` →
//! `rt_region_*`, and `RegionParam` → the hidden trailing-`i32` param), calls
//! (`CallDirect`/`CallWithRegion` → a direct `call` to the resolved wasm index;
//! `Call` → dynamic dispatch through `rt_call` with arguments marshalled through
//! the scratch buffer), closures (`AllocClosure` → `rt_make_fn*` over the shared
//! function table), globals/vars (`LoadGlobal`/`LoadVar`/`DefVar`/`SetBang` →
//! the `rt_load_global`/`rt_load_var`/`rt_def_var`/`rt_set_bang` bridges with
//! ns/name bytes drawn from the rodata pool), exceptions (`Throw` → `rt_throw`
//! and `KnownFn::TryCatchFinally` → `rt_try`, the thread-local error path),
//! cross-function tail calls (`return_call`), and all control flow (branches,
//! diamonds, and `loop`/`recur` with φ).  The `rt_call_ic` inline cache (needs a
//! writable IC data region), the wasm exception-handling proposal (gated on
//! `WasmBackend::exceptions`; the thread-local path is always used for now), and
//! the async ABI return [`WasmError::Unsupported`] — the next lowering
//! increments.
//!
//! # Unboxed scalar values
//!
//! [`typeinfer::infer`] assigns each [`VarId`] an unboxed [`Repr`]
//! (`Long`→`i64`, `Double`→`f64`, `Bool`→`i32` 0/1) where the boxed bridge's
//! exact semantics are expressible on the raw representation, so a value's wasm
//! local takes that machine type and intermediate scalar arithmetic compiles to
//! native `i64`/`f64` ops instead of the boxing `rt_*` bridges (each of which
//! allocates or interns on the GC heap).  A value is **boxed only where it flows
//! into a boxed context** — a call argument, collection element, `return`, a
//! boxed φ, a global/var bridge — via [`Emitter::get`] (which boxes an unboxed
//! local on demand); unboxed operands are read with [`Emitter::get_i64`] /
//! [`Emitter::get_f64`].  Checked `+`/`-` on `Long` emit the same signed-overflow
//! branch the native backend does (`rt_overflow_error` + `rt_throw`, then an
//! early boxed-`nil` `return`); `Long` `*` (which needs a 128-bit overflow check
//! wasm cannot express without `i64.mul_hi`) and every other non-trivial unboxed
//! producer are **demoted back to boxed** by [`refine_reprs`], so the repr map
//! the emitter consumes only ever marks a value unboxed when the emitter can
//! produce it unboxed.
//!
//! # Typed parameter ABI
//!
//! A function with `^long`/`^double` parameter hints (`seed_reprs`, see
//! [`is_typed`]) compiles to **two** wasm functions: a *typed body* whose hinted
//! params are unboxed `i64`/`f64` (so the body reads them with no per-use unbox),
//! and a boxed-entry **trampoline** ([`emit_trampoline`]) with the all-`i32`
//! signature every dispatcher expects.  The trampoline is the function's primary
//! entry — exported, installed in the shared table, and the target of every
//! `CallDirect` — so all the existing always-boxed dispatch paths (dynamic
//! `rt_call`, indirect closure calls, cross-function direct calls) reach a typed
//! function unchanged; the trampoline coerces each boxed argument
//! (`rt_coerce_long`/`rt_coerce_double`) and (tail-)calls the body.  There is no
//! in-sandbox deopt seam, so a violated static hint *coerces or throws* rather
//! than re-dispatching at Tier 1.  Passing unboxed arguments *directly* on a
//! same-bundle `CallDirect` (skipping the trampoline for the caller-side win) is
//! a further optimization left for later.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, ElementSection, Elements, EntityType,
    ExportKind, ExportSection, Function, FunctionSection, Ieee64, ImportSection, Instruction,
    MemArg, MemoryType, Module, RefType, TableType, TypeSection, ValType,
};

use crate::ir::{
    Block, BlockId, ClosureTemplate, Const, Inst, IrFunction, KnownFn, RegionAllocKind, Repr,
    Terminator, VarId,
};

use super::abi::{self, RtImport, WasmValType};
use super::reloop::Structured;
use super::{WasmBackend, WasmError};
use crate::typeinfer;

/// Emit a wasm module containing `func` as a single exported function.
///
/// A thin wrapper over [`emit_bundle`] for the common single-function case.
pub fn emit_function(
    func: &IrFunction,
    structured: &Structured,
    cfg: &WasmBackend,
) -> Result<Vec<u8>, WasmError> {
    emit_bundle(&[(func, structured)], cfg)
}

/// Emit a wasm module containing every `(func, structured)` in `funcs`, each
/// exported under its name, so that a [`Inst::CallDirect`]/[`Inst::CallWithRegion`]
/// can resolve its callee to that function's wasm index.
///
/// See the module docs ("Multi-function modules") for why this runs two passes:
/// imported functions occupy the low function-index space, so the import count
/// must be settled before a `call` to a *defined* function can be encoded.
pub fn emit_bundle(
    funcs: &[(&IrFunction, &Structured)],
    cfg: &WasmBackend,
) -> Result<Vec<u8>, WasmError> {
    for (func, _) in funcs {
        if func.takes_state_param() {
            return Err(WasmError::Unsupported(
                "async poll-function ABI (state-machine params)".into(),
            ));
        }
    }

    // Map each named function to its position among the *defined* functions.
    // The absolute wasm index is `func_base + position`, where `func_base` is the
    // import count (settled after pass 1).  Direct calls resolve through this.
    let mut func_index_of: HashMap<&str, u32> = HashMap::new();
    for (i, (func, _)) in funcs.iter().enumerate() {
        if let Some(name) = func.name.as_deref() {
            func_index_of.insert(name, i as u32);
        }
    }

    // A function with `^long`/`^double` param hints ([`is_typed`]) compiles to a
    // boxed-entry trampoline (its *primary* wasm function, at the same index a
    // non-typed function would occupy) plus a *typed body* appended after all
    // `n` primaries.  The `k`-th typed function's typed body is at wasm index
    // `func_base + n + k`; `typed_ordinal[p]` is that `k` for primary position
    // `p` (`None` for non-typed functions).
    let n = funcs.len();
    let typed: Vec<bool> = funcs.iter().map(|(f, _)| is_typed(f)).collect();
    let mut typed_ordinal: Vec<Option<usize>> = vec![None; n];
    let mut n_typed = 0usize;
    for (p, &is_t) in typed.iter().enumerate() {
        if is_t {
            typed_ordinal[p] = Some(n_typed);
            n_typed += 1;
        }
    }

    let mut asm = ModuleAsm::new();

    // Pass 1: lower every body into a throwaway buffer to discover the imports
    // each needs.  Defined-function calls (and the trampoline→typed-body call)
    // use provisional indices of 0; the bodies are discarded, so the wrong
    // indices never reach the module.  Both the trampoline and the typed body of
    // a typed function are emitted so their imports (e.g. `rt_coerce_long`) are
    // discovered before `func_base` is settled.
    for (p, (func, structured)) in funcs.iter().enumerate() {
        if typed[p] {
            let _ = emit_trampoline(func, &mut asm, 0, cfg)?;
            let _ = emit_one(func, structured, &mut asm, 0, &func_index_of, cfg, true)?;
        } else {
            let _ = emit_one(func, structured, &mut asm, 0, &func_index_of, cfg, false)?;
        }
    }

    // Imports are now fully discovered; defined functions start right after them.
    let func_base = asm.imports.len() as u32;

    // Pass 2: re-lower with the settled `func_base`, keeping the bodies + their
    // interned signature type indices.  Primaries (one per function, in order)
    // occupy `func_base..func_base+n`; typed bodies are appended after them.
    let mut primaries: Vec<(Function, u32)> = Vec::with_capacity(n);
    let mut typed_bodies: Vec<(Function, u32)> = Vec::with_capacity(n_typed);
    for (p, (func, structured)) in funcs.iter().enumerate() {
        if let Some(k) = typed_ordinal[p] {
            let typed_body_index = func_base + n as u32 + k as u32;
            primaries.push(emit_trampoline(func, &mut asm, typed_body_index, cfg)?);
            typed_bodies.push(emit_one(
                func,
                structured,
                &mut asm,
                func_base,
                &func_index_of,
                cfg,
                true,
            )?);
        } else {
            primaries.push(emit_one(
                func,
                structured,
                &mut asm,
                func_base,
                &func_index_of,
                cfg,
                false,
            )?);
        }
    }

    // The module's defined functions, in wasm-index order: all `n` primaries,
    // then the typed bodies.
    let bodies: Vec<&(Function, u32)> = primaries.iter().chain(typed_bodies.iter()).collect();

    // ── Assemble ─────────────────────────────────────────────────────────────
    let mut module = Module::new();

    let mut types = TypeSection::new();
    for (params, results) in &asm.types {
        types
            .ty()
            .function(params.iter().map(valtype), results.iter().map(valtype));
    }
    module.section(&types);

    let mut imports = ImportSection::new();
    for (name, ty_idx) in &asm.imports {
        imports.import("rt", name, EntityType::Function(*ty_idx));
    }
    if asm.needs_memory {
        // The runtime owns linear memory; the module imports it.  Memory lives
        // in its own index space, so this does not shift function indices.
        imports.import(
            "rt",
            "memory",
            EntityType::Memory(MemoryType {
                minimum: 1,
                maximum: None,
                memory64: false,
                shared: false,
                page_size_log2: None,
            }),
        );
    }
    if asm.needs_table {
        // The runtime owns the shared indirect function table; the module imports
        // it so a closure's `fn_ptr` (a table index) and the runtime's
        // `call_indirect` agree.  Tables have their own index space, so this does
        // not shift function indices.  The declared minimum covers the slots the
        // AOT functions occupy (`[FUNC_TABLE_BASE, +n)`).
        imports.import(
            "rt",
            abi::FUNC_TABLE_NAME,
            EntityType::Table(TableType {
                element_type: RefType::FUNCREF,
                table64: false,
                minimum: abi::FUNC_TABLE_BASE as u64 + n as u64,
                maximum: None,
                shared: false,
            }),
        );
    }
    module.section(&imports);

    let mut func_section = FunctionSection::new();
    for (_, ty_idx) in bodies.iter().copied() {
        func_section.function(*ty_idx);
    }
    module.section(&func_section);

    let mut exports = ExportSection::new();
    for (i, (func, _)) in funcs.iter().enumerate() {
        let name = func.name.as_deref().unwrap_or("main");
        exports.export(name, ExportKind::Func, func_base + i as u32);
    }
    module.section(&exports);

    if asm.needs_table {
        // Install every function's *primary* entry into the shared table at
        // `[FUNC_TABLE_BASE, +n)`, so a closure's `fn_ptr` for the function at
        // bundle position `p` is `FUNC_TABLE_BASE + p`.  The element *contents*
        // are wasm function indices (`func_base + p`); the *slots* are the table
        // base plus the position.  For a typed function the primary is its boxed
        // trampoline, so indirect (always-boxed) dispatch lands on the trampoline
        // — the appended typed bodies are never table-installed.  (Element
        // sections precede the code section.)
        let func_indices: Vec<u32> = (0..n as u32).map(|p| func_base + p).collect();
        let mut elements = ElementSection::new();
        elements.active(
            Some(0),
            &ConstExpr::i32_const(abi::FUNC_TABLE_BASE as i32),
            Elements::Functions(Cow::Owned(func_indices)),
        );
        module.section(&elements);
    }

    let mut code = CodeSection::new();
    for (body, _) in bodies.iter().copied() {
        code.function(body);
    }
    module.section(&code);

    if !asm.rodata.is_empty() {
        // The string/keyword/symbol constant pool, copied into the runtime's
        // imported memory at `[RODATA_BASE, +len)` at instantiation.  The data
        // section follows the code section in wasm's section order.
        let mut data = DataSection::new();
        data.active(
            0,
            &ConstExpr::i32_const(abi::RODATA_BASE as i32),
            asm.rodata.iter().copied(),
        );
        module.section(&data);
    }

    Ok(module.finish())
}

/// Lower one function's body to a [`Function`], registering any imports/types it
/// needs into the shared `asm`.  Returns the body and its interned signature
/// type index.  `func_base` is the absolute wasm index of the first defined
/// function (the import count); `func_index_of` maps a callee name to its
/// position among the defined functions for [`Inst::CallDirect`] resolution.
fn emit_one(
    func: &IrFunction,
    structured: &Structured,
    asm: &mut ModuleAsm,
    func_base: u32,
    func_index_of: &HashMap<&str, u32>,
    cfg: &WasmBackend,
    typed: bool,
) -> Result<(Function, u32), WasmError> {
    // The ABI param count includes the hidden trailing region handle (an `i32`)
    // when this is a region-parameterised callee variant — mirroring
    // [`IrFunction::abi_param_count`].  Visible params occupy wasm locals
    // `0..nparams`; the region param, if any, is the next local and is bound by
    // [`Inst::RegionParam`].
    let nparams = func.params.len();
    let abi_nparams = func.abi_param_count();
    let region_param_local = func.takes_region_param().then_some(nparams as u32);

    // Per-VarId machine representation: `Long`→`i64`, `Double`→`f64`,
    // everything else (boxed pointers, `Bool` 0/1, region handles)→`i32`.  The
    // refined map only marks a value unboxed when the emitter can produce it
    // unboxed (see [`refine_reprs`]).
    //
    // For a *typed* body the inference is seeded with the function's static
    // `^long`/`^double` parameter hints (`seed_reprs`), so the hinted params
    // carry their unboxed repr (the wasm signature materializes them unboxed —
    // see [`function_signature`]); `keep_params` stops [`refine_reprs`] from
    // demoting those (otherwise-def-less) params back to boxed.  A non-typed
    // body keeps the all-boxed param ABI.
    let (specs, keep_params): (&[Repr], HashSet<VarId>) = if typed {
        let keep = func
            .params
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                matches!(
                    func.seed_reprs.get(*i).copied().unwrap_or(Repr::Boxed),
                    Repr::Long | Repr::Double
                )
            })
            .map(|(_, (_, vid))| *vid)
            .collect();
        (&func.seed_reprs, keep)
    } else {
        (&[], HashSet::new())
    };
    let reprs = refine_reprs(func, typeinfer::infer(func, specs), &keep_params);

    // Locals: visible params first (wasm locals `0..nparams`, always boxed
    // `i32`), then the hidden region param (if any), then the remaining VarIds as
    // declared locals typed by their repr, then one `i32` scratch local.
    let mut local_of: HashMap<VarId, u32> = HashMap::new();
    for (i, (_, vid)) in func.params.iter().enumerate() {
        local_of.insert(*vid, i as u32);
    }
    let mut decl_types: Vec<ValType> = Vec::new();
    let mut next_local = abi_nparams as u32;
    for v in 0..func.next_var {
        let vid = VarId(v);
        if let std::collections::hash_map::Entry::Vacant(e) = local_of.entry(vid) {
            e.insert(next_local);
            next_local += 1;
            decl_types.push(local_valtype(reprs.get(&vid)));
        }
    }
    // One extra i32 local, past all VarId locals, holds the scratch-buffer
    // pointer transiently while marshalling an allocation's / call's argument array.
    let scratch_local = next_local;
    decl_types.push(ValType::I32);

    let block_of: HashMap<BlockId, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, i))
        .collect();
    let forward_preds = forward_pred_counts(func);

    // Each declared local is its own single-element run (params come from the
    // function type, not here).
    let mut body = Function::new(decl_types.iter().map(|t| (1, *t)));

    {
        let mut em = Emitter {
            func,
            asm,
            body: &mut body,
            local_of: &local_of,
            block_of: &block_of,
            forward_preds: &forward_preds,
            reprs: &reprs,
            scratch_local,
            region_param_local,
            func_base,
            func_index_of,
            cfg,
            labels: Vec::new(),
            current: func.blocks[0].id,
        };
        // GC safepoint at function entry (mirrors the native backend).
        em.call_import("rt_safepoint")?;
        em.emit_node(structured)?;
    }
    // A guaranteed-valid terminator: every real path already returned, so this
    // is unreachable, but it makes the function's end stack-polymorphic.
    body.instruction(&Instruction::Unreachable);
    body.instruction(&Instruction::End);

    // A typed body's signature carries the hinted params unboxed
    // (`function_signature`); a non-typed body is all-`i32` (boxed).
    let ty_idx = if typed {
        let (params, results) = function_signature(func);
        asm.intern_type(params, results)
    } else {
        asm.intern_type(vec![WasmValType::I32; abi_nparams], vec![WasmValType::I32])
    };
    Ok((body, ty_idx))
}

/// The wasm function signature for `func`: `(params, results)`.
///
/// Honors the hidden trailing region param and the poll-function ABI, mirroring
/// [`IrFunction::abi_param_count`].  The visible params carry their static
/// `^long`/`^double` hint (`seed_reprs`) unboxed; this is the signature of a
/// *typed body* ([`is_typed`]).  A non-typed body uses the all-`i32` boxed
/// signature instead (sized from [`IrFunction::abi_param_count`]).
pub fn function_signature(func: &IrFunction) -> (Vec<WasmValType>, Vec<WasmValType>) {
    if func.takes_state_param() {
        return (
            vec![WasmValType::I32, WasmValType::I32],
            vec![WasmValType::I32],
        );
    }

    let mut params: Vec<WasmValType> = func
        .params
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let repr = func.seed_reprs.get(i).copied().unwrap_or(Repr::Boxed);
            WasmValType::for_repr(repr)
        })
        .collect();

    if func.takes_region_param() {
        params.push(WasmValType::I32);
    }

    (params, vec![WasmValType::I32])
}

/// Whether `func` is compiled with the **typed parameter ABI** — i.e. at least
/// one visible parameter carries a static `^long`/`^double` hint
/// (`seed_reprs`), so its wasm signature takes that argument unboxed
/// (`i64`/`f64`) rather than as a boxed `i32`.
///
/// A typed function compiles to *two* wasm functions: the typed body (the
/// optimized code, whose hinted params are already unboxed) and a boxed-entry
/// **trampoline** ([`emit_trampoline`]) that coerces boxed arguments and calls
/// it.  The trampoline is the function's primary entry — exported, installed in
/// the shared table, and the target of every `CallDirect` — so all the existing
/// boxed dispatch paths reach a typed function unchanged.  Async poll functions
/// (whose `seed_reprs` are cleared during lowering) are never typed.
fn is_typed(func: &IrFunction) -> bool {
    !func.takes_state_param()
        && func
            .seed_reprs
            .iter()
            .any(|r| matches!(r, Repr::Long | Repr::Double))
}

/// Emit the boxed-entry **trampoline** for a typed function (see [`is_typed`]).
///
/// The trampoline has the all-`i32` boxed signature every dispatcher expects
/// (`[i32; abi_param_count] → [i32]`).  It coerces each `^long`/`^double`-hinted
/// argument from its boxed form (`rt_coerce_long`/`rt_coerce_double`), passes
/// the remaining boxed pointers (and the hidden trailing region handle, if any)
/// through unchanged, and calls the typed body at `typed_body_index`.  With the
/// tail-call proposal it `return_call`s the body so no frame is added; otherwise
/// it `call`s and returns the boxed result.  This is the wasm answer to "the
/// typed-parameter ABI needs a boxed-entry trampoline + guard story": the guard
/// is a coercion, there being no in-sandbox deopt seam.
fn emit_trampoline(
    func: &IrFunction,
    asm: &mut ModuleAsm,
    typed_body_index: u32,
    cfg: &WasmBackend,
) -> Result<(Function, u32), WasmError> {
    let nparams = func.params.len();
    let abi_nparams = func.abi_param_count();

    // No declared locals: the trampoline only reads its own params.
    let mut body = Function::new(std::iter::empty());
    for i in 0..nparams {
        body.instruction(&Instruction::LocalGet(i as u32));
        match func.seed_reprs.get(i).copied().unwrap_or(Repr::Boxed) {
            Repr::Long => {
                let idx = asm.import(abi::lookup("rt_coerce_long").expect("rt_coerce_long import"));
                body.instruction(&Instruction::Call(idx));
            }
            Repr::Double => {
                let idx =
                    asm.import(abi::lookup("rt_coerce_double").expect("rt_coerce_double import"));
                body.instruction(&Instruction::Call(idx));
            }
            // Boxed params (and any non-numeric hint) pass through verbatim.
            _ => {}
        }
    }
    // The hidden trailing region handle, if present, is already an `i32` and
    // passes through unchanged.
    if func.takes_region_param() {
        body.instruction(&Instruction::LocalGet(nparams as u32));
    }
    if cfg.tail_calls {
        body.instruction(&Instruction::ReturnCall(typed_body_index));
    } else {
        body.instruction(&Instruction::Call(typed_body_index));
    }
    body.instruction(&Instruction::End);

    let ty_idx = asm.intern_type(vec![WasmValType::I32; abi_nparams], vec![WasmValType::I32]);
    Ok((body, ty_idx))
}

// ── Module assembly state ────────────────────────────────────────────────────

fn valtype(w: &WasmValType) -> ValType {
    match w {
        WasmValType::I32 => ValType::I32,
        WasmValType::I64 => ValType::I64,
        WasmValType::F64 => ValType::F64,
    }
}

/// The wasm local type carrying a value of the given (optional) repr.
/// `Long`→`i64`, `Double`→`f64`, everything else (boxed pointers, `Bool` 0/1
/// payloads, region handles, array handles)→`i32`.
fn local_valtype(repr: Option<&Repr>) -> ValType {
    match repr {
        Some(Repr::Long) => ValType::I64,
        Some(Repr::Double) => ValType::F64,
        _ => ValType::I32,
    }
}

/// Demote every unboxed repr the emitter cannot *produce* unboxed back to
/// `Boxed`, transitively, so the returned map only marks a value unboxed when
/// its defining instruction is one this emitter lowers to native ops with all
/// the operands it needs already unboxed.
///
/// [`typeinfer::infer`] is sound but more ambitious than this backend: it marks
/// e.g. `Long` `*` results unboxed (the native backend has a 128-bit
/// `i64.mul_hi`-style overflow check; wasm lacks the primitive), and it can seed
/// params from specs (the typed-parameter ABI is deferred here).  Keeping a
/// value unboxed while emitting it boxed — or vice versa — would mistype its
/// local, so this pass conservatively boxes anything the emitter does not handle
/// and re-checks dependents until a fixpoint.  Boxing only ever *removes*
/// entries, so it terminates.
fn refine_reprs(
    func: &IrFunction,
    mut reprs: HashMap<VarId, Repr>,
    keep_params: &HashSet<VarId>,
) -> HashMap<VarId, Repr> {
    // Defining instruction of each SSA var (params have none).
    let mut def: HashMap<VarId, &Inst> = HashMap::new();
    for block in &func.blocks {
        for inst in block.phis.iter().chain(block.insts.iter()) {
            if let Some(d) = inst.dst() {
                def.insert(d, inst);
            }
        }
    }

    let repr_of = |reprs: &HashMap<VarId, Repr>, v: VarId| -> Repr {
        reprs.get(&v).copied().unwrap_or(Repr::Boxed)
    };
    let is_unboxed_num = |r: Repr| matches!(r, Repr::Long | Repr::Double);

    loop {
        let mut demote: Vec<VarId> = Vec::new();
        for (&v, &r) in &reprs {
            // A param (no def site) is normally kept boxed — but a typed-ABI
            // param whose unboxed repr is materialized by the wasm function
            // signature (`keep_params`, the `^long`/`^double`-hinted params) is
            // genuinely produced unboxed and must stay so, or its local's wasm
            // type and its repr would disagree.
            let Some(inst) = def.get(&v) else {
                if !keep_params.contains(&v) {
                    demote.push(v);
                }
                continue;
            };
            let ok = match inst {
                // Scalar constants are produced directly.
                Inst::Const(_, Const::Long(_)) => r == Repr::Long,
                Inst::Const(_, Const::Double(_)) => r == Repr::Double,
                Inst::Const(_, Const::Bool(_)) => r == Repr::Bool,
                // A φ is same-repr local copies: every entry must share `r`.
                Inst::Phi(_, entries) => entries.iter().all(|(_, s)| repr_of(&reprs, *s) == r),
                // Binary arithmetic / comparison this emitter lowers unboxed.
                Inst::CallKnown(_, kf, args) if args.len() == 2 => {
                    let a = repr_of(&reprs, args[0]);
                    let b = repr_of(&reprs, args[1]);
                    match (r, kf) {
                        // Checked `+`/`-` (overflow branch) and unchecked
                        // `+`/`-`/`*` (wrapping) on two longs.
                        (
                            Repr::Long,
                            KnownFn::Add
                            | KnownFn::Sub
                            | KnownFn::UncheckedAdd
                            | KnownFn::UncheckedSub
                            | KnownFn::UncheckedMul,
                        ) => a == Repr::Long && b == Repr::Long,
                        // `f64` arithmetic (mixed operands promote).
                        (
                            Repr::Double,
                            KnownFn::Add | KnownFn::Sub | KnownFn::Mul | KnownFn::Div,
                        ) => is_unboxed_num(a) && is_unboxed_num(b),
                        // Ordered comparisons over two unboxed numbers.
                        (Repr::Bool, KnownFn::Lt | KnownFn::Gt | KnownFn::Lte | KnownFn::Gte) => {
                            is_unboxed_num(a) && is_unboxed_num(b)
                        }
                        // `=` only when both are longs (i64 equality).
                        (Repr::Bool, KnownFn::Eq) => a == Repr::Long && b == Repr::Long,
                        _ => false,
                    }
                }
                _ => false,
            };
            if !ok {
                demote.push(v);
            }
        }
        if demote.is_empty() {
            break;
        }
        for v in demote {
            reprs.remove(&v);
        }
    }
    reprs
}

/// Accumulates the wasm module's interned function types and `rt_abi` imports.
struct ModuleAsm {
    /// Interned function types, in section order.
    types: Vec<(Vec<WasmValType>, Vec<WasmValType>)>,
    type_map: HashMap<(Vec<WasmValType>, Vec<WasmValType>), u32>,
    /// `(import name, type index)`, in function-index order.
    imports: Vec<(&'static str, u32)>,
    import_map: HashMap<&'static str, u32>,
    /// Whether the function uses linear memory (e.g. marshalling alloc element
    /// arrays into the scratch buffer).  When set, the module imports `"rt"
    /// "memory"`.  Memory imports occupy a separate index space from function
    /// imports, so this does not affect function indices.
    needs_memory: bool,
    /// Whether any function materializes a closure (`AllocClosure`).  When set,
    /// the module imports the runtime's shared indirect function table and
    /// installs its defined functions into it with an active element segment, so
    /// a closure's `fn_ptr` (a table index) resolves.  Like `needs_memory`, this
    /// occupies a separate index space and does not affect function indices.
    needs_table: bool,
    /// The deduplicated read-only data pool for string/keyword/symbol constant
    /// bytes, emitted as a single active data segment at [`abi::RODATA_BASE`].
    /// A constant at pool offset `o` resolves to the pointer `RODATA_BASE + o`.
    rodata: Vec<u8>,
    /// Maps a constant's bytes to its offset in `rodata`, so identical
    /// constants (and the two emission passes) share one set of bytes.
    rodata_map: HashMap<Vec<u8>, u32>,
}

impl ModuleAsm {
    fn new() -> Self {
        ModuleAsm {
            types: Vec::new(),
            type_map: HashMap::new(),
            imports: Vec::new(),
            import_map: HashMap::new(),
            needs_memory: false,
            needs_table: false,
            rodata: Vec::new(),
            rodata_map: HashMap::new(),
        }
    }

    /// Intern a constant byte string into the read-only data pool, returning its
    /// offset within the pool (relative to [`abi::RODATA_BASE`]).  Identical
    /// byte strings are deduplicated, so repeated string/keyword/symbol
    /// constants — and the two emission passes — share one set of bytes.
    fn intern_rodata(&mut self, bytes: &[u8]) -> u32 {
        if let Some(&off) = self.rodata_map.get(bytes) {
            return off;
        }
        let off = self.rodata.len() as u32;
        self.rodata.extend_from_slice(bytes);
        self.rodata_map.insert(bytes.to_vec(), off);
        off
    }

    fn intern_type(&mut self, params: Vec<WasmValType>, results: Vec<WasmValType>) -> u32 {
        let key = (params, results);
        if let Some(&i) = self.type_map.get(&key) {
            return i;
        }
        let i = self.types.len() as u32;
        self.types.push(key.clone());
        self.type_map.insert(key, i);
        i
    }

    /// Intern `rt`'s type and import it, returning its function index.
    fn import(&mut self, rt: &RtImport) -> u32 {
        if let Some(&idx) = self.import_map.get(rt.name) {
            return idx;
        }
        let ty = self.intern_type(rt.params.to_vec(), rt.results.to_vec());
        let func_idx = self.imports.len() as u32;
        self.imports.push((rt.name, ty));
        self.import_map.insert(rt.name, func_idx);
        func_idx
    }
}

/// Count forward (non-`RecurJump`) predecessors of each block — a block with ≥2
/// is a merge node whose φs are resolved at the `Br` sites that reach it.
fn forward_pred_counts(func: &IrFunction) -> HashMap<BlockId, usize> {
    let mut counts: HashMap<BlockId, usize> = HashMap::new();
    for b in &func.blocks {
        match &b.terminator {
            Terminator::RecurJump { .. } => {}
            Terminator::Jump(t) => *counts.entry(*t).or_default() += 1,
            Terminator::Branch {
                then_block,
                else_block,
                ..
            } => {
                *counts.entry(*then_block).or_default() += 1;
                *counts.entry(*else_block).or_default() += 1;
            }
            Terminator::Return(_) | Terminator::Unreachable => {}
        }
    }
    counts
}

// ── Emitter ──────────────────────────────────────────────────────────────────

struct Emitter<'a> {
    func: &'a IrFunction,
    asm: &'a mut ModuleAsm,
    body: &'a mut Function,
    local_of: &'a HashMap<VarId, u32>,
    block_of: &'a HashMap<BlockId, usize>,
    forward_preds: &'a HashMap<BlockId, usize>,
    /// Per-VarId machine representation; absent ⇒ [`Repr::Boxed`].  Drives the
    /// box/unbox conversions at use sites and the unboxed arithmetic paths.
    reprs: &'a HashMap<VarId, Repr>,
    /// An i32 local (past all VarId locals) that transiently holds the
    /// scratch-buffer pointer while marshalling an allocation's element array.
    scratch_local: u32,
    /// The wasm local holding the hidden trailing region handle, present iff
    /// this is a region-parameterised callee variant.  Bound by
    /// [`Inst::RegionParam`].
    region_param_local: Option<u32>,
    /// Absolute wasm index of the first defined (non-imported) function.  A
    /// direct call to defined function at position `p` is `Call(func_base + p)`.
    func_base: u32,
    /// Maps a callee name to its position among the defined functions, for
    /// [`Inst::CallDirect`]/[`Inst::CallWithRegion`] resolution.
    func_index_of: &'a HashMap<&'a str, u32>,
    /// Backend feature flags (e.g. whether to emit `return_call` for
    /// cross-function tail calls).
    cfg: &'a WasmBackend,
    /// Enclosing control frames; `Some(b)` is a `block`/`loop` labeled `b`,
    /// `None` is an `if`.  Innermost is last.
    labels: Vec<Option<BlockId>>,
    /// The IR block currently being emitted — the predecessor for φ resolution.
    current: BlockId,
}

impl Emitter<'_> {
    fn block(&self, id: BlockId) -> &Block {
        &self.func.blocks[self.block_of[&id]]
    }

    fn local(&self, v: VarId) -> Result<u32, WasmError> {
        self.local_of
            .get(&v)
            .copied()
            .ok_or_else(|| WasmError::Unsupported(format!("reference to unmapped {v}")))
    }

    fn is_merge(&self, id: BlockId) -> bool {
        self.forward_preds.get(&id).copied().unwrap_or(0) >= 2
    }

    fn ins(&mut self, i: &Instruction) {
        self.body.instruction(i);
    }

    fn call_import(&mut self, name: &str) -> Result<(), WasmError> {
        let rt = abi::lookup(name)
            .ok_or_else(|| WasmError::Unsupported(format!("no rt_abi import for {name}")))?;
        let idx = self.asm.import(rt);
        self.ins(&Instruction::Call(idx));
        Ok(())
    }

    fn repr_of(&self, v: VarId) -> Repr {
        self.reprs.get(&v).copied().unwrap_or(Repr::Boxed)
    }

    /// Push `v` as a **boxed** `*const Value` (`i32`).  An unboxed local is boxed
    /// on demand through the matching `rt_*` constructor; a boxed local (or a
    /// region handle) is pushed verbatim.
    fn get(&mut self, v: VarId) -> Result<(), WasmError> {
        let l = self.local(v)?;
        self.ins(&Instruction::LocalGet(l));
        match self.repr_of(v) {
            Repr::Long => self.call_import("rt_const_long")?,
            Repr::Double => self.call_import("rt_const_double")?,
            Repr::Bool => self.call_import("rt_box_bool")?,
            _ => {}
        }
        Ok(())
    }

    /// Push `v`'s local verbatim (no box/unbox).  Used for φ parallel moves,
    /// where source and destination always share a repr.
    fn get_raw(&mut self, v: VarId) -> Result<(), WasmError> {
        let l = self.local(v)?;
        self.ins(&Instruction::LocalGet(l));
        Ok(())
    }

    /// Push `v` as an unboxed `i64`.  A `Long` local is pushed verbatim; a boxed
    /// local is unboxed via `rt_unbox_long`.
    fn get_i64(&mut self, v: VarId) -> Result<(), WasmError> {
        let l = self.local(v)?;
        self.ins(&Instruction::LocalGet(l));
        match self.repr_of(v) {
            Repr::Long => {}
            Repr::Boxed => self.call_import("rt_unbox_long")?,
            other => {
                return Err(WasmError::Unsupported(format!(
                    "cannot read {other:?} value as i64"
                )));
            }
        }
        Ok(())
    }

    /// Push `v` as an unboxed `f64`.  A `Double` local is pushed verbatim; a
    /// `Long` is converted (`f64.convert_i64_s`, mirroring the native backend's
    /// mixed-operand promotion); a boxed local is unboxed via `rt_unbox_double`.
    fn get_f64(&mut self, v: VarId) -> Result<(), WasmError> {
        let l = self.local(v)?;
        self.ins(&Instruction::LocalGet(l));
        match self.repr_of(v) {
            Repr::Double => {}
            Repr::Long => self.ins(&Instruction::F64ConvertI64S),
            Repr::Boxed => self.call_import("rt_unbox_double")?,
            other => {
                return Err(WasmError::Unsupported(format!(
                    "cannot read {other:?} value as f64"
                )));
            }
        }
        Ok(())
    }

    fn set(&mut self, v: VarId) -> Result<(), WasmError> {
        let l = self.local(v)?;
        self.ins(&Instruction::LocalSet(l));
        Ok(())
    }

    // ── Tree walk ────────────────────────────────────────────────────────────

    fn emit_node(&mut self, node: &Structured) -> Result<(), WasmError> {
        match node {
            Structured::Simple { block, next } => {
                // Inlined single-predecessor edge: resolve the target's φs from
                // the predecessor we just came from.  Merge blocks resolve their
                // φs at the `Br` sites instead.
                if !self.is_merge(*block) {
                    self.emit_phi_copies(*block, self.current)?;
                }
                self.current = *block;
                // Cross-function tail call: a block ending in a direct call whose
                // result is the function's return value becomes a `return_call`,
                // reclaiming this frame before the callee runs.  Skips the
                // trailing `local.set` + `local.get`/`return` the normal path
                // would emit, so `next` (the `Return`) is already satisfied.
                if self.cfg.tail_calls
                    && let Structured::Return(rv) = next.as_ref()
                    && self.try_emit_tail_call(*block, *rv)?
                {
                    return Ok(());
                }
                self.emit_block_body(*block)?;
                self.emit_node(next)
            }
            Structured::Labeled { label, body, next } => {
                self.ins(&Instruction::Block(BlockType::Empty));
                self.labels.push(Some(*label));
                self.emit_node(body)?;
                self.labels.pop();
                self.ins(&Instruction::End);
                self.emit_node(next)
            }
            Structured::Loop { header, body } => {
                self.ins(&Instruction::Loop(BlockType::Empty));
                self.labels.push(Some(*header));
                // Back-edge safepoint: the loop label is the continue target, so
                // emitting here runs it on every iteration.
                self.call_import("rt_safepoint")?;
                self.emit_node(body)?;
                self.labels.pop();
                self.ins(&Instruction::End);
                Ok(())
            }
            Structured::If {
                cond,
                then_arm,
                else_arm,
            } => {
                // Branch on the condition's truthiness (left as an i32 0/1).
                self.emit_truthy(*cond)?;
                self.ins(&Instruction::If(BlockType::Empty));
                self.labels.push(None);
                self.emit_node(then_arm)?;
                self.ins(&Instruction::Else);
                self.emit_node(else_arm)?;
                self.labels.pop();
                self.ins(&Instruction::End);
                Ok(())
            }
            Structured::Br(target) => self.emit_br(*target),
            Structured::Return(v) => {
                self.get(*v)?;
                self.ins(&Instruction::Return);
                Ok(())
            }
            Structured::Unreachable => {
                self.ins(&Instruction::Unreachable);
                Ok(())
            }
            Structured::Nil => Ok(()),
        }
    }

    fn emit_br(&mut self, target: BlockId) -> Result<(), WasmError> {
        self.emit_phi_copies(target, self.current)?;
        let depth = self.br_depth(target)?;
        self.ins(&Instruction::Br(depth));
        Ok(())
    }

    /// `br` depth to the enclosing frame labeled `target` (0 = innermost).
    fn br_depth(&self, target: BlockId) -> Result<u32, WasmError> {
        for (k, frame) in self.labels.iter().rev().enumerate() {
            if *frame == Some(target) {
                return Ok(k as u32);
            }
        }
        Err(WasmError::Unsupported(format!(
            "br target {target} not in an enclosing block/loop"
        )))
    }

    /// Copy each φ of `target` from its `pred` entry into the φ's local, using
    /// the operand stack for parallel-move semantics.
    fn emit_phi_copies(&mut self, target: BlockId, pred: BlockId) -> Result<(), WasmError> {
        let mut moves: Vec<(VarId, VarId)> = Vec::new(); // (dst, src)
        for inst in &self.block(target).phis {
            if let Inst::Phi(dst, entries) = inst
                && let Some((_, src)) = entries.iter().find(|(p, _)| *p == pred)
            {
                moves.push((*dst, *src));
            }
        }
        if moves.is_empty() {
            return Ok(());
        }
        // Each source is read as the destination's repr.  When `dst` is unboxed,
        // `meet` guarantees every entry shares that repr, so a raw copy is
        // type-correct; when `dst` is boxed, an unboxed source must be boxed
        // first (a φ may join an unboxed value with a boxed one).  Reads precede
        // all writes, preserving parallel-move semantics.
        for (dst, src) in &moves {
            if self.repr_of(*dst) == Repr::Boxed {
                self.get(*src)?;
            } else {
                self.get_raw(*src)?;
            }
        }
        for (dst, _) in moves.iter().rev() {
            self.set(*dst)?;
        }
        Ok(())
    }

    /// Push an `i32` truthiness flag (`0`/non-zero) for `v`, consumed directly by
    /// a wasm `if`.  An unboxed `Bool` is already `0`/`1`; an unboxed number is
    /// constant-`true` (every number is truthy, matching `rt_truthiness`); a
    /// boxed value goes through the `rt_truthiness` bridge.
    fn emit_truthy(&mut self, v: VarId) -> Result<(), WasmError> {
        match self.repr_of(v) {
            Repr::Bool => self.get_raw(v),
            Repr::Long | Repr::Double => {
                self.ins(&Instruction::I32Const(1));
                Ok(())
            }
            _ => {
                self.get(v)?;
                self.call_import("rt_truthiness")
            }
        }
    }

    // ── Instruction lowering ─────────────────────────────────────────────────

    fn emit_block_body(&mut self, block: BlockId) -> Result<(), WasmError> {
        // Phis are resolved on edges, not here.
        let idx = self.block_of[&block];
        let n = self.func.blocks[idx].insts.len();
        for i in 0..n {
            let inst = self.func.blocks[idx].insts[i].clone();
            self.emit_inst(&inst)?;
        }
        Ok(())
    }

    /// If `block` ends in a direct call (`CallDirect`/`CallWithRegion`) whose
    /// result is exactly `rv`, emit the block's straight-line body followed by a
    /// `return_call` to the resolved callee and report `true` (the caller then
    /// skips the `Return` node, since the tail call already returned).  Otherwise
    /// emit nothing and report `false`, so the caller falls back to the ordinary
    /// `call` + `return`.
    ///
    /// Only direct calls qualify: `return_call` gives a true cross-function tail
    /// call there, with the callee's signature (`[i32; abi_param_count] →
    /// [i32]`) matching this function's result.  Dynamic `Call`s (dispatched
    /// through the `rt_call` import) keep the ordinary call + return.
    fn try_emit_tail_call(&mut self, block: BlockId, rv: VarId) -> Result<bool, WasmError> {
        let idx = self.block_of[&block];
        let insts = &self.func.blocks[idx].insts;
        let Some(last) = insts.last() else {
            return Ok(false);
        };
        let (name, args, region, dst) = match last {
            Inst::CallDirect(dst, name, args) => (name.clone(), args.clone(), None, *dst),
            Inst::CallWithRegion(dst, name, args, r) => {
                (name.clone(), args.clone(), Some(*r), *dst)
            }
            _ => return Ok(false),
        };
        if dst != rv {
            return Ok(false);
        }
        let callee = self.resolve_func(&name)?;

        // Everything before the trailing call is ordinary straight-line code.
        let n = insts.len();
        for i in 0..n - 1 {
            let inst = self.func.blocks[idx].insts[i].clone();
            self.emit_inst(&inst)?;
        }

        // Tail call: arguments, then the region handle for a region-parameterised
        // callee, then `return_call` (mirrors `emit_direct_call`'s argument order).
        for a in &args {
            self.get(*a)?;
        }
        if let Some(r) = region {
            self.get(r)?;
        }
        self.ins(&Instruction::ReturnCall(callee));
        Ok(true)
    }

    fn emit_inst(&mut self, inst: &Inst) -> Result<(), WasmError> {
        match inst {
            Inst::Const(dst, c) => {
                // An unboxed scalar materializes directly into its typed local;
                // anything else is boxed through an `rt_const_*` bridge.
                match (self.repr_of(*dst), c) {
                    (Repr::Long, Const::Long(n)) => self.ins(&Instruction::I64Const(*n)),
                    (Repr::Double, Const::Double(f)) => {
                        self.ins(&Instruction::F64Const(Ieee64::from(*f)))
                    }
                    (Repr::Bool, Const::Bool(b)) => self.ins(&Instruction::I32Const(i32::from(*b))),
                    _ => self.emit_const(c)?,
                }
                self.set(*dst)
            }
            // In compiled code locals are bound by params / let bindings; an
            // unresolved LoadLocal is nil, matching the Cranelift backend.
            Inst::LoadLocal(dst, _name) => {
                self.call_import("rt_const_nil")?;
                self.set(*dst)
            }
            // An unboxed result is produced in registers (native `i64`/`f64`
            // ops, leaving its raw value); otherwise the boxed `rt_*` bridge.
            Inst::CallKnown(dst, kf, args) => {
                if self.repr_of(*dst) != Repr::Boxed && args.len() == 2 {
                    self.emit_known_unboxed(*dst, kf, args[0], args[1])
                } else {
                    self.emit_known(kf, args)?;
                    self.set(*dst)
                }
            }
            // Direct call to another function in this bundle, resolved by name to
            // its wasm function index (mirrors `codegen.rs::emit_direct_call`).
            Inst::CallDirect(dst, name, args) => {
                self.emit_direct_call(name, args, None)?;
                self.set(*dst)
            }
            // Direct call to a region-parameterised variant, threading the
            // caller's region handle as the hidden trailing argument (mirrors
            // `codegen.rs::emit_direct_call_with_extra`).
            Inst::CallWithRegion(dst, name, args, region) => {
                self.emit_direct_call(name, args, Some(*region))?;
                self.set(*dst)
            }
            // Dynamic dispatch through a boxed callable value (mirrors
            // `codegen.rs::emit_unknown_call`, minus the inline cache).
            Inst::Call(dst, callee, args) => {
                self.emit_dynamic_call(*callee, args)?;
                self.set(*dst)
            }
            Inst::AllocVector(dst, elems) => {
                self.emit_alloc("rt_alloc_vector", elems, elems.len() as u64)?;
                self.set(*dst)
            }
            Inst::AllocSet(dst, elems) => {
                self.emit_alloc("rt_alloc_set", elems, elems.len() as u64)?;
                self.set(*dst)
            }
            Inst::AllocList(dst, elems) => {
                self.emit_alloc("rt_alloc_list", elems, elems.len() as u64)?;
                self.set(*dst)
            }
            Inst::AllocMap(dst, pairs) => {
                // Flatten to [k0, v0, k1, v1, ...]; the count is the pair count,
                // matching `rt_alloc_map`'s contract (mirrors the native backend).
                let flat: Vec<VarId> = pairs.iter().flat_map(|(k, v)| [*k, *v]).collect();
                self.emit_alloc("rt_alloc_map", &flat, pairs.len() as u64)?;
                self.set(*dst)
            }
            Inst::AllocCons(dst, head, tail) => {
                // Two pointer args, no element array.
                self.get(*head)?;
                self.get(*tail)?;
                self.call_import("rt_alloc_cons")?;
                self.set(*dst)
            }
            Inst::AllocClosure(dst, template, captures) => {
                self.emit_alloc_closure(template, captures)?;
                self.set(*dst)
            }
            // ── Globals / vars ─────────────────────────────────────────────────
            // Resolve a namespaced binding to its value (mirrors
            // `codegen.rs::emit_load_global`).  Versioned `name@sha` names are
            // handled inside `rt_load_global` (uncached — the per-call-site
            // versioned IC is deferred with `rt_call_ic`), so the wasm path makes
            // no version distinction.
            Inst::LoadGlobal(dst, ns, name) => {
                self.push_name_args(ns, name);
                self.call_import("rt_load_global")?;
                self.set(*dst)
            }
            // Resolve the Var object itself (for `set!`/`binding`), mirroring
            // `codegen.rs::emit_load_var`.
            Inst::LoadVar(dst, ns, name) => {
                self.push_name_args(ns, name);
                self.call_import("rt_load_var")?;
                self.set(*dst)
            }
            // `(def ns/name val)` — intern the var with its value, mirroring
            // `codegen.rs::emit_def_var`.
            Inst::DefVar(dst, ns, name, val) => {
                self.push_name_args(ns, name);
                self.get(*val)?;
                self.call_import("rt_def_var")?;
                self.set(*dst)
            }
            // `(set! var val)` — mutate a Var's binding.  `rt_set_bang` returns a
            // `*const Value` the IR has no destination for, so it is dropped.
            Inst::SetBang(var, val) => {
                self.get(*var)?;
                self.get(*val)?;
                self.call_import("rt_set_bang")?;
                self.ins(&Instruction::Drop);
                Ok(())
            }
            // ── Exceptions (thread-local error path) ───────────────────────────
            // `(throw exc)` — stash the exception in the runtime's thread-local
            // and continue to the block's terminator (mirrors `codegen.rs`'s
            // `Inst::Throw`; `rt_throw` returns nil, which is dropped).  The
            // enclosing `rt_try` checks the thread-local after the body runs.
            Inst::Throw(val) => {
                self.get(*val)?;
                self.call_import("rt_throw")?;
                self.ins(&Instruction::Drop);
                Ok(())
            }
            // ── Region operations ────────────────────────────────────────────
            Inst::RegionStart(dst) => {
                // Allocate and activate a bump region; keep its i32 handle.
                self.call_import(abi::RT_REGION_START)?;
                self.set(*dst)
            }
            Inst::RegionAlloc(dst, region, kind, operands) => {
                self.emit_region_alloc(*region, *kind, operands)?;
                self.set(*dst)
            }
            Inst::RegionEnd(region) => {
                // Pop and free the bump region; drop the bridge's i32 result.
                self.get(*region)?;
                self.call_import(abi::RT_REGION_END)?;
                self.ins(&Instruction::Drop);
                Ok(())
            }
            Inst::RegionParam(dst) => {
                // Bind the caller's region, received as the hidden trailing
                // function parameter (mirrors `codegen.rs`'s `region_param`).
                let l = self.region_param_local.ok_or_else(|| {
                    WasmError::Unsupported(
                        "RegionParam in a function compiled without a region parameter".into(),
                    )
                })?;
                self.ins(&Instruction::LocalGet(l));
                self.set(*dst)
            }
            // Resolved on edges / by terminators.
            Inst::Phi(..) | Inst::Recur(..) | Inst::SourceLoc(..) => Ok(()),
            other => Err(WasmError::Unsupported(format!(
                "instruction not yet lowered: {other}"
            ))),
        }
    }

    /// Materialize a constant as a boxed value, left on the operand stack.
    fn emit_const(&mut self, c: &Const) -> Result<(), WasmError> {
        match c {
            Const::Nil => self.call_import("rt_const_nil"),
            Const::Bool(true) => self.call_import("rt_const_true"),
            Const::Bool(false) => self.call_import("rt_const_false"),
            Const::Long(n) => {
                self.ins(&Instruction::I64Const(*n));
                self.call_import("rt_const_long")
            }
            Const::Double(f) => {
                self.ins(&Instruction::F64Const(Ieee64::from(*f)));
                self.call_import("rt_const_double")
            }
            Const::Char(ch) => {
                self.ins(&Instruction::I32Const(*ch as i32));
                self.call_import("rt_const_char")
            }
            // Strings/keywords/symbols: their UTF-8 bytes live in the read-only
            // data pool (one active data segment at `RODATA_BASE`); the constant
            // resolves to the `(ptr, len)` pair passed to the `rt_const_*`
            // bridge.  Mirrors `codegen.rs::emit_string_const` (the native
            // backend defines an anonymous data object per string); keywords go
            // through `rt_const_keyword` rather than the per-call-site inline
            // cache, which is deferred with the rest of the IC work.
            Const::Str(s) => self.emit_string_like(s, "rt_const_string"),
            Const::Keyword(s) => self.emit_string_like(s, "rt_const_keyword"),
            Const::Symbol(s) => self.emit_string_like(s, "rt_const_symbol"),
        }
    }

    /// Materialize a string/keyword/symbol constant: intern its UTF-8 bytes into
    /// the rodata pool, push `(ptr, len)` (`ptr = RODATA_BASE + offset`), and
    /// call the boxing `rt_const_*` bridge.  The boxed result is left on the
    /// operand stack.
    fn emit_string_like(&mut self, s: &str, bridge: &str) -> Result<(), WasmError> {
        // The pool lives in the runtime's imported linear memory.
        self.asm.needs_memory = true;
        let off = self.asm.intern_rodata(s.as_bytes());
        self.ins(&Instruction::I32Const((abi::RODATA_BASE + off) as i32));
        self.ins(&Instruction::I64Const(s.len() as i64));
        self.call_import(bridge)
    }

    /// Push `(ns_ptr, ns_len, name_ptr, name_len)` for a namespaced reference,
    /// interning both strings into the rodata pool.  Used by the global/var
    /// bridges (mirrors `codegen.rs`'s per-name anonymous data objects).
    fn push_name_args(&mut self, ns: &str, name: &str) {
        self.asm.needs_memory = true;
        let ns_off = self.asm.intern_rodata(ns.as_bytes());
        self.ins(&Instruction::I32Const((abi::RODATA_BASE + ns_off) as i32));
        self.ins(&Instruction::I64Const(ns.len() as i64));
        let name_off = self.asm.intern_rodata(name.as_bytes());
        self.ins(&Instruction::I32Const((abi::RODATA_BASE + name_off) as i32));
        self.ins(&Instruction::I64Const(name.len() as i64));
    }

    /// Lower a known-function call to its boxed `rt_abi` bridge, result left on
    /// the operand stack.
    fn emit_known(&mut self, kf: &KnownFn, args: &[VarId]) -> Result<(), WasmError> {
        // try/catch/finally is a fixed three-arg bridge, not an arithmetic fold
        // or binary comparison: `rt_try(body, catch, finally)` invokes the body
        // thunk, routes a pending thread-local exception into the catch thunk,
        // and always runs the finally thunk (mirrors `codegen.rs`, where
        // `KnownFn::TryCatchFinally` maps to `rt_try`).
        if let KnownFn::TryCatchFinally = kf {
            if args.len() != 3 {
                return Err(WasmError::Unsupported(format!(
                    "try/catch/finally expects 3 thunks, got {}",
                    args.len()
                )));
            }
            for a in args {
                self.get(*a)?;
            }
            return self.call_import("rt_try");
        }

        let (bridge, is_cmp) = match kf {
            KnownFn::Add => ("rt_add", false),
            KnownFn::Sub => ("rt_sub", false),
            KnownFn::Mul => ("rt_mul", false),
            KnownFn::Div => ("rt_div", false),
            KnownFn::Rem => ("rt_rem", false),
            KnownFn::Eq => ("rt_eq", true),
            KnownFn::Lt => ("rt_lt", true),
            KnownFn::Gt => ("rt_gt", true),
            KnownFn::Lte => ("rt_lte", true),
            KnownFn::Gte => ("rt_gte", true),
            other => {
                return Err(WasmError::Unsupported(format!(
                    "known function not yet lowered: {other:?}"
                )));
            }
        };

        if is_cmp {
            if args.len() != 2 {
                return Err(WasmError::Unsupported(format!(
                    "n-ary comparison {kf:?} (only binary lowered)"
                )));
            }
            self.get(args[0])?;
            self.get(args[1])?;
            self.call_import(bridge)?;
        } else {
            // Left-fold the boxed binary bridge: (op a b c) = op(op(a,b),c).
            let Some((first, rest)) = args.split_first() else {
                return Err(WasmError::Unsupported(format!(
                    "0-ary arithmetic {kf:?} (no identity element lowered)"
                )));
            };
            self.get(*first)?;
            for a in rest {
                self.get(*a)?;
                self.call_import(bridge)?;
            }
        }
        Ok(())
    }

    /// Lower a binary known call whose result [`refine_reprs`] kept unboxed,
    /// emitting native `i64`/`f64` ops and storing the raw result into `dst`'s
    /// typed local (so, unlike [`Self::emit_known`], this sets `dst` itself).
    /// Mirrors `codegen.rs`'s unboxed known-call path + `emit_long_overflow_check`.
    fn emit_known_unboxed(
        &mut self,
        dst: VarId,
        kf: &KnownFn,
        a: VarId,
        b: VarId,
    ) -> Result<(), WasmError> {
        match (self.repr_of(dst), kf) {
            // ── Checked long `+`/`-`: store the wrapped result, then branch to a
            // throw on signed overflow (Clojure primitive-long semantics). ──────
            (Repr::Long, KnownFn::Add | KnownFn::Sub) => {
                self.get_i64(a)?;
                self.get_i64(b)?;
                self.ins(if matches!(kf, KnownFn::Add) {
                    &Instruction::I64Add
                } else {
                    &Instruction::I64Sub
                });
                self.set(dst)?; // dst = a ± b (wrapped)
                self.emit_long_overflow_check(kf, a, b, dst)
            }
            // ── Unchecked long `+`/`-`/`*`: plain wrapping ops, no overflow check.
            (Repr::Long, KnownFn::UncheckedAdd | KnownFn::UncheckedSub | KnownFn::UncheckedMul) => {
                self.get_i64(a)?;
                self.get_i64(b)?;
                self.ins(match kf {
                    KnownFn::UncheckedAdd => &Instruction::I64Add,
                    KnownFn::UncheckedSub => &Instruction::I64Sub,
                    _ => &Instruction::I64Mul,
                });
                self.set(dst)
            }
            // ── f64 arithmetic (mixed long/double operands promote to f64). ─────
            (Repr::Double, KnownFn::Add | KnownFn::Sub | KnownFn::Mul | KnownFn::Div) => {
                self.get_f64(a)?;
                self.get_f64(b)?;
                self.ins(match kf {
                    KnownFn::Add => &Instruction::F64Add,
                    KnownFn::Sub => &Instruction::F64Sub,
                    KnownFn::Mul => &Instruction::F64Mul,
                    _ => &Instruction::F64Div,
                });
                self.set(dst)
            }
            // ── Ordered comparison → an i32 0/1 boolean. ───────────────────────
            (Repr::Bool, KnownFn::Lt | KnownFn::Gt | KnownFn::Lte | KnownFn::Gte) => {
                let both_long = self.repr_of(a) == Repr::Long && self.repr_of(b) == Repr::Long;
                if both_long {
                    self.get_i64(a)?;
                    self.get_i64(b)?;
                    self.ins(match kf {
                        KnownFn::Lt => &Instruction::I64LtS,
                        KnownFn::Gt => &Instruction::I64GtS,
                        KnownFn::Lte => &Instruction::I64LeS,
                        _ => &Instruction::I64GeS,
                    });
                } else {
                    self.get_f64(a)?;
                    self.get_f64(b)?;
                    self.ins(match kf {
                        KnownFn::Lt => &Instruction::F64Lt,
                        KnownFn::Gt => &Instruction::F64Gt,
                        KnownFn::Lte => &Instruction::F64Le,
                        _ => &Instruction::F64Ge,
                    });
                }
                self.set(dst)
            }
            // ── `=` on two longs → i64 equality. ───────────────────────────────
            (Repr::Bool, KnownFn::Eq) => {
                self.get_i64(a)?;
                self.get_i64(b)?;
                self.ins(&Instruction::I64Eq);
                self.set(dst)
            }
            (repr, other) => Err(WasmError::Unsupported(format!(
                "unboxed known call {other:?} → {repr:?} not lowered"
            ))),
        }
    }

    /// After a wrapped `i64` `+`/`-` whose result is already stored in `sum`'s
    /// local, branch to a throw block when the signed operation overflowed,
    /// raising the integer-overflow exception and returning boxed `nil` (mirrors
    /// `codegen.rs::emit_long_overflow_check`).  Signed-overflow predicates:
    /// add `((a ^ s) & (b ^ s)) < 0`, sub `((a ^ b) & (a ^ s)) < 0`.
    fn emit_long_overflow_check(
        &mut self,
        kf: &KnownFn,
        a: VarId,
        b: VarId,
        sum: VarId,
    ) -> Result<(), WasmError> {
        // Compute the overflow predicate as an i32 boolean.
        if matches!(kf, KnownFn::Add) {
            self.get_i64(a)?; // a ^ s
            self.get_raw(sum)?;
            self.ins(&Instruction::I64Xor);
            self.get_i64(b)?; // b ^ s
            self.get_raw(sum)?;
            self.ins(&Instruction::I64Xor);
        } else {
            self.get_i64(a)?; // a ^ b
            self.get_i64(b)?;
            self.ins(&Instruction::I64Xor);
            self.get_i64(a)?; // a ^ s
            self.get_raw(sum)?;
            self.ins(&Instruction::I64Xor);
        }
        self.ins(&Instruction::I64And);
        self.ins(&Instruction::I64Const(0));
        self.ins(&Instruction::I64LtS);
        self.ins(&Instruction::If(BlockType::Empty));
        // Overflow: raise the exception (stashed in the thread-local) and return
        // its boxed-nil result, matching the native backend's throw block.
        self.call_import("rt_overflow_error")?;
        self.call_import("rt_throw")?;
        self.ins(&Instruction::Return);
        self.ins(&Instruction::End);
        Ok(())
    }

    /// Resolve a callee name to its absolute wasm function index, or report a
    /// clean [`WasmError::Unsupported`] (mirrors `codegen.rs`'s lookup error).
    fn resolve_func(&self, name: &str) -> Result<u32, WasmError> {
        self.func_index_of
            .get(name)
            .map(|pos| self.func_base + pos)
            .ok_or_else(|| {
                WasmError::Unsupported(format!("direct call to function not in bundle: {name}"))
            })
    }

    /// Resolve an arity function name to its **table slot** — the wasm32 function
    /// pointer the runtime calls through.  This is `FUNC_TABLE_BASE` plus the
    /// callee's bundle position, not its wasm function index (see
    /// [`emit_bundle`]'s element segment).
    fn resolve_table_index(&self, name: &str) -> Result<u32, WasmError> {
        self.func_index_of
            .get(name)
            .map(|pos| abi::FUNC_TABLE_BASE + pos)
            .ok_or_else(|| {
                WasmError::Unsupported(format!("closure arity function not in bundle: {name}"))
            })
    }

    /// Lower a direct call to a bundled function: push the argument locals, then
    /// (for a region-parameterised callee) the region handle as the hidden
    /// trailing argument, then `call` the resolved index.  The boxed result is
    /// left on the operand stack.
    fn emit_direct_call(
        &mut self,
        name: &str,
        args: &[VarId],
        region: Option<VarId>,
    ) -> Result<(), WasmError> {
        let idx = self.resolve_func(name)?;
        for a in args {
            self.get(*a)?;
        }
        if let Some(region) = region {
            self.get(region)?;
        }
        self.ins(&Instruction::Call(idx));
        Ok(())
    }

    /// Lower a dynamic call through `rt_call(callee, args_ptr, nargs)`: marshal
    /// the argument `*const Value` pointers into the scratch buffer, then call the
    /// bridge.  A zero-arg call passes a null pointer and a zero count.  The boxed
    /// result is left on the operand stack.
    ///
    /// This is the inline-cache-free path; `rt_call_ic` additionally needs a
    /// writable per-call-site IC slot in linear memory (a data-segment follow-up).
    fn emit_dynamic_call(&mut self, callee: VarId, args: &[VarId]) -> Result<(), WasmError> {
        let n = args.len();
        if n == 0 {
            self.get(callee)?;
            self.ins(&Instruction::I32Const(0)); // null args pointer
            self.ins(&Instruction::I64Const(0)); // zero count
            return self.call_import("rt_call");
        }

        self.asm.needs_memory = true;

        // scratch = rt_scratch_ptr(n * 4) — argument pointers are wasm i32s.
        self.ins(&Instruction::I32Const((n * 4) as i32));
        self.call_import("rt_scratch_ptr")?;
        self.ins(&Instruction::LocalSet(self.scratch_local));

        // Store each argument pointer at scratch + i*4.
        for (i, a) in args.iter().enumerate() {
            self.ins(&Instruction::LocalGet(self.scratch_local));
            self.get(*a)?;
            self.ins(&Instruction::I32Store(MemArg {
                offset: (i * 4) as u64,
                align: 2, // 2^2 == 4-byte alignment
                memory_index: 0,
            }));
        }

        // rt_call(callee, scratch, n)
        self.get(callee)?;
        self.ins(&Instruction::LocalGet(self.scratch_local));
        self.ins(&Instruction::I64Const(n as i64));
        self.call_import("rt_call")
    }

    /// Lower a slice-taking allocation bridge (`rt_alloc_vector` etc.): marshal
    /// the element `*const Value` pointers into the scratch buffer, then call
    /// `bridge(ptr, count)`.  The boxed result is left on the operand stack.
    ///
    /// `elems` are the pointers stored contiguously (for maps, the flattened
    /// key/value sequence); `count` is the value passed to the bridge (element
    /// count, or pair count for maps).  Mirrors the native
    /// `codegen.rs::emit_alloc_collection`.
    fn emit_alloc(&mut self, bridge: &str, elems: &[VarId], count: u64) -> Result<(), WasmError> {
        let n = elems.len();
        if n == 0 {
            // Empty literal: pass a null array pointer and a zero count.
            self.ins(&Instruction::I32Const(0));
            self.ins(&Instruction::I64Const(count as i64));
            return self.call_import(bridge);
        }

        self.asm.needs_memory = true;

        // scratch = rt_scratch_ptr(n * 4)  — element pointers are wasm i32s.
        self.ins(&Instruction::I32Const((n * 4) as i32));
        self.call_import("rt_scratch_ptr")?;
        self.ins(&Instruction::LocalSet(self.scratch_local));

        // Store each element pointer at scratch + i*4.
        for (i, e) in elems.iter().enumerate() {
            self.ins(&Instruction::LocalGet(self.scratch_local));
            self.get(*e)?;
            self.ins(&Instruction::I32Store(MemArg {
                offset: (i * 4) as u64,
                align: 2, // 2^2 == 4-byte alignment
                memory_index: 0,
            }));
        }

        // bridge(scratch, count)
        self.ins(&Instruction::LocalGet(self.scratch_local));
        self.ins(&Instruction::I64Const(count as i64));
        self.call_import(bridge)
    }

    /// Lower a region-aware allocation (`rt_region_alloc_*`): like
    /// [`Self::emit_alloc`] but threads the region `handle` as the leading `i32`
    /// argument — `bridge(handle, ptr, count)` — bump-allocating into the
    /// caller's arena instead of the GC heap.  Mirrors the native
    /// `codegen.rs::emit_region_alloc_collection`.  The boxed result is left on
    /// the operand stack.
    fn emit_region_alloc(
        &mut self,
        region: VarId,
        kind: RegionAllocKind,
        operands: &[VarId],
    ) -> Result<(), WasmError> {
        let (bridge, count) = match kind {
            RegionAllocKind::Vector => ("rt_region_alloc_vector", operands.len() as u64),
            RegionAllocKind::Set => ("rt_region_alloc_set", operands.len() as u64),
            RegionAllocKind::List => ("rt_region_alloc_list", operands.len() as u64),
            // `operands` is the flattened [k0, v0, …]; the bridge wants the pair
            // count (mirrors the native backend's `n / 2`).
            RegionAllocKind::Map => ("rt_region_alloc_map", (operands.len() / 2) as u64),
            RegionAllocKind::Cons => {
                // Two pointer args directly: rt_region_alloc_cons(handle, h, t).
                // A degenerate cons falls back to nil, as in the native backend.
                if operands.len() != 2 {
                    return self.call_import("rt_const_nil");
                }
                self.get(region)?;
                self.get(operands[0])?;
                self.get(operands[1])?;
                return self.call_import("rt_region_alloc_cons");
            }
        };

        let n = operands.len();
        if n == 0 {
            // Empty literal: handle, null array pointer, zero count.
            self.get(region)?;
            self.ins(&Instruction::I32Const(0));
            self.ins(&Instruction::I64Const(count as i64));
            return self.call_import(bridge);
        }

        self.asm.needs_memory = true;

        // scratch = rt_scratch_ptr(n * 4) — element pointers are wasm i32s.
        self.ins(&Instruction::I32Const((n * 4) as i32));
        self.call_import("rt_scratch_ptr")?;
        self.ins(&Instruction::LocalSet(self.scratch_local));

        // Store each element pointer at scratch + i*4.
        for (i, e) in operands.iter().enumerate() {
            self.ins(&Instruction::LocalGet(self.scratch_local));
            self.get(*e)?;
            self.ins(&Instruction::I32Store(MemArg {
                offset: (i * 4) as u64,
                align: 2, // 2^2 == 4-byte alignment
                memory_index: 0,
            }));
        }

        // bridge(handle, scratch, count)
        self.get(region)?;
        self.ins(&Instruction::LocalGet(self.scratch_local));
        self.ins(&Instruction::I64Const(count as i64));
        self.call_import(bridge)
    }

    /// Materialize a closure value via `rt_make_fn` / `rt_make_fn_variadic` /
    /// `rt_make_fn_multi` (mirrors `codegen.rs`'s `AllocClosure`).  The arity
    /// function pointer(s) are wasm32 table indices resolved by
    /// [`Self::resolve_table_index`]; the closure name bytes, the captured-value
    /// pointer array, and (for multi-arity) the fn-pointer / param-count /
    /// variadic-flag arrays are all marshalled contiguously through one scratch
    /// reservation, since `rt_scratch_ptr` returns a single shared buffer.  The
    /// boxed closure is left on the operand stack.
    fn emit_alloc_closure(
        &mut self,
        tmpl: &ClosureTemplate,
        captures: &[VarId],
    ) -> Result<(), WasmError> {
        // No compiled arities — no closure value to build; fall back to nil
        // (mirrors the native backend).
        if tmpl.arity_fn_names.is_empty() {
            return self.call_import("rt_const_nil");
        }

        self.asm.needs_table = true;

        let name_str: &str = tmpl.name.as_deref().unwrap_or(&tmpl.arity_fn_names[0]);
        let name_bytes = name_str.as_bytes();
        let name_len = name_bytes.len();
        let ncaptures = captures.len();
        let n_arities = tmpl.arity_fn_names.len();

        // Resolve each arity's table slot up front (clean error if a referenced
        // arity isn't in the bundle).
        let slots: Vec<u32> = tmpl
            .arity_fn_names
            .iter()
            .map(|f| self.resolve_table_index(f))
            .collect::<Result<_, _>>()?;

        if n_arities == 1 {
            // Single arity.  Scratch layout: [name bytes][captures: i32 × k'].
            let off_name = 0usize;
            let off_caps = align_up(name_len, 4);
            let total = off_caps + ncaptures * 4;
            if total > 0 {
                self.scratch_reserve(total)?;
            }
            self.store_const_bytes(off_name, name_bytes);
            self.store_ptr_array(off_caps, captures)?;

            // rt_make_fn(name_ptr, name_len, fn_ptr, param_count, captures, ncaptures)
            self.push_scratch_or_null(off_name, name_len);
            self.ins(&Instruction::I64Const(name_len as i64));
            self.ins(&Instruction::I32Const(slots[0] as i32));
            self.ins(&Instruction::I64Const(tmpl.param_counts[0] as i64));
            self.push_scratch_or_null(off_caps, ncaptures);
            self.ins(&Instruction::I64Const(ncaptures as i64));
            let bridge = if tmpl.is_variadic[0] {
                "rt_make_fn_variadic"
            } else {
                "rt_make_fn"
            };
            return self.call_import(bridge);
        }

        // Multi-arity.  Scratch layout:
        //   [name][fn_ptrs: i32 × k][param_counts: i64 × k][is_variadic: u8 × k][captures: i32 × k']
        let off_name = 0usize;
        let off_fnptrs = align_up(off_name + name_len, 4);
        let off_pc = align_up(off_fnptrs + n_arities * 4, 8);
        let off_var = off_pc + n_arities * 8;
        let off_caps = align_up(off_var + n_arities, 4);
        let total = off_caps + ncaptures * 4;
        self.scratch_reserve(total)?;

        self.store_const_bytes(off_name, name_bytes);
        for (i, &slot) in slots.iter().enumerate() {
            self.store_const_i32(off_fnptrs + i * 4, slot as i32);
        }
        for (i, &pc) in tmpl.param_counts.iter().enumerate() {
            self.store_const_i64(off_pc + i * 8, pc as i64);
        }
        for (i, &v) in tmpl.is_variadic.iter().enumerate() {
            self.store_const_i8(off_var + i, v as i32);
        }
        self.store_ptr_array(off_caps, captures)?;

        // rt_make_fn_multi(name_ptr, name_len, fn_ptrs, param_counts,
        //                  is_variadic, n_arities, captures, ncaptures)
        self.push_scratch_or_null(off_name, name_len);
        self.ins(&Instruction::I64Const(name_len as i64));
        self.push_scratch_addr(off_fnptrs);
        self.push_scratch_addr(off_pc);
        self.push_scratch_addr(off_var);
        self.ins(&Instruction::I64Const(n_arities as i64));
        self.push_scratch_or_null(off_caps, ncaptures);
        self.ins(&Instruction::I64Const(ncaptures as i64));
        self.call_import("rt_make_fn_multi")
    }

    // ── Scratch-buffer marshalling helpers ───────────────────────────────────

    /// Reserve `total` bytes of scratch (one `rt_scratch_ptr` call), leaving the
    /// buffer base in `scratch_local`.
    fn scratch_reserve(&mut self, total: usize) -> Result<(), WasmError> {
        self.asm.needs_memory = true;
        self.ins(&Instruction::I32Const(total as i32));
        self.call_import("rt_scratch_ptr")?;
        self.ins(&Instruction::LocalSet(self.scratch_local));
        Ok(())
    }

    /// Push the `i32` address `scratch + offset` onto the operand stack.
    fn push_scratch_addr(&mut self, offset: usize) {
        self.ins(&Instruction::LocalGet(self.scratch_local));
        if offset != 0 {
            self.ins(&Instruction::I32Const(offset as i32));
            self.ins(&Instruction::I32Add);
        }
    }

    /// Push `scratch + offset` if `count > 0`, else a null (`0`) pointer — for
    /// the optional name / captures arrays.
    fn push_scratch_or_null(&mut self, offset: usize, count: usize) {
        if count > 0 {
            self.push_scratch_addr(offset);
        } else {
            self.ins(&Instruction::I32Const(0));
        }
    }

    /// Store the compile-time-constant `bytes` into scratch at `offset`, one
    /// `i32.store8` per byte.
    fn store_const_bytes(&mut self, offset: usize, bytes: &[u8]) {
        for (i, &b) in bytes.iter().enumerate() {
            self.ins(&Instruction::LocalGet(self.scratch_local));
            self.ins(&Instruction::I32Const(b as i32));
            self.ins(&Instruction::I32Store8(MemArg {
                offset: (offset + i) as u64,
                align: 0,
                memory_index: 0,
            }));
        }
    }

    /// Store each variable's boxed `i32` pointer into scratch at `offset + i*4`.
    fn store_ptr_array(&mut self, offset: usize, vars: &[VarId]) -> Result<(), WasmError> {
        for (i, v) in vars.iter().enumerate() {
            self.ins(&Instruction::LocalGet(self.scratch_local));
            self.get(*v)?;
            self.ins(&Instruction::I32Store(MemArg {
                offset: (offset + i * 4) as u64,
                align: 2, // 2^2 == 4-byte alignment
                memory_index: 0,
            }));
        }
        Ok(())
    }

    /// Store a constant `i32` into scratch at `offset`.
    fn store_const_i32(&mut self, offset: usize, val: i32) {
        self.ins(&Instruction::LocalGet(self.scratch_local));
        self.ins(&Instruction::I32Const(val));
        self.ins(&Instruction::I32Store(MemArg {
            offset: offset as u64,
            align: 2,
            memory_index: 0,
        }));
    }

    /// Store a constant `i64` into scratch at `offset` (8-byte aligned by the
    /// caller's layout).
    fn store_const_i64(&mut self, offset: usize, val: i64) {
        self.ins(&Instruction::LocalGet(self.scratch_local));
        self.ins(&Instruction::I64Const(val));
        self.ins(&Instruction::I64Store(MemArg {
            offset: offset as u64,
            align: 3, // 2^3 == 8-byte alignment
            memory_index: 0,
        }));
    }

    /// Store a constant byte into scratch at `offset` (an `i32.store8`).
    fn store_const_i8(&mut self, offset: usize, val: i32) {
        self.ins(&Instruction::LocalGet(self.scratch_local));
        self.ins(&Instruction::I32Const(val));
        self.ins(&Instruction::I32Store8(MemArg {
            offset: offset as u64,
            align: 0,
            memory_index: 0,
        }));
    }
}

/// Round `n` up to the next multiple of `a` (a power of two).
fn align_up(n: usize, a: usize) -> usize {
    (n + a - 1) & !(a - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Block, Terminator};
    use std::sync::Arc;

    fn validate(bytes: &[u8]) {
        wasmparser::Validator::new()
            .validate_all(bytes)
            .expect("emitted module should validate");
    }

    /// Whether any function body contains a `return_call` operator — a robust
    /// replacement for scanning the whole module for the raw `0x12` opcode byte
    /// (which can collide with section-size / index LEB bytes).
    fn contains_return_call(bytes: &[u8]) -> bool {
        use wasmparser::{Parser, Payload};
        for payload in Parser::new(0).parse_all(bytes) {
            if let Ok(Payload::CodeSectionEntry(body)) = payload
                && let Ok(reader) = body.get_operators_reader()
            {
                for op in reader {
                    if matches!(op, Ok(wasmparser::Operator::ReturnCall { .. })) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn compile(f: &IrFunction) -> Result<Vec<u8>, WasmError> {
        let s = super::super::reloop::reloop(f).expect("reloop");
        emit_function(f, &s, &WasmBackend::default())
    }

    /// Whether the module imports the named `rt_abi` bridge from `"rt"`.
    fn imports_rt(bytes: &[u8], name: &str) -> bool {
        use wasmparser::{Parser, Payload, TypeRef};
        for payload in Parser::new(0).parse_all(bytes) {
            if let Ok(Payload::ImportSection(reader)) = payload {
                // Each `Imports` group iterates into individual `Import`s.
                for group in reader.into_iter().flatten() {
                    for item in group {
                        if let Ok((_, imp)) = item
                            && imp.module == "rt"
                            && imp.name == name
                            && matches!(imp.ty, TypeRef::Func(_))
                        {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Number of *defined* (non-imported) functions in the module — one entry
    /// per code-section body.  A typed function contributes two (trampoline +
    /// typed body); every other function contributes one.
    fn defined_function_count(bytes: &[u8]) -> usize {
        use wasmparser::{Parser, Payload};
        for payload in Parser::new(0).parse_all(bytes) {
            if let Ok(Payload::CodeSectionStart { count, .. }) = payload {
                return count as usize;
            }
        }
        0
    }

    /// `(fn [x] (+ x 1))`-shaped IR: one block, a constant, an add, a return.
    #[test]
    fn arithmetic_function_validates() {
        let mut f = IrFunction::new(Some(Arc::from("addone")), None);
        let x = f.fresh_var();
        f.params = vec![(Arc::from("x"), x)];
        let one = f.fresh_var();
        let sum = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![
                Inst::Const(one, Const::Long(1)),
                Inst::CallKnown(sum, KnownFn::Add, vec![x, one]),
            ],
            terminator: Terminator::Return(sum),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
    }

    /// Diamond with a φ at the merge: `(if c a b)` joining two values.
    #[test]
    fn diamond_with_phi_validates() {
        let mut f = IrFunction::new(Some(Arc::from("pick")), None);
        let c = f.fresh_var();
        let a = f.fresh_var();
        let bb = f.fresh_var();
        f.params = vec![
            (Arc::from("c"), c),
            (Arc::from("a"), a),
            (Arc::from("b"), bb),
        ];
        let (b0, b1, b2, b3) = (
            f.fresh_block(),
            f.fresh_block(),
            f.fresh_block(),
            f.fresh_block(),
        );
        let phi = f.fresh_var();
        f.blocks.push(Block {
            id: b0,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Branch {
                cond: c,
                then_block: b1,
                else_block: b2,
            },
        });
        f.blocks.push(Block {
            id: b1,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Jump(b3),
        });
        f.blocks.push(Block {
            id: b2,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Jump(b3),
        });
        f.blocks.push(Block {
            id: b3,
            phis: vec![Inst::Phi(phi, vec![(b1, a), (b2, bb)])],
            insts: vec![],
            terminator: Terminator::Return(phi),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
    }

    /// Loop with a φ counter and a conditional `recur`, validating loop + φ
    /// parallel-move + continue.
    #[test]
    fn loop_with_phi_validates() {
        // b0(header): n = phi[(entry,n0),(b1,n1)]; cond = (< n n); branch b1/b2
        // b1: n1 = (+ n n); recur -> b0
        // b2: return n
        let mut f = IrFunction::new(Some(Arc::from("spin")), None);
        let n0 = f.fresh_var();
        f.params = vec![(Arc::from("n0"), n0)];
        let (b0, b1, b2) = (f.fresh_block(), f.fresh_block(), f.fresh_block());
        let n = f.fresh_var();
        let cond = f.fresh_var();
        let n1 = f.fresh_var();
        f.blocks.push(Block {
            id: b0,
            phis: vec![Inst::Phi(n, vec![(b1, n1)])],
            insts: vec![Inst::CallKnown(cond, KnownFn::Lt, vec![n, n])],
            terminator: Terminator::Branch {
                cond,
                then_block: b1,
                else_block: b2,
            },
        });
        f.blocks.push(Block {
            id: b1,
            phis: vec![],
            insts: vec![Inst::CallKnown(n1, KnownFn::Add, vec![n, n])],
            terminator: Terminator::RecurJump {
                target: b0,
                args: vec![n1],
            },
        });
        f.blocks.push(Block {
            id: b2,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Return(n),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
    }

    /// A region-parameterised callee variant: `RegionParam` binds the hidden
    /// trailing handle, and a `RegionAlloc` bump-allocates into it.  The wasm
    /// signature gains one trailing `i32` (mirroring `abi_param_count`).
    #[test]
    fn region_param_variant_validates() {
        use crate::ir::RegionAllocKind;
        let mut f = IrFunction::new(Some(Arc::from("rv__rg")), None);
        let x = f.fresh_var();
        f.params = vec![(Arc::from("x"), x)];
        let rh = f.fresh_var();
        let v = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![
                Inst::RegionParam(rh),
                Inst::RegionAlloc(v, rh, RegionAllocKind::Vector, vec![x]),
            ],
            terminator: Terminator::Return(v),
        });
        assert!(f.takes_region_param());
        assert_eq!(f.abi_param_count(), 2); // x + hidden region handle
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
    }

    /// A function-scoped region (the `optimize.rs` wrap): `RegionStart` opens an
    /// arena, `RegionAlloc` bump-allocates, `RegionEnd` frees it.  No hidden
    /// param — the handle is a plain local.
    #[test]
    fn function_scoped_region_validates() {
        use crate::ir::RegionAllocKind;
        let mut f = IrFunction::new(Some(Arc::from("scoped")), None);
        let x = f.fresh_var();
        f.params = vec![(Arc::from("x"), x)];
        let rh = f.fresh_var();
        let v = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![
                Inst::RegionStart(rh),
                Inst::RegionAlloc(v, rh, RegionAllocKind::Vector, vec![x]),
                Inst::RegionEnd(rh),
            ],
            terminator: Terminator::Return(v),
        });
        assert!(!f.takes_region_param());
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
    }

    /// Region-allocated map (pair count) and cons (two direct pointer args).
    #[test]
    fn region_alloc_map_and_cons_validate() {
        use crate::ir::RegionAllocKind;
        let mut f = IrFunction::new(Some(Arc::from("rmc")), None);
        let k = f.fresh_var();
        let val = f.fresh_var();
        f.params = vec![(Arc::from("k"), k), (Arc::from("val"), val)];
        let rh = f.fresh_var();
        let m = f.fresh_var();
        let c = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![
                Inst::RegionStart(rh),
                // Flattened [k, val]; the bridge receives a pair count of 1.
                Inst::RegionAlloc(m, rh, RegionAllocKind::Map, vec![k, val]),
                Inst::RegionAlloc(c, rh, RegionAllocKind::Cons, vec![k, val]),
                Inst::RegionEnd(rh),
            ],
            terminator: Terminator::Return(c),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
    }

    /// A direct call to a function not in the bundle cannot resolve to a wasm
    /// index, so it reports cleanly.
    #[test]
    fn call_direct_unknown_target_is_unsupported() {
        let mut f = IrFunction::new(Some(Arc::from("caller")), None);
        let x = f.fresh_var();
        f.params = vec![(Arc::from("x"), x)];
        let r = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![Inst::CallDirect(r, Arc::from("missing"), vec![x])],
            terminator: Terminator::Return(r),
        });
        match compile(&f) {
            Err(WasmError::Unsupported(msg)) => assert!(msg.contains("not in bundle")),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    /// A two-function bundle where `caller` directly calls `callee`: the
    /// `CallDirect` resolves to the callee's wasm index.
    #[test]
    fn bundle_direct_call_validates() {
        // callee: (fn [x] (+ x 1))
        let mut callee = IrFunction::new(Some(Arc::from("callee")), None);
        let cx = callee.fresh_var();
        callee.params = vec![(Arc::from("x"), cx)];
        let one = callee.fresh_var();
        let sum = callee.fresh_var();
        let cb = callee.fresh_block();
        callee.blocks.push(Block {
            id: cb,
            phis: vec![],
            insts: vec![
                Inst::Const(one, Const::Long(1)),
                Inst::CallKnown(sum, KnownFn::Add, vec![cx, one]),
            ],
            terminator: Terminator::Return(sum),
        });

        // caller: (fn [y] (callee y))
        let mut caller = IrFunction::new(Some(Arc::from("caller")), None);
        let y = caller.fresh_var();
        caller.params = vec![(Arc::from("y"), y)];
        let r = caller.fresh_var();
        let cb2 = caller.fresh_block();
        caller.blocks.push(Block {
            id: cb2,
            phis: vec![],
            insts: vec![Inst::CallDirect(r, Arc::from("callee"), vec![y])],
            terminator: Terminator::Return(r),
        });

        let bytes = super::super::compile_bundle(&[&caller, &callee], &WasmBackend::default())
            .expect("emit bundle");
        validate(&bytes);
    }

    /// `CallWithRegion` resolves to a bundled region-parameterised variant and
    /// threads the caller's region handle as the hidden trailing argument.
    #[test]
    fn bundle_call_with_region_validates() {
        use crate::ir::RegionAllocKind;

        // callee variant: takes a hidden trailing region, allocates into it.
        let mut callee = IrFunction::new(Some(Arc::from("callee__rg")), None);
        let cx = callee.fresh_var();
        callee.params = vec![(Arc::from("x"), cx)];
        let rh = callee.fresh_var();
        let v = callee.fresh_var();
        let cb = callee.fresh_block();
        callee.blocks.push(Block {
            id: cb,
            phis: vec![],
            insts: vec![
                Inst::RegionParam(rh),
                Inst::RegionAlloc(v, rh, RegionAllocKind::Vector, vec![cx]),
            ],
            terminator: Terminator::Return(v),
        });
        assert!(callee.takes_region_param());

        // caller: opens a region and calls the variant, passing its handle.
        let mut caller = IrFunction::new(Some(Arc::from("caller")), None);
        let y = caller.fresh_var();
        caller.params = vec![(Arc::from("y"), y)];
        let crh = caller.fresh_var();
        let r = caller.fresh_var();
        let cb2 = caller.fresh_block();
        caller.blocks.push(Block {
            id: cb2,
            phis: vec![],
            insts: vec![
                Inst::RegionStart(crh),
                Inst::CallWithRegion(r, Arc::from("callee__rg"), vec![y], crh),
                Inst::RegionEnd(crh),
            ],
            terminator: Terminator::Return(r),
        });

        let bytes = super::super::compile_bundle(&[&caller, &callee], &WasmBackend::default())
            .expect("emit bundle");
        validate(&bytes);
    }

    /// `(fn [f x] (f x))`: a dynamic call through `rt_call`, with the single
    /// argument marshalled into the scratch buffer.
    #[test]
    fn dynamic_call_validates() {
        let mut f = IrFunction::new(Some(Arc::from("apply1")), None);
        let g = f.fresh_var();
        let x = f.fresh_var();
        f.params = vec![(Arc::from("f"), g), (Arc::from("x"), x)];
        let r = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![Inst::Call(r, g, vec![x])],
            terminator: Terminator::Return(r),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
    }

    /// `(fn [f] (f))`: a zero-arg dynamic call passes a null args pointer and a
    /// zero count — no memory marshalling needed.
    #[test]
    fn dynamic_call_zero_args_validates() {
        let mut f = IrFunction::new(Some(Arc::from("apply0")), None);
        let g = f.fresh_var();
        f.params = vec![(Arc::from("f"), g)];
        let r = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![Inst::Call(r, g, vec![])],
            terminator: Terminator::Return(r),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
    }

    /// A bundle compiled from a top-level function flattens its subfunctions, so
    /// a `CallDirect` to a subfunction resolves.
    #[test]
    fn bundle_flattens_subfunctions() {
        let mut sub = IrFunction::new(Some(Arc::from("inner")), None);
        let sx = sub.fresh_var();
        sub.params = vec![(Arc::from("x"), sx)];
        let sb = sub.fresh_block();
        sub.blocks.push(Block {
            id: sb,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Return(sx),
        });

        let mut outer = IrFunction::new(Some(Arc::from("outer")), None);
        let oy = outer.fresh_var();
        outer.params = vec![(Arc::from("y"), oy)];
        let r = outer.fresh_var();
        let ob = outer.fresh_block();
        outer.blocks.push(Block {
            id: ob,
            phis: vec![],
            insts: vec![Inst::CallDirect(r, Arc::from("inner"), vec![oy])],
            terminator: Terminator::Return(r),
        });
        outer.subfunctions.push(sub);

        let bytes =
            super::super::compile_bundle(&[&outer], &WasmBackend::default()).expect("emit bundle");
        validate(&bytes);
    }

    #[test]
    fn unsupported_instruction_reports_cleanly() {
        // `Deref` is not yet lowered; it should report cleanly.
        let mut f = IrFunction::new(Some(Arc::from("deref")), None);
        let x = f.fresh_var();
        f.params = vec![(Arc::from("x"), x)];
        let v = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![Inst::Deref(v, x)],
            terminator: Terminator::Return(v),
        });
        match compile(&f) {
            Err(WasmError::Unsupported(msg)) => assert!(msg.contains("not yet lowered")),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    /// `(fn [x] [x 1])`: build a vector from two boxed elements — exercises the
    /// scratch-buffer marshalling and the imported linear memory.
    #[test]
    fn alloc_vector_validates() {
        let mut f = IrFunction::new(Some(Arc::from("pair")), None);
        let x = f.fresh_var();
        f.params = vec![(Arc::from("x"), x)];
        let one = f.fresh_var();
        let v = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![
                Inst::Const(one, Const::Long(1)),
                Inst::AllocVector(v, vec![x, one]),
            ],
            terminator: Terminator::Return(v),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
    }

    /// An empty vector literal passes a null pointer and zero count — no memory
    /// import needed.
    #[test]
    fn alloc_empty_vector_validates() {
        let mut f = IrFunction::new(Some(Arc::from("empty")), None);
        let v = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![Inst::AllocVector(v, vec![])],
            terminator: Terminator::Return(v),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
    }

    /// `(fn [k val] {k val})`: build a map from one key/value pair — the count
    /// passed to the bridge is the pair count, not the pointer count.
    #[test]
    fn alloc_map_validates() {
        let mut f = IrFunction::new(Some(Arc::from("m")), None);
        let k = f.fresh_var();
        let val = f.fresh_var();
        f.params = vec![(Arc::from("k"), k), (Arc::from("val"), val)];
        let m = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![Inst::AllocMap(m, vec![(k, val)])],
            terminator: Terminator::Return(m),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
    }

    /// `(fn [h t] (cons h t))`: a cons cell takes two pointer args, no array.
    #[test]
    fn alloc_cons_validates() {
        let mut f = IrFunction::new(Some(Arc::from("c")), None);
        let h = f.fresh_var();
        let t = f.fresh_var();
        f.params = vec![(Arc::from("h"), h), (Arc::from("t"), t)];
        let c = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![Inst::AllocCons(c, h, t)],
            terminator: Terminator::Return(c),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
    }

    /// `(fn [] ["hi" :kw 'sym])`: each string-like constant interns its bytes
    /// into the rodata pool and resolves to a `(ptr, len)` pair; the module
    /// imports memory and emits a data segment.
    #[test]
    fn string_like_constants_validate() {
        let mut f = IrFunction::new(Some(Arc::from("consts")), None);
        let s = f.fresh_var();
        let k = f.fresh_var();
        let sy = f.fresh_var();
        let v = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![
                Inst::Const(s, Const::Str(Arc::from("hi"))),
                Inst::Const(k, Const::Keyword(Arc::from("kw"))),
                Inst::Const(sy, Const::Symbol(Arc::from("sym"))),
                Inst::AllocVector(v, vec![s, k, sy]),
            ],
            terminator: Terminator::Return(v),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
        // A non-empty data section (id 11) is present for the constant bytes.
        assert!(
            bytes.contains(&0x0b),
            "module should contain a data section for the rodata pool"
        );
    }

    /// Identical string constants share one set of bytes in the rodata pool:
    /// the bytes appear exactly once even when the constant is used twice.
    #[test]
    fn duplicate_string_constants_dedupe() {
        let mut f = IrFunction::new(Some(Arc::from("dup")), None);
        let a = f.fresh_var();
        let b = f.fresh_var();
        let v = f.fresh_var();
        let blk = f.fresh_block();
        f.blocks.push(Block {
            id: blk,
            phis: vec![],
            insts: vec![
                Inst::Const(a, Const::Str(Arc::from("hello"))),
                Inst::Const(b, Const::Str(Arc::from("hello"))),
                Inst::AllocVector(v, vec![a, b]),
            ],
            terminator: Terminator::Return(v),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
        // "hello" appears once in the emitted module (the deduplicated pool),
        // not twice.
        let needle = b"hello";
        let count = bytes.windows(needle.len()).filter(|w| *w == needle).count();
        assert_eq!(
            count, 1,
            "deduplicated pool should hold one copy of the bytes"
        );
    }

    /// `(fn [] clojure.core/+)`: a `LoadGlobal` resolves a namespaced binding
    /// through `rt_load_global`, with the ns/name bytes drawn from the rodata
    /// pool (so the module imports memory and emits a data segment).
    #[test]
    fn load_global_validates() {
        let mut f = IrFunction::new(Some(Arc::from("g")), None);
        let v = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![Inst::LoadGlobal(
                v,
                Arc::from("clojure.core"),
                Arc::from("+"),
            )],
            terminator: Terminator::Return(v),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
        assert!(
            bytes.contains(&0x0b),
            "module should contain a data section for the ns/name bytes"
        );
    }

    /// `(def my.ns/x v)` followed by `(set! var v)`: `DefVar` interns the var
    /// with a value, `LoadVar` resolves the Var object, and `SetBang` mutates it
    /// (its bridge result is dropped, leaving a balanced stack).
    #[test]
    fn def_var_load_var_set_bang_validate() {
        let mut f = IrFunction::new(Some(Arc::from("d")), None);
        let v = f.fresh_var();
        f.params = vec![(Arc::from("v"), v)];
        let defd = f.fresh_var();
        let var_obj = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![
                Inst::DefVar(defd, Arc::from("my.ns"), Arc::from("x"), v),
                Inst::LoadVar(var_obj, Arc::from("my.ns"), Arc::from("x")),
                Inst::SetBang(var_obj, v),
            ],
            terminator: Terminator::Return(defd),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
    }

    /// `(fn [e] (throw e))`: `Throw` stashes the exception via `rt_throw`
    /// (result dropped) and the block falls into its `unreachable` terminator,
    /// mirroring the native backend's thread-local error path.
    #[test]
    fn throw_validates() {
        let mut f = IrFunction::new(Some(Arc::from("boom")), None);
        let e = f.fresh_var();
        f.params = vec![(Arc::from("e"), e)];
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![Inst::Throw(e)],
            terminator: Terminator::Unreachable,
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
    }

    /// `(fn [body catch finally] (try ...))`: `KnownFn::TryCatchFinally` lowers
    /// to the three-arg `rt_try` bridge over the boxed thunks.
    #[test]
    fn try_catch_finally_validates() {
        let mut f = IrFunction::new(Some(Arc::from("guarded")), None);
        let body = f.fresh_var();
        let catch = f.fresh_var();
        let finally = f.fresh_var();
        f.params = vec![
            (Arc::from("body"), body),
            (Arc::from("catch"), catch),
            (Arc::from("finally"), finally),
        ];
        let r = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![Inst::CallKnown(
                r,
                KnownFn::TryCatchFinally,
                vec![body, catch, finally],
            )],
            terminator: Terminator::Return(r),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
    }

    /// Whether any function body contains a given non-control opcode, detected
    /// via `wasmparser` operator iteration (robust against LEB byte collisions).
    fn body_has<F: Fn(&wasmparser::Operator) -> bool>(bytes: &[u8], pred: F) -> bool {
        use wasmparser::{Parser, Payload};
        for payload in Parser::new(0).parse_all(bytes) {
            if let Ok(Payload::CodeSectionEntry(body)) = payload
                && let Ok(reader) = body.get_operators_reader()
            {
                for op in reader {
                    if op.as_ref().map(&pred).unwrap_or(false) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// `(let [a 2 b 3] (< a b))`: two unboxed long constants and an unboxed
    /// comparison — the result is a raw `i32` boolean, boxed only at the return.
    /// Asserts the emitter used `i64.lt_s` (not the `rt_lt` bridge).
    #[test]
    fn unboxed_long_compare() {
        let mut f = IrFunction::new(Some(Arc::from("cmp")), None);
        let a = f.fresh_var();
        let b = f.fresh_var();
        let c = f.fresh_var();
        let blk = f.fresh_block();
        f.blocks.push(Block {
            id: blk,
            phis: vec![],
            insts: vec![
                Inst::Const(a, Const::Long(2)),
                Inst::Const(b, Const::Long(3)),
                Inst::CallKnown(c, KnownFn::Lt, vec![a, b]),
            ],
            terminator: Terminator::Return(c),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
        assert!(
            body_has(&bytes, |op| matches!(op, wasmparser::Operator::I64LtS)),
            "long comparison should lower to i64.lt_s"
        );
    }

    /// `(+ 1.5 2.5)`: unboxed `f64` arithmetic — `f64.add`, boxed at the return.
    #[test]
    fn unboxed_double_add() {
        let mut f = IrFunction::new(Some(Arc::from("dadd")), None);
        let a = f.fresh_var();
        let b = f.fresh_var();
        let s = f.fresh_var();
        let blk = f.fresh_block();
        f.blocks.push(Block {
            id: blk,
            phis: vec![],
            insts: vec![
                Inst::Const(a, Const::Double(1.5)),
                Inst::Const(b, Const::Double(2.5)),
                Inst::CallKnown(s, KnownFn::Add, vec![a, b]),
            ],
            terminator: Terminator::Return(s),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
        assert!(
            body_has(&bytes, |op| matches!(op, wasmparser::Operator::F64Add)),
            "double addition should lower to f64.add"
        );
    }

    /// A loop accumulator: `(loop [i 0 acc 0] (if (< i n) (recur (inc i) (+ acc i)) acc))`
    /// shape.  With no param spec, `i`/`acc` still infer unboxed `Long` from the
    /// `0` seeds, so the `+`s lower to checked `i64.add` (overflow → throw) while
    /// the `(< i n)` against the boxed param `n` stays on the `rt_lt` bridge —
    /// exercising checked unboxed add *and* a boxed-compare with one unboxed
    /// operand (boxed on demand).  Mirrors `typeinfer`'s loop-counter test.
    #[test]
    fn unboxed_loop_accumulator() {
        let mut f = IrFunction::new(Some(Arc::from("sum")), None);
        let n = f.fresh_var();
        f.params = vec![(Arc::from("n"), n)];

        let entry = f.fresh_block();
        let header = f.fresh_block();
        let body = f.fresh_block();
        let exit = f.fresh_block();

        let zero1 = f.fresh_var();
        let zero2 = f.fresh_var();
        let i = f.fresh_var();
        let acc = f.fresh_var();
        let cond = f.fresh_var();
        let one = f.fresh_var();
        let i2 = f.fresh_var();
        let acc2 = f.fresh_var();

        f.blocks.push(Block {
            id: entry,
            phis: vec![],
            insts: vec![
                Inst::Const(zero1, Const::Long(0)),
                Inst::Const(zero2, Const::Long(0)),
            ],
            // Forward entry into the loop header (the back edge is `body`'s
            // `RecurJump`); mirrors how `loop` lowering enters the header.
            terminator: Terminator::Jump(header),
        });
        f.blocks.push(Block {
            id: header,
            phis: vec![
                Inst::Phi(i, vec![(entry, zero1), (body, i2)]),
                Inst::Phi(acc, vec![(entry, zero2), (body, acc2)]),
            ],
            insts: vec![Inst::CallKnown(cond, KnownFn::Lt, vec![i, n])],
            terminator: Terminator::Branch {
                cond,
                then_block: body,
                else_block: exit,
            },
        });
        f.blocks.push(Block {
            id: body,
            phis: vec![],
            insts: vec![
                Inst::Const(one, Const::Long(1)),
                Inst::CallKnown(i2, KnownFn::Add, vec![i, one]),
                Inst::CallKnown(acc2, KnownFn::Add, vec![acc, i]),
            ],
            terminator: Terminator::RecurJump {
                target: header,
                args: vec![i2, acc2],
            },
        });
        f.blocks.push(Block {
            id: exit,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Return(acc),
        });

        let bytes = compile(&f).expect("emit");
        validate(&bytes);
        // Checked unboxed long addition (i64.add) and its overflow guard (i64.lt_s).
        assert!(
            body_has(&bytes, |op| matches!(op, wasmparser::Operator::I64Add)),
            "unboxed accumulator should use i64.add"
        );
    }

    /// `(* a b)` on two longs is **demoted** to the boxed `rt_mul` bridge (wasm
    /// lacks an `i64.mul_hi` for the 128-bit overflow check), so it still
    /// validates with no `i64.mul` in the body.
    #[test]
    fn long_mul_demotes_to_boxed() {
        let mut f = IrFunction::new(Some(Arc::from("prod")), None);
        let a = f.fresh_var();
        let b = f.fresh_var();
        let p = f.fresh_var();
        let blk = f.fresh_block();
        f.blocks.push(Block {
            id: blk,
            phis: vec![],
            insts: vec![
                Inst::Const(a, Const::Long(6)),
                Inst::Const(b, Const::Long(7)),
                Inst::CallKnown(p, KnownFn::Mul, vec![a, b]),
            ],
            terminator: Terminator::Return(p),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
        assert!(
            !body_has(&bytes, |op| matches!(op, wasmparser::Operator::I64Mul)),
            "checked long multiply should stay on the boxed bridge"
        );
    }

    fn template(name: &str, fns: &[&str], pcs: &[usize], variadic: &[bool]) -> ClosureTemplate {
        ClosureTemplate {
            name: Some(Arc::from(name)),
            arity_fn_names: fns.iter().map(|s| Arc::from(*s)).collect(),
            param_counts: pcs.to_vec(),
            is_variadic: variadic.to_vec(),
            capture_names: vec![],
        }
    }

    /// `(fn outer [x] (fn inner [y] x))`: the outer function materializes a
    /// single-arity closure over its subfunction, capturing `x`.  The bundle
    /// imports the shared table, installs both functions via the element segment,
    /// and `rt_make_fn` receives the inner's table slot.
    #[test]
    fn alloc_closure_single_arity_validates() {
        let mut inner = IrFunction::new(Some(Arc::from("inner")), None);
        let iy = inner.fresh_var();
        inner.params = vec![(Arc::from("y"), iy)];
        let ib = inner.fresh_block();
        inner.blocks.push(Block {
            id: ib,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Return(iy),
        });

        let mut outer = IrFunction::new(Some(Arc::from("outer")), None);
        let ox = outer.fresh_var();
        outer.params = vec![(Arc::from("x"), ox)];
        let clo = outer.fresh_var();
        let ob = outer.fresh_block();
        outer.blocks.push(Block {
            id: ob,
            phis: vec![],
            insts: vec![Inst::AllocClosure(
                clo,
                template("inner", &["inner"], &[1], &[false]),
                vec![ox],
            )],
            terminator: Terminator::Return(clo),
        });
        outer.subfunctions.push(inner);

        let bytes =
            super::super::compile_bundle(&[&outer], &WasmBackend::default()).expect("emit bundle");
        validate(&bytes);
    }

    /// A variadic closure with no captures routes through `rt_make_fn_variadic`
    /// and passes a null captures pointer.
    #[test]
    fn alloc_closure_variadic_no_captures_validates() {
        let mut inner = IrFunction::new(Some(Arc::from("v")), None);
        let iy = inner.fresh_var();
        inner.params = vec![(Arc::from("y"), iy)];
        let ib = inner.fresh_block();
        inner.blocks.push(Block {
            id: ib,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Return(iy),
        });

        let mut outer = IrFunction::new(Some(Arc::from("mk")), None);
        let clo = outer.fresh_var();
        let ob = outer.fresh_block();
        outer.blocks.push(Block {
            id: ob,
            phis: vec![],
            insts: vec![Inst::AllocClosure(
                clo,
                template("v", &["v"], &[1], &[true]),
                vec![],
            )],
            terminator: Terminator::Return(clo),
        });
        outer.subfunctions.push(inner);

        let bytes =
            super::super::compile_bundle(&[&outer], &WasmBackend::default()).expect("emit bundle");
        validate(&bytes);
    }

    /// A multi-arity closure routes through `rt_make_fn_multi`, marshalling the
    /// fn-pointer / param-count / variadic-flag arrays into one scratch buffer
    /// alongside a capture.
    #[test]
    fn alloc_closure_multi_arity_validates() {
        let mut a0 = IrFunction::new(Some(Arc::from("f__0")), None);
        let a0v = a0.fresh_var();
        let a0b = a0.fresh_block();
        a0.blocks.push(Block {
            id: a0b,
            phis: vec![],
            insts: vec![Inst::Const(a0v, Const::Nil)],
            terminator: Terminator::Return(a0v),
        });

        let mut a1 = IrFunction::new(Some(Arc::from("f__1")), None);
        let a1x = a1.fresh_var();
        a1.params = vec![(Arc::from("x"), a1x)];
        let a1b = a1.fresh_block();
        a1.blocks.push(Block {
            id: a1b,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Return(a1x),
        });

        let mut outer = IrFunction::new(Some(Arc::from("mk")), None);
        let cap = outer.fresh_var();
        outer.params = vec![(Arc::from("c"), cap)];
        let clo = outer.fresh_var();
        let ob = outer.fresh_block();
        outer.blocks.push(Block {
            id: ob,
            phis: vec![],
            insts: vec![Inst::AllocClosure(
                clo,
                template("f", &["f__0", "f__1"], &[0, 1], &[false, false]),
                vec![cap],
            )],
            terminator: Terminator::Return(clo),
        });
        outer.subfunctions.push(a0);
        outer.subfunctions.push(a1);

        let bytes =
            super::super::compile_bundle(&[&outer], &WasmBackend::default()).expect("emit bundle");
        validate(&bytes);
    }

    /// A closure over an arity function not in the bundle cannot resolve a table
    /// slot, so it reports cleanly.
    #[test]
    fn alloc_closure_unknown_arity_is_unsupported() {
        let mut f = IrFunction::new(Some(Arc::from("mk")), None);
        let clo = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![Inst::AllocClosure(
                clo,
                template("gone", &["gone"], &[0], &[false]),
                vec![],
            )],
            terminator: Terminator::Return(clo),
        });
        match compile(&f) {
            Err(WasmError::Unsupported(msg)) => assert!(msg.contains("not in bundle")),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    /// A closure with zero compiled arities falls back to nil (no table needed).
    #[test]
    fn alloc_closure_no_arities_is_nil() {
        let mut f = IrFunction::new(Some(Arc::from("mk")), None);
        let clo = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![Inst::AllocClosure(
                clo,
                template("e", &[], &[], &[]),
                vec![],
            )],
            terminator: Terminator::Return(clo),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
    }

    /// `(fn caller [y] (callee y))` in tail position: with `tail_calls` the
    /// trailing `CallDirect` becomes a `return_call`.
    #[test]
    fn tail_call_direct_emits_return_call() {
        let mut callee = IrFunction::new(Some(Arc::from("callee")), None);
        let cx = callee.fresh_var();
        callee.params = vec![(Arc::from("x"), cx)];
        let cb = callee.fresh_block();
        callee.blocks.push(Block {
            id: cb,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Return(cx),
        });

        let mut caller = IrFunction::new(Some(Arc::from("caller")), None);
        let y = caller.fresh_var();
        caller.params = vec![(Arc::from("y"), y)];
        let r = caller.fresh_var();
        let cb2 = caller.fresh_block();
        caller.blocks.push(Block {
            id: cb2,
            phis: vec![],
            insts: vec![Inst::CallDirect(r, Arc::from("callee"), vec![y])],
            terminator: Terminator::Return(r),
        });

        // tail_calls on: validates (return_call is a default-enabled proposal).
        let cfg = WasmBackend {
            tail_calls: true,
            exceptions: true,
        };
        let on = super::super::compile_bundle(&[&caller, &callee], &cfg).expect("emit");
        validate(&on);

        // tail_calls off: ordinary call + return, still valid.
        let cfg_off = WasmBackend {
            tail_calls: false,
            exceptions: true,
        };
        let off = super::super::compile_bundle(&[&caller, &callee], &cfg_off).expect("emit");
        validate(&off);

        // The tail-call build emits a `return_call`; the non-tail build does not.
        assert!(
            contains_return_call(&on),
            "tail_calls build should contain return_call"
        );
        assert!(
            !contains_return_call(&off),
            "non-tail build should not contain return_call"
        );
    }

    /// A `CallWithRegion` in tail position threads the region handle as the
    /// trailing argument of the `return_call`.
    #[test]
    fn tail_call_with_region_validates() {
        let mut callee = IrFunction::new(Some(Arc::from("callee__rg")), None);
        let cx = callee.fresh_var();
        callee.params = vec![(Arc::from("x"), cx)];
        let crh = callee.fresh_var();
        let cb = callee.fresh_block();
        callee.blocks.push(Block {
            id: cb,
            phis: vec![],
            insts: vec![Inst::RegionParam(crh)],
            terminator: Terminator::Return(cx),
        });
        assert!(callee.takes_region_param());

        let mut caller = IrFunction::new(Some(Arc::from("caller__rg")), None);
        let y = caller.fresh_var();
        caller.params = vec![(Arc::from("y"), y)];
        let rh = caller.fresh_var();
        let r = caller.fresh_var();
        let cb2 = caller.fresh_block();
        caller.blocks.push(Block {
            id: cb2,
            phis: vec![],
            insts: vec![
                Inst::RegionParam(rh),
                Inst::CallWithRegion(r, Arc::from("callee__rg"), vec![y], rh),
            ],
            terminator: Terminator::Return(r),
        });

        let bytes = super::super::compile_bundle(&[&caller, &callee], &WasmBackend::default())
            .expect("emit");
        validate(&bytes);
    }

    #[test]
    fn function_signature_region_and_long_hint() {
        let mut f = IrFunction::new(Some(Arc::from("g")), None);
        let p = f.fresh_var();
        f.params = vec![(Arc::from("n"), p)];
        f.seed_reprs = vec![Repr::Long];
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Return(p),
        });
        let (params, results) = function_signature(&f);
        assert_eq!(params, vec![WasmValType::I64]);
        assert_eq!(results, vec![WasmValType::I32]);
    }

    // ── Typed parameter ABI (item 1) ─────────────────────────────────────────

    /// `(fn [^long n] (+ n 1))`: the `^long` hint makes the function compile to a
    /// typed body (its param is an unboxed `i64`, so the `+ 1` is a checked
    /// `i64.add`) plus a boxed-entry trampoline that coerces the boxed argument
    /// (`rt_coerce_long`) and `return_call`s the body.
    #[test]
    fn typed_long_param_emits_trampoline_and_typed_body() {
        let mut f = IrFunction::new(Some(Arc::from("addone")), None);
        let x = f.fresh_var();
        f.params = vec![(Arc::from("x"), x)];
        f.seed_reprs = vec![Repr::Long];
        let one = f.fresh_var();
        let sum = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![
                Inst::Const(one, Const::Long(1)),
                Inst::CallKnown(sum, KnownFn::Add, vec![x, one]),
            ],
            terminator: Terminator::Return(sum),
        });

        let bytes = compile(&f).expect("emit");
        validate(&bytes);
        // Trampoline + typed body.
        assert_eq!(defined_function_count(&bytes), 2);
        // The trampoline coerces the `^long` argument.
        assert!(imports_rt(&bytes, "rt_coerce_long"));
        // The trampoline tail-calls the typed body (tail calls on by default).
        assert!(contains_return_call(&bytes));
    }

    /// `(fn [^double x] (+ x 1.0))`: the `^double` hint yields an unboxed `f64`
    /// param (so `+ 1.0` is an `f64.add`) and a trampoline coercing via
    /// `rt_coerce_double`.
    #[test]
    fn typed_double_param_coerces_via_double_bridge() {
        let mut f = IrFunction::new(Some(Arc::from("addhalf")), None);
        let x = f.fresh_var();
        f.params = vec![(Arc::from("x"), x)];
        f.seed_reprs = vec![Repr::Double];
        let one = f.fresh_var();
        let sum = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![
                Inst::Const(one, Const::Double(1.0)),
                Inst::CallKnown(sum, KnownFn::Add, vec![x, one]),
            ],
            terminator: Terminator::Return(sum),
        });

        let bytes = compile(&f).expect("emit");
        validate(&bytes);
        assert_eq!(defined_function_count(&bytes), 2);
        assert!(imports_rt(&bytes, "rt_coerce_double"));
        assert!(!imports_rt(&bytes, "rt_coerce_long"));
    }

    /// With the tail-call proposal disabled the trampoline falls back to a plain
    /// `call` + return, so no `return_call` appears (a function that merely
    /// returns its `^long` param has no other call site).
    #[test]
    fn typed_param_trampoline_without_tail_calls_uses_plain_call() {
        let mut f = IrFunction::new(Some(Arc::from("identity_long")), None);
        let n = f.fresh_var();
        f.params = vec![(Arc::from("n"), n)];
        f.seed_reprs = vec![Repr::Long];
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Return(n),
        });
        let s = super::super::reloop::reloop(&f).expect("reloop");

        let with_tc = emit_function(&f, &s, &WasmBackend::default()).expect("emit");
        validate(&with_tc);
        assert!(contains_return_call(&with_tc));

        let no_tc = emit_function(
            &f,
            &s,
            &WasmBackend {
                tail_calls: false,
                exceptions: true,
            },
        )
        .expect("emit");
        validate(&no_tc);
        assert!(!contains_return_call(&no_tc));
    }

    /// A non-typed function is unaffected: `(fn [x] (+ x 1))` with no hint emits a
    /// single boxed body and no coercion import.
    #[test]
    fn untyped_function_unchanged_single_body() {
        let mut f = IrFunction::new(Some(Arc::from("addone_boxed")), None);
        let x = f.fresh_var();
        f.params = vec![(Arc::from("x"), x)];
        let one = f.fresh_var();
        let sum = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![
                Inst::Const(one, Const::Long(1)),
                Inst::CallKnown(sum, KnownFn::Add, vec![x, one]),
            ],
            terminator: Terminator::Return(sum),
        });
        let bytes = compile(&f).expect("emit");
        validate(&bytes);
        assert_eq!(defined_function_count(&bytes), 1);
        assert!(!imports_rt(&bytes, "rt_coerce_long"));
    }

    /// A boxed caller reaches a typed callee through its trampoline: a `CallDirect`
    /// to `(fn [^long n] n)` resolves to the callee's primary (the boxed
    /// trampoline), so the caller passes a boxed argument and the bundle still
    /// validates.  The bundle has three defined functions: the caller's boxed
    /// body, the callee trampoline, and the callee typed body.
    #[test]
    fn boxed_caller_reaches_typed_callee_via_trampoline() {
        let mut callee = IrFunction::new(Some(Arc::from("id_long")), None);
        let cn = callee.fresh_var();
        callee.params = vec![(Arc::from("n"), cn)];
        callee.seed_reprs = vec![Repr::Long];
        let cb = callee.fresh_block();
        callee.blocks.push(Block {
            id: cb,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Return(cn),
        });

        let mut caller = IrFunction::new(Some(Arc::from("call_id")), None);
        let y = caller.fresh_var();
        caller.params = vec![(Arc::from("y"), y)];
        let r = caller.fresh_var();
        let bb = caller.fresh_block();
        caller.blocks.push(Block {
            id: bb,
            phis: vec![],
            insts: vec![Inst::CallDirect(r, Arc::from("id_long"), vec![y])],
            terminator: Terminator::Return(r),
        });

        let bytes = super::super::compile_bundle(&[&caller, &callee], &WasmBackend::default())
            .expect("emit");
        validate(&bytes);
        assert_eq!(defined_function_count(&bytes), 3);
        assert!(imports_rt(&bytes, "rt_coerce_long"));
    }
}
