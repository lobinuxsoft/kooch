//! Large-world precision (#51): a floating origin for what moves, an integer cell for what is far.
//!
//! 🔴 f32 steps 0.5 m at Earth radius. Everything the frame computes stays near the origin: when the
//! [`FloatingOrigin`] entity strays a cell away, every root without a [`GridCell`] is shifted by whole
//! cells at once and [`OriginShift`] tells the caches (solver, camera rig) to follow. Whole cells keep
//! the shift exact in f32, so a rebase never moves anything relative to anything else.

use std::collections::HashSet;

use glam::{I64Vec3, IVec3, Mat4, Vec3};
use kooch_core::resource::Resources;
use kooch_core::run_state::{Mirrored, Playing};

use crate::component::{Component, ComponentRegistry};
use crate::entity::Entity;
use crate::transform::Transform;

use super::global_transform::GlobalTransform;
use super::parent::Parent;

/// Side of a cell, in metres. A power of two, so a shift by whole cells is exact in f32.
pub const CELL: f64 = 2048.0;

/// Which cell a root sits in; its `Transform` is local to that cell. For what the origin must not
/// drag along: a planet, a far station. Ignored below the root — a child is local to its parent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, crate::Reflect)]
#[reflect(category = "Core")]
pub struct GridCell {
    /// Cell along X.
    pub x: i32,
    /// Cell along Y.
    pub y: i32,
    /// Cell along Z.
    pub z: i32,
}

impl Component for GridCell {}

impl GridCell {
    /// The cell as a vector.
    pub fn cell(self) -> IVec3 {
        IVec3::new(self.x, self.y, self.z)
    }
}

/// The entity the world is kept centred on, usually the player. Carries the origin's cell as a
/// component rather than a resource so the editor's mirror of a remote Play receives it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, crate::Reflect)]
#[reflect(category = "Core")]
pub struct FloatingOrigin {
    /// The origin's cell along X. Written by the rebase.
    pub x: i32,
    /// The same, along Y.
    pub y: i32,
    /// The same, along Z.
    pub z: i32,
}

impl Component for FloatingOrigin {}

impl FloatingOrigin {
    /// The cell as a vector.
    pub fn cell(self) -> IVec3 {
        IVec3::new(self.x, self.y, self.z)
    }
}

/// Whole cells the origin moved this frame; zero on every frame it did not. Anything caching a world
/// position across frames adds [`OriginShift::offset`] to it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OriginShift {
    /// Cells moved, in the origin's direction.
    pub cells: IVec3,
}

impl OriginShift {
    /// Whether the origin moved this frame.
    pub fn moved(&self) -> bool {
        self.cells != IVec3::ZERO
    }

    /// What every world position gains: the origin moved one way, the world the other.
    pub fn offset(&self) -> Vec3 {
        (-self.cells.as_dvec3() * CELL).as_vec3()
    }

    /// This frame's shift, `None` when the origin stayed.
    pub fn of(resources: &Resources) -> Option<Vec3> {
        resources
            .get::<OriginShift>()
            .filter(|shift| shift.moved())
            .map(OriginShift::offset)
    }
}

/// The origin's cell: the first [`FloatingOrigin`]'s, or zero in a world without one.
pub fn origin_cell(registry: &ComponentRegistry) -> IVec3 {
    registry
        .get_cpu::<FloatingOrigin>()
        .and_then(|storage| storage.iter().next().map(|(_, origin)| origin.cell()))
        .unwrap_or(IVec3::ZERO)
}

/// Where a gridded root's local position sits relative to the origin. In f64 so a far cell loses
/// nothing before the difference makes it near.
pub fn anchored(cell: IVec3, origin: IVec3, local: Vec3) -> Vec3 {
    let cells = cell.as_i64vec3() - origin.as_i64vec3();
    (cells.as_dvec3() * CELL + local.as_dvec3()).as_vec3()
}

/// A root's world matrix: its local one, carried from its cell to the origin's when it has one.
pub(super) fn root_matrix(local: &Transform, cell: Option<IVec3>, origin: IVec3) -> Mat4 {
    match cell {
        Some(cell) => Transform {
            position: anchored(cell, origin, local.position),
            ..*local
        }
        .to_matrix(),
        None => local.to_matrix(),
    }
}

/// A fingerprint of the origin and every grid cell: a change forces a full propagation pass, which
/// the per-entity pose diff cannot see.
pub(super) fn anchors(registry: &ComponentRegistry) -> u64 {
    let mut hash = mix(0, origin_cell(registry).as_i64vec3());
    if let Some(cells) = registry.get_cpu::<GridCell>() {
        for (entity, cell) in cells.iter() {
            hash = mix(
                hash,
                I64Vec3::new(entity.index() as i64, entity.generation() as i64, 0),
            );
            hash = mix(hash, cell.cell().as_i64vec3());
        }
    }
    hash
}

fn mix(hash: u64, value: I64Vec3) -> u64 {
    [value.x, value.y, value.z].iter().fold(hash, |h, &v| {
        (h ^ v as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Recentres the world on the [`FloatingOrigin`] once it strays past a cell, then republishes the
/// hierarchy so everything this frame reads the shifted world. Gameplay only, and never in a mirror:
/// the authored scene must not move while it is edited.
pub fn rebase_origin(resources: &mut Resources) {
    resources.insert(OriginShift::default());
    if !Playing::is_playing(resources) || Mirrored::is_mirrored(resources) {
        return;
    }
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    let Some(cells) = straying(registry) else {
        return;
    };
    let offset = OriginShift { cells }.offset();
    shift_roots(registry, offset);
    if let Some(mut origins) = registry.get_cpu_mut::<FloatingOrigin>()
        && let Some((_, mut origin)) = origins.iter_mut().next()
    {
        let cell = origin.cell() + cells;
        (origin.x, origin.y, origin.z) = (cell.x, cell.y, cell.z);
    }
    resources.insert(OriginShift { cells });
    super::transform_propagation_system(resources);
}

/// Whole cells the origin entity has strayed, `None` inside one cell. Past a whole cell rather than
/// half, so the entity lands within half a cell and a step back does not rebase again.
fn straying(registry: &ComponentRegistry) -> Option<IVec3> {
    let entity: Entity = registry
        .get_cpu::<FloatingOrigin>()?
        .iter()
        .next()
        .map(|(entity, _)| *entity)?;
    let at = registry
        .get_cpu::<GlobalTransform>()?
        .get(entity)?
        .matrix
        .w_axis
        .truncate();
    let at = at.as_dvec3();
    if !(at.abs().max_element() > CELL) {
        return None;
    }
    Some((at / CELL).round().as_ivec3())
}

/// Moves every root that is not anchored to a cell. Children ride their parents.
fn shift_roots(registry: &mut ComponentRegistry, offset: Vec3) {
    let still: HashSet<Entity> = {
        let parents = registry.get_cpu::<Parent>();
        let cells = registry.get_cpu::<GridCell>();
        let Some(transforms) = registry.get_cpu::<Transform>() else {
            return;
        };
        transforms
            .iter()
            .map(|(entity, _)| *entity)
            .filter(|&entity| {
                let parented = parents.as_ref().is_some_and(|p| p.get(entity).is_some());
                let anchored = cells.as_ref().is_some_and(|c| c.get(entity).is_some());
                parented || anchored
            })
            .collect()
    };
    let Some(mut transforms) = registry.get_cpu_mut::<Transform>() else {
        return;
    };
    for (entity, mut transform) in transforms.iter_mut() {
        if !still.contains(entity) {
            transform.position += offset;
        }
    }
}

#[cfg(test)]
mod tests;
