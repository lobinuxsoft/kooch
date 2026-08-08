//! Entity identifier for the ECS.
//!
//! Each entity has a `u32` index (used as GPU buffer offset) and a `u32`
//! generation counter that guards against the ABA problem when slots are
//! recycled.

use std::fmt;

/// A lightweight handle that identifies an entity.
///
/// - **CPU side** — both `index` and `generation` are used to detect stale
///   references after a slot has been recycled.
/// - **GPU side** — only `index` is sent as a plain `u32` buffer offset
///   via [`Entity::to_gpu`].
///
/// Use [`Entity::INVALID`] as a sentinel for uninitialised slots or
/// `Option<Entity>` where the absence of an entity has semantic meaning.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Entity {
    index: u32,
    generation: u32,
}

/// [`Entity::INVALID`] — the sentinel that already means "points at
/// nothing".
///
/// Needed because `#[derive(Reflect)]` requires [`Default`], so without
/// this no component holding an `Entity` field could use the derive at
/// all. `INVALID` is the only defensible default: it cannot collide with
/// an allocated entity, and [`Entity::is_valid`] already exists to check
/// for it.
impl Default for Entity {
    fn default() -> Self {
        Self::INVALID
    }
}

impl Entity {
    /// Sentinel value for uninitialised / empty slots.
    ///
    /// Uses `u32::MAX` as the index so it can never collide with a valid
    /// entity spawned by the allocator.
    pub const INVALID: Self = Self {
        index: u32::MAX,
        generation: 0,
    };

    /// Creates a new entity handle.
    #[inline]
    pub const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    /// Returns the slot index (also the GPU buffer offset).
    #[inline]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// Returns the generation counter.
    #[inline]
    pub const fn generation(&self) -> u32 {
        self.generation
    }

    /// Returns the index as a plain `u32` suitable for GPU storage buffers.
    #[inline]
    pub const fn to_gpu(&self) -> u32 {
        self.index
    }

    /// Returns `true` if this entity is not the [`INVALID`](Self::INVALID) sentinel.
    #[inline]
    pub const fn is_valid(&self) -> bool {
        self.index != u32::MAX
    }
}

impl fmt::Display for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Entity(index={}, gen={})", self.index, self.generation)
    }
}

#[cfg(test)]
mod tests;
