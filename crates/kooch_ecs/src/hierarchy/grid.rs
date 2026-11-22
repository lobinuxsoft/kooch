//! Large-world precision (#51, #1972): positions are f64, everything narrowed to f32 is relative to
//! a world origin that follows the view.
//!
//! 🔴 f32 steps 0.5 m at Earth radius. `Transform` keeps the exact f64 position; `GlobalTransform`
//! is relative to [`WorldOrigin`], which moves by whole cells when its anchor strays — the editor
//! camera while editing, the [`FloatingOrigin`] entity while playing. Nothing authored moves: a
//! rebase only republishes `GlobalTransform`, and [`OriginShift`] tells caches (solver, camera rig)
//! that hold origin-relative positions to follow.

use glam::{DVec3, I64Vec3, Mat4};
use kooch_core::resource::Resources;
use kooch_core::run_state::{Mirrored, Playing};

use crate::component::{Component, ComponentRegistry};
use crate::entity::Entity;
use crate::transform::Transform;

use super::global_transform::GlobalTransform;
use super::parent::Parent;

/// Side of a cell, in metres. A power of two, so a shift by whole cells is exact in f32.
pub const CELL: f64 = 2048.0;

/// Past this, a new root gets a [`GridCell`]: f64 still holds a micrometre out to here.
pub const FAR: f64 = 4_294_967_296.0;

/// Which cell a root sits in; its `Transform` is local to that cell, so it stays exact at any
/// distance (#1977): f64 alone loses a metre a light-year out. `rehome_cells` keeps the local
/// position inside its cell. Ignored below the root.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, crate::Reflect)]
#[reflect(category = "Core")]
pub struct GridCell {
    /// Cell along X.
    pub x: i64,
    /// Cell along Y.
    pub y: i64,
    /// Cell along Z.
    pub z: i64,
}

impl Component for GridCell {}

impl GridCell {
    /// The cell as a vector.
    pub fn cell(self) -> I64Vec3 {
        I64Vec3::new(self.x, self.y, self.z)
    }
}

/// The entity the world is kept centred on while playing, usually the player. Carries the origin's
/// cell so the editor's mirror of a remote Play receives it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, crate::Reflect)]
#[reflect(category = "Core")]
pub struct FloatingOrigin {
    /// The origin's cell along X. Written by the rebase.
    pub x: i64,
    /// The same, along Y.
    pub y: i64,
    /// The same, along Z.
    pub z: i64,
}

impl Component for FloatingOrigin {}

impl FloatingOrigin {
    /// The cell as a vector.
    pub fn cell(self) -> I64Vec3 {
        I64Vec3::new(self.x, self.y, self.z)
    }
}

/// The cell `GlobalTransform` is relative to. Zero until something anchors it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorldOrigin {
    /// The origin's cell.
    pub cell: I64Vec3,
}

impl WorldOrigin {
    /// The origin in metres, for a root without a [`GridCell`].
    pub fn offset(&self) -> DVec3 {
        self.cell.as_dvec3() * CELL
    }

    /// The origin in `resources`, as a root without a [`GridCell`] measures it; zero without one.
    pub fn of(resources: &Resources) -> DVec3 {
        Self::get(resources).offset()
    }

    /// The origin resource, zero without one.
    pub fn get(resources: &Resources) -> Self {
        resources.get::<WorldOrigin>().copied().unwrap_or_default()
    }

    /// The origin as seen from `cell`: subtracted in integers first, so it stays exact however
    /// far both are from zero. `None` is cell zero.
    pub fn from(&self, cell: Option<&GridCell>) -> DVec3 {
        let cell = cell.map_or(I64Vec3::ZERO, |c| c.cell());
        (self.cell - cell).as_dvec3() * CELL
    }

    /// The origin as root `entity`'s `Transform` measures it: through its [`GridCell`], if any.
    pub fn of_root(&self, registry: &ComponentRegistry, entity: Entity) -> DVec3 {
        self.from(registry.component::<GridCell>(entity))
    }

