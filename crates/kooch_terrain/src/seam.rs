//! Who holds the cells just past a block (#313 step 4c). Surface nets reaches one cell beyond its
//! block, so per each of its 26 neighbour directions a block needs only the leaf there: one of its
//! own level, a coarser one whose tile it borrows, or finer ones that stitch the seam themselves.

use std::collections::HashSet;

use bytemuck::{Pod, Zeroable};
use glam::IVec3;

use super::generate::Backlog;
use super::lod::root_level;
use super::{BlockKey, BlockPool, Planets};

#[cfg(test)]
mod tests;

/// The direction's cells are the block's own: a neighbour of its level shares them, and no leaf
/// there means no surface either.
pub const OWN: u32 = 0;

/// Finer leaves lie across the direction, and they emit the seam.
pub const FINER: u32 = 1 << 31;

/// A leaf of the block's level lies across, in this slot. Set only on directions past a + plane of
/// a block with a coarser neighbour: where three blocks of a level meet a coarser one, the block
/// that emits the edge between them reads the other two's cells past its tile (#393).
pub const SAME: u32 = 1 << 30;

const SLOT_BITS: u32 = 24;
const SLOT_MASK: u32 = (1 << SLOT_BITS) - 1;

/// The pool's leaves, and every block holding one.
#[derive(Clone, Copy)]
pub struct Leaves<'a> {
    pub pool: &'a BlockPool,
    pub split: &'a HashSet<BlockKey>,
}

/// A block's 26 neighbours, as the mesh passes read them. Mirrors `Near` in `nets.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct Near {
    /// The block's cell, which aligns its samples with a coarser neighbour's.
    pub cell: [i32; 3],
    /// Per [`direction`]: [`OWN`], [`FINER`], [`SAME`] with a slot, or a coarser leaf's slot with
    /// the levels up to it.
    pub sides: [u32; 26],
}

impl Near {
    /// Whether every leaf it reads past its tile, coarser or [`SAME`], still holds its slot, so the
    /// write meshes what the count counted. A slot that changed hands makes the write skip.
    pub fn holds(&self, pool: &BlockPool, key: BlockKey) -> bool {
        self.sides.iter().enumerate().all(|(d, &side)| {
            let across = BlockKey {
                cell: key.cell + direction(d),
                ..key
            };
            if side != FINER && side & SAME != 0 {
                return pool.key(side & SLOT_MASK) == Some(across);
            }
            let up = levels(side);
            if up == 0 {
                return true;
            }
            let leaf = std::iter::successors(Some(across), |k| Some(k.parent())).nth(up as usize);
            leaf.is_some_and(|leaf| pool.key(side & SLOT_MASK) == Some(leaf))
        })
    }
}

/// The offset of direction `d` in `0..26`: x fastest, the centre left out.
pub fn direction(d: usize) -> IVec3 {
    let d = d as i32 + i32::from(d >= 13);
    IVec3::new(d % 3, (d / 3) % 3, d / 9) - IVec3::ONE
}

/// Levels up to the coarser leaf a side borrows from; zero when it borrows none.
pub fn levels(side: u32) -> u32 {
    (side >> SLOT_BITS) & 0x3f
}

/// Meshed blocks whose neighbours changed since they were counted (#313 step 4c piece 5): a leaf
/// that appeared, left or finished generating. A sweep finds them while anything changes, so a
/// still pool costs nothing; they mesh again from the tile they hold, never generated again.
#[derive(Default)]
pub(crate) struct Stale {
    /// Per slot, the neighbours its mesh was counted with.
    meshed: Vec<Near>,
    queue: Backlog,
    cursor: u32,
    left: u32,
}

impl Stale {
    pub fn counted(&mut self, slot: u32, near: Near) {
        let i = slot as usize;
        if self.meshed.len() <= i {
            self.meshed.resize(i + 1, Near::default());
        }
        self.meshed[i] = near;
    }

    /// Something changed: every slot is checked once more.
    pub fn touch(&mut self, pool: &BlockPool) {
        self.left = pool.capacity();
    }

