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
//! aligns the wasm signature with [`function_signature`] is a follow-up; until
//! then the emitter rejects region-parameterised and async poll functions
//! (whose ABIs add hidden params) with [`WasmError::Unsupported`].
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
//! # Status
//!
//! Emits valid, `wasmparser`-validated modules for the subset: scalar
//! constants, `LoadLocal`, boxed arithmetic (`+ - * / rem`, folded) and binary
//! comparison (`= < > <= >=`) via the `rt_abi` bridges, collection allocation
//! (`AllocVector`/`AllocMap`/`AllocSet`/`AllocList`/`AllocCons` — element arrays
//! marshalled through an imported linear memory and the `rt_scratch_ptr`
//! buffer), and all control flow (branches, diamonds, and `loop`/`recur` with
//! φ).  Calls, globals, string/keyword/symbol constants, and the region/async
//! ABIs return [`WasmError::Unsupported`] — the next lowering increments.

use std::collections::HashMap;

use wasm_encoder::{
    BlockType, CodeSection, EntityType, ExportKind, ExportSection, Function, FunctionSection,
    Ieee64, ImportSection, Instruction, MemArg, MemoryType, Module, TypeSection, ValType,
};

use crate::ir::{Block, BlockId, Const, Inst, IrFunction, KnownFn, Repr, Terminator, VarId};

use super::abi::{self, RtImport, WasmValType};
use super::reloop::Structured;
use super::{WasmBackend, WasmError};

/// Emit a wasm module containing `func` as a single exported function.
///
/// Pipeline: assign one `i32` local per [`VarId`], walk `structured` lowering
/// each [`Inst`], then assemble the type/import/function/export/code sections.
pub fn emit_function(
    func: &IrFunction,
    structured: &Structured,
    _cfg: &WasmBackend,
) -> Result<Vec<u8>, WasmError> {
    if func.takes_state_param() {
        return Err(WasmError::Unsupported(
            "async poll-function ABI (state-machine params)".into(),
        ));
    }
    if func.takes_region_param() {
        return Err(WasmError::Unsupported(
            "region-parameterised variant ABI (hidden region param)".into(),
        ));
    }

    // One i32 local per VarId: visible params first (wasm locals 0..n), then the
    // remaining VarIds as declared locals.
    let nparams = func.params.len();
    let mut local_of: HashMap<VarId, u32> = HashMap::new();
    for (i, (_, vid)) in func.params.iter().enumerate() {
        local_of.insert(*vid, i as u32);
    }
    let mut next_local = nparams as u32;
    for v in 0..func.next_var {
        let vid = VarId(v);
        local_of.entry(vid).or_insert_with(|| {
            let l = next_local;
            next_local += 1;
            l
        });
    }
    // One extra i32 local, past all VarId locals, holds the scratch-buffer
    // pointer transiently while marshalling an allocation's element array.
    let scratch_local = next_local;
    let declared = next_local + 1 - nparams as u32;

    let block_of: HashMap<BlockId, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, i))
        .collect();
    let forward_preds = forward_pred_counts(func);

    let mut body = Function::new([(declared, ValType::I32)]);
    let mut asm = ModuleAsm::new();

    {
        let mut em = Emitter {
            func,
            asm: &mut asm,
            body: &mut body,
            local_of: &local_of,
            block_of: &block_of,
            forward_preds: &forward_preds,
            scratch_local,
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

    // Assemble.  The main function type goes last; import types were interned
    // during emission.
    let main_ty = asm.intern_type(vec![WasmValType::I32; nparams], vec![WasmValType::I32]);

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
    module.section(&imports);

    let mut funcs = FunctionSection::new();
    funcs.function(main_ty);
    module.section(&funcs);

    let mut exports = ExportSection::new();
    let func_index = asm.imports.len() as u32; // imports occupy 0..k; this fn is k
    exports.export(
        func.name.as_deref().unwrap_or("main"),
        ExportKind::Func,
        func_index,
    );
    module.section(&exports);

    let mut code = CodeSection::new();
    code.function(&body);
    module.section(&code);

    Ok(module.finish())
}

/// The wasm function signature for `func`: `(params, results)`.
///
/// Honors the hidden trailing region param and the poll-function ABI, mirroring
/// [`IrFunction::abi_param_count`].  This describes the *eventual* typed ABI;
/// [`emit_function`] currently emits a boxed-only (all-`i32`) signature and
/// rejects the region/poll cases.
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

// ── Module assembly state ────────────────────────────────────────────────────

fn valtype(w: &WasmValType) -> ValType {
    match w {
        WasmValType::I32 => ValType::I32,
        WasmValType::I64 => ValType::I64,
        WasmValType::F64 => ValType::F64,
    }
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
}

impl ModuleAsm {
    fn new() -> Self {
        ModuleAsm {
            types: Vec::new(),
            type_map: HashMap::new(),
            imports: Vec::new(),
            import_map: HashMap::new(),
            needs_memory: false,
        }
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
    /// An i32 local (past all VarId locals) that transiently holds the
    /// scratch-buffer pointer while marshalling an allocation's element array.
    scratch_local: u32,
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

    fn get(&mut self, v: VarId) -> Result<(), WasmError> {
        let l = self.local(v)?;
        self.ins(&Instruction::LocalGet(l));
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
                // Branch on the boxed condition's truthiness.
                self.get(*cond)?;
                self.call_import("rt_truthiness")?;
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
        for (_, src) in &moves {
            self.get(*src)?;
        }
        for (dst, _) in moves.iter().rev() {
            self.set(*dst)?;
        }
        Ok(())
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

    fn emit_inst(&mut self, inst: &Inst) -> Result<(), WasmError> {
        match inst {
            Inst::Const(dst, c) => {
                self.emit_const(c)?;
                self.set(*dst)
            }
            // In compiled code locals are bound by params / let bindings; an
            // unresolved LoadLocal is nil, matching the Cranelift backend.
            Inst::LoadLocal(dst, _name) => {
                self.call_import("rt_const_nil")?;
                self.set(*dst)
            }
            Inst::CallKnown(dst, kf, args) => {
                self.emit_known(kf, args)?;
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
            // Strings/keywords/symbols need their bytes in a data segment
            // coordinated with the runtime's memory layout — a later increment.
            Const::Str(_) | Const::Keyword(_) | Const::Symbol(_) => Err(WasmError::Unsupported(
                "string/keyword/symbol constants (need a data segment)".into(),
            )),
        }
    }

    /// Lower a known-function call to its boxed `rt_abi` bridge, result left on
    /// the operand stack.
    fn emit_known(&mut self, kf: &KnownFn, args: &[VarId]) -> Result<(), WasmError> {
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

    fn compile(f: &IrFunction) -> Result<Vec<u8>, WasmError> {
        let s = super::super::reloop::reloop(f).expect("reloop");
        emit_function(f, &s, &WasmBackend::default())
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

    #[test]
    fn region_variant_is_unsupported() {
        let mut f = IrFunction::new(Some(Arc::from("rv")), None);
        let p = f.fresh_var();
        f.params = vec![(Arc::from("x"), p)];
        let rp = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![Inst::RegionParam(rp)],
            terminator: Terminator::Return(p),
        });
        assert!(f.takes_region_param());
        assert!(matches!(compile(&f), Err(WasmError::Unsupported(_))));
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
}