    /// Where a new root `rel` from the origin goes: one absolute f64 position near home, a cell
    /// and a position local to it past [`FAR`].
    pub fn place(&self, rel: glam::Vec3) -> (Option<GridCell>, DVec3) {
        let absolute = self.offset() + rel.as_dvec3();
        if !(absolute.abs().max_element() > FAR) {
            return (None, absolute);
        }
        let whole = (rel.as_dvec3() / CELL).round();
        let at = self.cell + whole.as_i64vec3();
        let cell = GridCell {
            x: at.x,
            y: at.y,
            z: at.z,
        };
        (Some(cell), rel.as_dvec3() - whole * CELL)
    }

    /// Root `entity`'s `transform` relative to the origin, in f32.
    pub fn relative(
        &self,
        registry: &ComponentRegistry,
        entity: Entity,
        transform: &Transform,
    ) -> glam::Vec3 {
        transform.relative_to(self.of_root(registry, entity))
    }

    /// The `Transform` position that puts root `entity` at `rel` from the origin.
    pub fn local(&self, registry: &ComponentRegistry, entity: Entity, rel: glam::Vec3) -> DVec3 {
        self.of_root(registry, entity) + rel.as_dvec3()
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
    pub cells: I64Vec3,
}

impl OriginShift {
    /// Whether the origin moved this frame.
    pub fn moved(&self) -> bool {
        self.cells != I64Vec3::ZERO
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
pub(super) fn root_matrix(local: &Transform, cell: Option<I64Vec3>, origin: I64Vec3) -> Mat4 {
    let from = (origin - cell.unwrap_or(I64Vec3::ZERO)).as_dvec3() * CELL;
    local.matrix_from(from)
}

/// A fingerprint of the origin and the grid cells' revision: a change forces a full propagation
/// pass, which the per-entity pose diff cannot see. O(1): planet blocks carry cells by thousands.
pub(super) fn anchors(registry: &ComponentRegistry, origin: I64Vec3) -> u64 {
    let revision = registry
        .get_cpu::<GridCell>()
        .map_or(0, |cells| cells.revision());
    mix(mix(0, origin), I64Vec3::new(revision as i64, 0, 0))
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
        None if mirrored => first_origin(registry).map_or(I64Vec3::ZERO, |(_, o)| o.cell()),
        // Nothing to follow: a scene without a FloatingOrigin, or the game after Stop.
        None => I64Vec3::ZERO,
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

/// Moves whole cells out of a playing root's `Transform` into its [`GridCell`] once it strays past
/// one, so its f64 stays small. The sum is unchanged, so nothing it draws moves. A mirror leaves it
/// to its source; an edit leaves the author's numbers alone.
pub fn rehome_cells(resources: &mut Resources) {
    if Mirrored::is_mirrored(resources) || !Playing::is_playing(resources) {
        return;
    }
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    let moves: Vec<(Entity, I64Vec3)> = {
        let (Some(cells), Some(transforms)) = (
            registry.get_cpu::<GridCell>(),
            registry.get_cpu::<Transform>(),
        ) else {
            return;
        };
        cells
            .iter()
            .filter(|(entity, _)| registry.component::<Parent>(**entity).is_none())
            .filter_map(|(entity, _)| {
                let at = transforms.get(*entity)?.position;
                (at.abs().max_element() > CELL).then(|| (*entity, (at / CELL).round().as_i64vec3()))
            })
            .collect()
    };
    if moves.is_empty() {
        return;
    }
    if let Some(mut transforms) = registry.get_cpu_mut::<Transform>() {
        for (entity, by) in &moves {
            if let Some(transform) = transforms.get_mut(*entity) {
                transform.position -= by.as_dvec3() * CELL;
            }
        }
    }
    if let Some(mut cells) = registry.get_cpu_mut::<GridCell>() {
        for (entity, by) in &moves {
            if let Some(cell) = cells.get_mut(*entity) {
                (cell.x, cell.y, cell.z) = (cell.x + by.x, cell.y + by.y, cell.z + by.z);
            }
        }
    }
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
fn publish(resources: &mut Resources, now: I64Vec3) {
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
fn straying(registry: &ComponentRegistry, entity: Entity) -> Option<I64Vec3> {
    let global = registry.get_cpu::<GlobalTransform>()?;
    let at = global.get(entity)?.matrix.w_axis.truncate().as_dvec3();
    if !(at.abs().max_element() > CELL) {
        return None;
    }
    Some((at / CELL).round().as_i64vec3())
}

#[cfg(test)]
mod tests;
