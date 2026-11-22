//! Large-world precision (#51, #1972): positions are f64, everything narrowed to f32 is relative to
//! a world origin that follows the view.
//!
//! 🔴 f32 steps 0.5 m at Earth radius. `Transform` keeps the exact f64 position; `GlobalTransform`
//! is relative to [`WorldOrigin`], which moves by whole cells when its anchor strays — the editor
//! camera while editing, the [`FloatingOrigin`] entity while playing. Nothing authored moves: a
//! rebase only republishes `GlobalTransform`, and [`OriginShift`] tells caches (solver, camera rig)
//! that hold origin-relative positions to follow.

use glam::{DVec3, I64Vec3, IVec3, Mat4};
use kooch_core::resource::Resources;
use kooch_core::run_state::{Mirrored, Playing};

use crate::component::{Component, ComponentRegistry};
use crate::entity::Entity;
use crate::transform::Transform;

use super::global_transform::GlobalTransform;

/// Side of a cell, in metres. A power of two, so a shift by whole cells is exact in f32.
pub const CELL: f64 = 2048.0;

/// Which cell a root sits in; its `Transform` is local to that cell. Reserved for sector-scale
/// worlds (#1977): f64 alone already holds a solar system. Ignored below the root.
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

/// The entity the world is kept centred on while playing, usually the player. Carries the origin's
/// cell so the editor's mirror of a remote Play receives it.
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

/// The cell `GlobalTransform` is relative to. Zero until something anchors it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorldOrigin {
    /// The origin's cell.
    pub cell: IVec3,
}

impl WorldOrigin {
    /// The origin in metres.
    pub fn offset(&self) -> DVec3 {
        self.cell.as_dvec3() * CELL
    }

    /// The origin in `resources`, zero without one.
    pub fn of(resources: &Resources) -> DVec3 {
        resources
            .get::<WorldOrigin>()
            .map_or(DVec3::ZERO, WorldOrigin::offset)
    }
}

/// What the origin follows while nothing plays: the editor's camera. A game leaves it empty.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OriginAnchor(pub Option<Entity>);

/// Whole cells the origin moved this frame; zero on every frame it did not. Anything caching an
/// origin-relative position across frames adds [`OriginShift::offset`] to it.
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

    /// What every origin-relative position gains: the origin moved one way, the world the other.
    pub fn offset(&self) -> glam::Vec3 {
        (-self.cells.as_dvec3() * CELL).as_vec3()
    }

    /// This frame's shift, `None` when the origin stayed.
    pub fn of(resources: &Resources) -> Option<glam::Vec3> {
        resources
            .get::<OriginShift>()
            .filter(|shift| shift.moved())
            .map(OriginShift::offset)
    }
}

/// A root's matrix relative to the origin: its own position, or its cell plus a local one.
pub(super) fn root_matrix(local: &Transform, cell: Option<IVec3>, origin: IVec3) -> Mat4 {
    let from = match cell {
        Some(cell) => (origin.as_i64vec3() - cell.as_i64vec3()).as_dvec3() * CELL,
        None => origin.as_dvec3() * CELL,
    };
    local.matrix_from(from)
}

/// A fingerprint of the origin and every grid cell: a change forces a full propagation pass, which
/// the per-entity pose diff cannot see.
pub(super) fn anchors(registry: &ComponentRegistry, origin: IVec3) -> u64 {
    let mut hash = mix(0, origin.as_i64vec3());
    if let Some(cells) = registry.get_cpu::<GridCell>() {
        for (entity, cell) in cells.iter() {
            let id = I64Vec3::new(entity.index() as i64, entity.generation() as i64, 0);
            hash = mix(hash, id);
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

/// Recentres the origin on its anchor once it strays past a cell, then republishes the hierarchy so
/// everything this frame reads it. A mirror takes the origin its source published instead.
pub fn rebase_origin(resources: &mut Resources) {
    resources.insert(OriginShift::default());
    let was = resources.get::<WorldOrigin>().copied().unwrap_or_default();
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return;
    };
    let mirrored = Mirrored::is_mirrored(resources);
    let now = match anchor(resources, registry, mirrored) {
        Some(entity) => was.cell + straying(registry, entity).unwrap_or_default(),
        None if mirrored => first_origin(registry).map_or(IVec3::ZERO, |(_, o)| o.cell()),
        // Nothing to follow: a scene without a FloatingOrigin, or the game after Stop.
        None => IVec3::ZERO,
    };
    // Every frame, not only on a move: Stop's restore writes the authored cell back over it.
    if !mirrored && Playing::is_playing(resources) {
        publish(resources, now);
    }
    if now == was.cell {
        return;
    }
    resources.insert(WorldOrigin { cell: now });
    resources.insert(OriginShift {
        cells: now - was.cell,
    });
    super::transform_propagation_system(resources);
}

/// The entity the origin follows: the player while playing, the editor camera otherwise. A mirror
/// without an editor anchor (the source is playing) takes the cell its source publishes.
fn anchor(resources: &Resources, registry: &ComponentRegistry, mirrored: bool) -> Option<Entity> {
    match !mirrored && Playing::is_playing(resources) {
        true => first_origin(registry).map(|(entity, _)| entity),
        false => resources.get::<OriginAnchor>().and_then(|anchor| anchor.0),
    }
}

/// Writes the origin's cell onto the `FloatingOrigin`, so a mirror of this world can follow it.
fn publish(resources: &mut Resources, now: IVec3) {
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    let Some(mut origins) = registry.get_cpu_mut::<FloatingOrigin>() else {
        return;
    };
    if let Some((_, mut origin)) = origins.iter_mut().next()
        && origin.cell() != now
    {
        (origin.x, origin.y, origin.z) = (now.x, now.y, now.z);
    }
}

fn first_origin(registry: &ComponentRegistry) -> Option<(Entity, FloatingOrigin)> {
    let origins = registry.get_cpu::<FloatingOrigin>()?;
    origins
        .iter()
        .next()
        .map(|(entity, origin)| (*entity, *origin))
}

/// Whole cells the anchor has strayed from the origin, `None` inside one cell. Past a whole cell
/// rather than half, so it lands within half a cell and a step back does not rebase again.
fn straying(registry: &ComponentRegistry, entity: Entity) -> Option<IVec3> {
    let global = registry.get_cpu::<GlobalTransform>()?;
    let at = global.get(entity)?.matrix.w_axis.truncate().as_dvec3();
    if !(at.abs().max_element() > CELL) {
        return None;
    }
    Some((at / CELL).round().as_ivec3())
}

#[cfg(test)]
mod tests;
