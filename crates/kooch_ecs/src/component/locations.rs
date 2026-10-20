//! Where each entity's component values live (#891).

use crate::entity::Entity;
use crate::storage::{TableId, TableRow};

/// One entity's row, and the generation that owns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Placed {
    generation: u32,
    table: TableId,
    row: TableRow,
}

/// Entity → the table row holding its values, indexed by [`Entity::index`].
///
/// 🔴 A `Vec`, not a map. The index is already dense — the allocator hands out the lowest free slot
/// — so a hash buys nothing and costs a cache miss on every single-entity read. The generation
/// travels with the row because an index is reused: without it a fresh entity would inherit the
/// row of whoever held that slot before, and read its components.
#[derive(Debug, Default)]
pub struct Locations {
    slots: Vec<Option<Placed>>,
}

impl Locations {
    /// Where `entity`'s values live, if they live in a table.
    #[inline]
    pub fn get(&self, entity: Entity) -> Option<(TableId, TableRow)> {
        let placed = (*self.slots.get(entity.index() as usize)?)?;
        (placed.generation == entity.generation()).then_some((placed.table, placed.row))
    }

    /// Records where `entity`'s values live.
    pub fn set(&mut self, entity: Entity, table: TableId, row: TableRow) {
        // 🔴 The sentinel's index is `u32::MAX`. Resizing to it would ask for sixty gigabytes.
        if !entity.is_valid() {
            return;
        }
        let slot = entity.index() as usize;
        if slot >= self.slots.len() {
            self.slots.resize(slot + 1, None);
        }
        self.slots[slot] = Some(Placed {
            generation: entity.generation(),
            table,
            row,
        });
    }

    /// Forgets where `entity`'s values live, returning where that was.
    pub fn take(&mut self, entity: Entity) -> Option<(TableId, TableRow)> {
        let at = self.get(entity)?;
        self.slots[entity.index() as usize] = None;
        Some(at)
    }
}
