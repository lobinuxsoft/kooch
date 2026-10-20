//! Which archetype slot holds each entity, indexed by [`Entity::index`].

use crate::archetype::ArchetypeId;
use crate::entity::Entity;

use super::registry::Where;

/// One entity's filing, and the generation that owns it.
#[derive(Debug, Clone, Copy)]
struct Slot {
    generation: u32,
    at: Where,
}

/// Entity → where it is filed.
///
/// 🔴 A `Vec`, not a map, for the same reason the values use one: the index is dense, so a hash
/// costs a cache miss and buys nothing. The generation travels with it because an index is reused.
#[derive(Debug, Default)]
pub struct Filed {
    slots: Vec<Option<Slot>>,
}

impl Filed {
    /// Where `entity` is filed.
    #[inline]
    pub fn get(&self, entity: Entity) -> Option<Where> {
        let slot = (*self.slots.get(entity.index() as usize)?)?;
        (slot.generation == entity.generation()).then_some(slot.at)
    }

    /// Records where `entity` is filed.
    pub fn set(&mut self, entity: Entity, archetype: ArchetypeId, at: u32) {
        // 🔴 The sentinel's index is `u32::MAX`. Resizing to it would ask for sixty gigabytes.
        if !entity.is_valid() {
            return;
        }
        let index = entity.index() as usize;
        if index >= self.slots.len() {
            self.slots.resize(index + 1, None);
        }
        self.slots[index] = Some(Slot {
            generation: entity.generation(),
            at: Where { archetype, at },
        });
    }

    /// Forgets `entity`, returning where it was filed.
    pub fn take(&mut self, entity: Entity) -> Option<Where> {
        let at = self.get(entity)?;
        self.slots[entity.index() as usize] = None;
        Some(at)
    }

    /// Forgets every entity.
    pub fn clear(&mut self) {
        self.slots.clear();
    }
}