    /// Queues the slot to mesh again.
    pub fn queue(&mut self, slot: u32) {
        self.queue.retry(slot);
    }

    pub fn pop(&mut self) -> Option<u32> {
        self.queue.pop()
    }

    pub fn holds(&self, slot: u32) -> bool {
        self.queue.holds(slot)
    }

    pub fn slots(&self) -> impl Iterator<Item = u32> + '_ {
        self.queue.slots()
    }

    /// Checks up to `budget` slots, putting in `found` those whose neighbours differ from their
    /// mesh's. The caller queues them.
    pub fn sweep(
        &mut self,
        leaves: Leaves,
        planets: &Planets,
        ready: impl Fn(u32) -> bool,
        budget: u32,
        found: &mut Vec<u32>,
    ) {
        let capacity = leaves.pool.capacity();
        if capacity == 0 {
            return;
        }
        for _ in 0..budget.min(self.left) {
            let slot = self.cursor;
            self.cursor = (self.cursor + 1) % capacity;
            self.left -= 1;
            // A slot not generated yet meshes when it is, with the neighbours of then.
            let Some(key) = leaves.pool.key(slot).filter(|_| ready(slot)) else {
                continue;
            };
            let Some(was) = self.meshed.get(slot as usize) else {
                continue;
            };
            let root = planets.shape.get(key.planet as usize).map_or(0, root_level);
            if near(leaves, root, key, &ready) != *was {
                found.push(slot);
            }
        }
    }
}

/// Every resident block whose seam reads `key`'s cells: within one cell of it, at its level or one
/// either side, which the 2:1 balance makes every level that can touch it.
pub(crate) fn touching(pool: &BlockPool, root: u8, key: BlockKey) -> Vec<u32> {
    let mut found = Vec::new();
    let mut around = |level: u8, lo: IVec3, hi: IVec3| {
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let at = BlockKey {
                        level,
                        cell: IVec3::new(x, y, z),
                        ..key
                    };
                    if at != key
                        && let Some(slot) = pool.slot(at)
                    {
                        found.push(slot);
                    }
                }
            }
        }
    };
    around(key.level, key.cell - IVec3::ONE, key.cell + IVec3::ONE);
    if key.level > 0 {
        let first = key.cell * 2;
        around(key.level - 1, first - IVec3::ONE, first + IVec3::splat(2));
    }
    if key.level < root {
        let up = key.parent().cell;
        around(key.level + 1, up - IVec3::ONE, up + IVec3::ONE);
    }
    found
}

/// `key`'s neighbours among `leaves`. A coarser leaf whose tile is not `ready` counts as
/// [`OWN`] until it is: a crack for a frame, never a mesh read from a stale tile.
pub(crate) fn near(leaves: Leaves, root: u8, key: BlockKey, ready: impl Fn(u32) -> bool) -> Near {
    let mut sides = [OWN; 26];
    for (d, side) in sides.iter_mut().enumerate() {
        let across = BlockKey {
            cell: key.cell + direction(d),
            ..key
        };
        if leaves.pool.slot(across).is_some() {
            continue;
        }
        if leaves.split.contains(&across) {
            *side = FINER;
            continue;
        }
        // Leaves never overlap: past a block holding one, no ancestor can be a leaf.
        let held = across
            .ancestors(root)
            .take_while(|up| !leaves.split.contains(up))
            .find_map(|up| Some((up.level, leaves.pool.slot(up)?)));
        if let Some((level, slot)) = held
            && slot <= SLOT_MASK
            && ready(slot)
        {
            *side = slot | (u32::from(level - key.level) << SLOT_BITS);
        }
    }
    if sides.iter().any(|&side| levels(side) != 0) {
        for (d, side) in sides.iter_mut().enumerate() {
            let across = BlockKey {
                cell: key.cell + direction(d),
                ..key
            };
            if *side == OWN
                && direction(d).max_element() == 1
                && let Some(slot) = leaves.pool.slot(across)
                && slot <= SLOT_MASK
                && ready(slot)
            {
                *side = SAME | slot;
            }
        }
    }
    Near {
        cell: key.cell.to_array(),
        sides,
    }
}
