//! Stable planet indices: a planet keeps its `BlockKey::planet` while it lives, so the pool keeps
//! its blocks across frames, and an entity replaced by an identical one keeps them too.

use kooch_core::Guid;
use kooch_ecs::Entity;

use crate::component::Planet;

/// What a planet's blocks are generated from. Two planets with the same one have the same blocks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Signature {
    shape: [u32; 4],
    generator: Option<Guid>,
    /// The generator's reload count, so an edited graph regenerates.
    revision: u64,
    /// Whether the generator's source was in hand, so it regenerates once it loads.
    loaded: bool,
}

impl Signature {
    pub(crate) fn new(planet: &Planet, revision: u64, loaded: bool) -> Self {
        Self {
            shape: [planet.radius, planet.relief, planet.depth, planet.voxel].map(f32::to_bits),
            generator: planet.generator,
            revision,
            loaded,
        }
    }
}

/// Planet indices as parallel arrays; a freed index keeps its signature until it is reused.
#[derive(Default)]
pub(crate) struct Roster {
    entity: Vec<Option<Entity>>,
    signature: Vec<Option<Signature>>,
}

impl Roster {
    pub(crate) fn len(&self) -> usize {
        self.entity.len()
    }

    /// Frees every index whose entity is not in `alive`.
    pub(crate) fn retain(&mut self, alive: &[Entity]) {
        for held in &mut self.entity {
            if held.is_some_and(|entity| !alive.contains(&entity)) {
                *held = None;
            }
        }
    }

    /// The index of `entity`, and whether its blocks must be generated again. A new entity takes
    /// the free index that already holds its signature first: what Play does to every planet.
    pub(crate) fn place(&mut self, entity: Entity, signature: Signature) -> (u32, bool) {
        let index = self
            .entity
            .iter()
            .position(|held| *held == Some(entity))
            .or_else(|| {
                (0..self.len())
                    .find(|&i| self.entity[i].is_none() && self.signature[i] == Some(signature))
            })
            .or_else(|| self.entity.iter().position(Option::is_none))
            .unwrap_or_else(|| {
                self.entity.push(None);
                self.signature.push(None);
                self.len() - 1
            });
        self.entity[index] = Some(entity);
        let changed = self.signature[index] != Some(signature);
        self.signature[index] = Some(signature);
        (index as u32, changed)
    }
}
