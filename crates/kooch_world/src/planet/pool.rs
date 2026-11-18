//! The shared block pool: slots as parallel arrays, handed out by a free-list.

use std::collections::HashMap;

use glam::IVec3;

use super::BlockKey;

/// Which slots changed in one [`BlockPool::apply`]: `added` need generating, `removed` are free.
#[derive(Clone, Debug, Default)]
pub struct Residency {
    pub added: Vec<u32>,
    pub removed: Vec<u32>,
}

/// A fixed number of block slots shared by every planet. The key → slot map is CPU-side
/// streaming bookkeeping; the GPU only ever sees slot indices.
pub struct BlockPool {
    planet: Vec<u32>,
    level: Vec<u8>,
    cell: Vec<IVec3>,
    /// The `apply` call that last wanted each slot.
    stamp: Vec<u32>,
    frame: u32,
    free: Vec<u32>,
    slots: HashMap<BlockKey, u32>,
}

impl BlockPool {
    pub fn new(capacity: u32) -> Self {
        let len = capacity as usize;
        Self {
            planet: vec![0; len],
            level: vec![0; len],
            cell: vec![IVec3::ZERO; len],
            stamp: vec![0; len],
            frame: 0,
            free: (0..capacity).rev().collect(),
            slots: HashMap::with_capacity(len),
        }
    }

    pub fn capacity(&self) -> u32 {
        self.stamp.len() as u32
    }

    pub fn resident(&self) -> u32 {
        self.slots.len() as u32
    }

    pub fn slot(&self, key: BlockKey) -> Option<u32> {
        self.slots.get(&key).copied()
    }

    /// The block a resident slot holds.
    pub fn key(&self, slot: u32) -> Option<BlockKey> {
        let i = slot as usize;
        let key = BlockKey {
            planet: *self.planet.get(i)?,
            level: *self.level.get(i)?,
            cell: *self.cell.get(i)?,
        };
        (self.slots.get(&key) == Some(&slot)).then_some(key)
    }

    /// Makes `wanted` resident and frees every other slot. Frees before it allocates, so a full
    /// pool can swap its blocks; a key beyond the capacity is left out.
    pub fn apply(&mut self, wanted: &[BlockKey], out: &mut Residency) {
        out.added.clear();
        out.removed.clear();
        self.frame = self.frame.wrapping_add(1);
        let frame = self.frame;
        for key in wanted {
            if let Some(&slot) = self.slots.get(key) {
                self.stamp[slot as usize] = frame;
            }
        }
        let (stamp, free) = (&self.stamp, &mut self.free);
        self.slots.retain(|_, slot| {
            let keep = stamp[*slot as usize] == frame;
            if !keep {
                free.push(*slot);
                out.removed.push(*slot);
            }
            keep
        });
        for &key in wanted {
            if self.slots.contains_key(&key) {
                continue;
            }
            let Some(slot) = self.free.pop() else {
                break;
            };
            let i = slot as usize;
            self.planet[i] = key.planet;
            self.level[i] = key.level;
            self.cell[i] = key.cell;
            self.stamp[i] = frame;
            self.slots.insert(key, slot);
            out.added.push(slot);
        }
    }
}
