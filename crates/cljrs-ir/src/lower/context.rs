//! Lowering context — mutable builder state for ANF IR construction.
//!
//! Mirrors `cljrs.compiler.ir` (the Clojure atom-based builder context).
//! All methods take `&mut self` instead of a Clojure atom.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::{Block, BlockId, Inst, IrFunction, Terminator, VarId};

// ── Global name counter (shared across all lowering calls in the process) ────

static GLOBAL_NAME_CTR: AtomicU32 = AtomicU32::new(0);

pub fn fresh_global_name_id() -> u32 {
    GLOBAL_NAME_CTR.fetch_add(1, Ordering::Relaxed)
}

// ── Builder context ───────────────────────────────────────────────────────────

pub struct LowerCtx {
    pub(crate) name: Option<Arc<str>>,
    pub(crate) ns: Arc<str>,

    /// Fully finalized blocks (finish_block moves the current state here).
    pub(crate) finished_blocks: Vec<Block>,

    /// ID of the block currently being built.
    pub(crate) current_block_id: u32,

    /// Phi instructions accumulating for the current block.
    pub(crate) current_phis: Vec<Inst>,

    /// Non-phi instructions accumulating for the current block.
    pub(crate) current_insts: Vec<Inst>,

    /// Scope stack: each entry is a map of local name → VarId.
    /// The last entry is the innermost scope.
    pub(crate) locals: Vec<HashMap<Arc<str>, VarId>>,

    /// Stack of (header_block_id, phi_var_ids) for nested loops.
    pub(crate) loop_headers: Vec<(BlockId, Vec<VarId>)>,

    pub(crate) next_var: u32,
    pub(crate) next_block: u32,

    /// Collected subfunctions from nested `fn*` forms.
    pub(crate) subfunctions: Vec<IrFunction>,
}

impl LowerCtx {
    pub fn new(name: Option<Arc<str>>, ns: Arc<str>) -> Self {
        Self {
            name,
            ns,
            finished_blocks: Vec::new(),
            current_block_id: 0,
            current_phis: Vec::new(),
            current_insts: Vec::new(),
            // One initial (outer) scope
            locals: vec![HashMap::new()],
            loop_headers: Vec::new(),
            next_var: 0,
            // Block 0 is the entry block (already "started")
            next_block: 1,
            subfunctions: Vec::new(),
        }
    }

    // ── ID allocation ────────────────────────────────────────────────────────

    pub fn fresh_var(&mut self) -> VarId {
        let id = VarId(self.next_var);
        self.next_var += 1;
        id
    }

    pub fn fresh_block(&mut self) -> BlockId {
        let id = BlockId(self.next_block);
        self.next_block += 1;
        id
    }

    // ── Instruction emission ─────────────────────────────────────────────────

    pub fn emit(&mut self, inst: Inst) {
        if matches!(inst, Inst::Phi(..)) {
            self.current_phis.push(inst);
        } else {
            self.current_insts.push(inst);
        }
    }

    pub fn emit_const(&mut self, c: crate::Const) -> VarId {
        let dst = self.fresh_var();
        self.emit(Inst::Const(dst, c));
        dst
    }

    pub fn emit_phi(&mut self, dst: VarId, entries: Vec<(BlockId, VarId)>) {
        self.emit(Inst::Phi(dst, entries));
    }

    // ── Block control ────────────────────────────────────────────────────────

    pub fn finish_block(&mut self, terminator: Terminator) {
        let block = Block {
            id: BlockId(self.current_block_id),
            phis: std::mem::take(&mut self.current_phis),
            insts: std::mem::take(&mut self.current_insts),
            terminator,
        };
        self.finished_blocks.push(block);
    }

    pub fn start_block(&mut self, id: BlockId) {
        self.current_block_id = id.0;
    }

    pub fn current_block_id(&self) -> BlockId {
        BlockId(self.current_block_id)
    }

    // ── Scope management ─────────────────────────────────────────────────────

    pub fn push_scope(&mut self) {
        self.locals.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        self.locals.pop();
    }

    pub fn bind_local(&mut self, name: Arc<str>, var: VarId) {
        if let Some(scope) = self.locals.last_mut() {
            scope.insert(name, var);
        }
    }

    pub fn lookup_local(&self, name: &str) -> Option<VarId> {
        for scope in self.locals.iter().rev() {
            if let Some(&v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }

    /// Collect all locals across all scopes (inner scopes shadow outer).
    /// Returns `(name, var_id)` pairs for use as closure captures.
    pub fn get_all_locals(&self) -> Vec<(Arc<str>, VarId)> {
        let mut map: HashMap<Arc<str>, VarId> = HashMap::new();
        for scope in &self.locals {
            for (k, v) in scope {
                map.insert(k.clone(), *v);
            }
        }
        let mut result: Vec<(Arc<str>, VarId)> = map.into_iter().collect();
        // Stable order to make capture lists deterministic.
        result.sort_by(|(a, _), (b, _)| a.as_ref().cmp(b.as_ref()));
        result
    }

    // ── Loop header stack ────────────────────────────────────────────────────

    pub fn push_loop_header(&mut self, header: BlockId, phi_vars: Vec<VarId>) {
        self.loop_headers.push((header, phi_vars));
    }

    pub fn pop_loop_header(&mut self) {
        self.loop_headers.pop();
    }

    pub fn current_loop_header(&self) -> Option<(BlockId, Vec<VarId>)> {
        self.loop_headers.last().cloned()
    }

    /// Patch a phi node in an already-finished block (for recur).
    ///
    /// The loop header is finalized before `lower_recur` runs, so we must
    /// find it in `finished_blocks` and mutate its phi list in place.
    pub fn update_phi_in_header(
        &mut self,
        header: BlockId,
        phi_index: usize,
        from_block: BlockId,
        var_id: VarId,
    ) {
        if let Some(block) = self.finished_blocks.iter_mut().find(|b| b.id == header) {
            if let Some(Inst::Phi(_, entries)) = block.phis.get_mut(phi_index) {
                entries.push((from_block, var_id));
            }
        }
    }

    // ── Subfunction collection ───────────────────────────────────────────────

    pub fn add_subfunction(&mut self, subfn: IrFunction) {
        self.subfunctions.push(subfn);
    }

    pub fn ns(&self) -> &Arc<str> {
        &self.ns
    }

    // ── Final assembly ───────────────────────────────────────────────────────

    /// Build the final `IrFunction`. Consumes the context.
    /// `params` must be set before calling this (the caller builds params first).
    pub fn build(self, params: Vec<(Arc<str>, VarId)>) -> IrFunction {
        IrFunction {
            name: self.name,
            params,
            blocks: self.finished_blocks,
            next_var: self.next_var,
            next_block: self.next_block,
            span: None,
            subfunctions: self.subfunctions,
        }
    }
}
