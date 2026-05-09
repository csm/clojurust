//! Region-membership computation: which `(block, inst_index)` positions
//! fall inside the dynamic scope of each `RegionStart`/`RegionEnd` pair.
//!
//! A region is identified by the `VarId` that names the region handle.
//! Its scope is:
//!  * the *start* block, from the `RegionStart` inst onward;
//!  * every block on a CFG path between start and end (exclusive of
//!    transit-through-RegionEnd); and
//!  * the *end* block, up to and including the `RegionEnd` inst.

use std::collections::{HashMap, HashSet};

use cljrs_ir::{Block, BlockId, Inst, IrFunction, Terminator, VarId};

/// All information we need about a region for rendering.
#[derive(Debug, Clone)]
pub struct Region {
    /// The handle var that names this region.
    pub handle: VarId,
    /// The block containing the `RegionStart` inst.
    pub start_block: BlockId,
    /// Index of the `RegionStart` inst within `start_block.insts`.
    pub start_inst_idx: usize,
    /// The block containing the `RegionEnd` inst.
    pub end_block: BlockId,
    /// Index of the `RegionEnd` inst within `end_block.insts`.
    pub end_inst_idx: usize,
    /// Every block reachable from `start_block` that the region covers
    /// (paths terminating at `end_block`).
    pub blocks: HashSet<BlockId>,
}

/// Find every region in the function.
pub fn collect_regions(ir: &IrFunction) -> Vec<Region> {
    let mut starts: HashMap<VarId, (BlockId, usize)> = HashMap::new();
    let mut ends: HashMap<VarId, (BlockId, usize)> = HashMap::new();

    for block in &ir.blocks {
        for (idx, inst) in block.insts.iter().enumerate() {
            match inst {
                Inst::RegionStart(handle) => {
                    starts.insert(*handle, (block.id, idx));
                }
                Inst::RegionEnd(handle) => {
                    ends.insert(*handle, (block.id, idx));
                }
                _ => {}
            }
        }
    }

    let mut out = Vec::new();
    for (handle, (start_block, start_inst_idx)) in starts {
        let Some(&(end_block, end_inst_idx)) = ends.get(&handle) else {
            continue;
        };
        let blocks = blocks_on_path(ir, start_block, end_block);
        out.push(Region {
            handle,
            start_block,
            start_inst_idx,
            end_block,
            end_inst_idx,
            blocks,
        });
    }
    // Stable order (lower handle first) so colors are deterministic.
    out.sort_by_key(|r| r.handle.0);
    out
}

fn block_successors(block: &Block) -> Vec<BlockId> {
    match &block.terminator {
        Terminator::Jump(t) => vec![*t],
        Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        Terminator::RecurJump { target, .. } => vec![*target],
        _ => vec![],
    }
}

fn blocks_on_path(ir: &IrFunction, start: BlockId, end: BlockId) -> HashSet<BlockId> {
    let by_id: HashMap<BlockId, &Block> = ir.blocks.iter().map(|b| (b.id, b)).collect();
    let mut stack = vec![start];
    let mut seen: HashSet<BlockId> = HashSet::new();
    while let Some(b) = stack.pop() {
        if !seen.insert(b) {
            continue;
        }
        if b == end {
            continue;
        }
        if let Some(block) = by_id.get(&b) {
            for s in block_successors(block) {
                stack.push(s);
            }
        }
    }
    seen
}

/// For every `(block_id, inst_index)` position in the function, return the
/// stack of regions active there, innermost first.
pub fn membership_map<'a>(
    ir: &IrFunction,
    regions: &'a [Region],
) -> HashMap<(BlockId, usize), Vec<&'a Region>> {
    let mut map: HashMap<(BlockId, usize), Vec<&'a Region>> = HashMap::new();
    for block in &ir.blocks {
        for (idx, _inst) in block.insts.iter().enumerate() {
            let mut active: Vec<&Region> = regions
                .iter()
                .filter(|r| inst_in_region(r, block.id, idx))
                .collect();
            // Innermost first — assume regions nest by start position; sort
            // by start position descending so deepest start (latest) is first.
            active.sort_by(|a, b| {
                (b.start_block.0, b.start_inst_idx).cmp(&(a.start_block.0, a.start_inst_idx))
            });
            if !active.is_empty() {
                map.insert((block.id, idx), active);
            }
        }
    }
    map
}

fn inst_in_region(region: &Region, block_id: BlockId, inst_idx: usize) -> bool {
    if !region.blocks.contains(&block_id) {
        return false;
    }
    if region.start_block == region.end_block {
        // Single-block region: only insts strictly between Start..=End.
        return block_id == region.start_block
            && inst_idx >= region.start_inst_idx
            && inst_idx <= region.end_inst_idx;
    }
    if block_id == region.start_block {
        return inst_idx >= region.start_inst_idx;
    }
    if block_id == region.end_block {
        return inst_idx <= region.end_inst_idx;
    }
    true
}
