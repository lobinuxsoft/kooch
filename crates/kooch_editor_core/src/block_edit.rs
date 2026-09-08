//! Selecting parts of a block, rather than the whole entity.

use glam::Vec2;
use kooch_blockmesh::{Block, BlockMesh, BuiltBlocks};
use kooch_core::assets::Assets;
use kooch_core::resource::Resources;
use kooch_ecs::GlobalTransform;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;

/// What a click in the viewport is aiming at.
///
/// A second axis beside `HandleMode`, not a fourth value of it: you
/// translate *a face*, so "face" and "translate" are answers to
/// different questions and a single enum would have to spell out every
/// pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ElementMode {
    /// Clicks select entities, and the handles move them. What every
    /// other kind of entity wants.
    #[default]
    Object,
    /// Clicks select faces of the selected block.
    Face,
}

impl ElementMode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Object => "Object",
            Self::Face => "Face",
        }
    }
}

/// Which faces of which block are selected.
///
/// One entity at a time. Editing elements across several blocks is a
/// different feature and mostly a different UI — the handle would have
/// to straddle two transforms.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct BlockSelection {
    pub(crate) entity: Option<Entity>,
    pub(crate) faces: Vec<u32>,
}

impl BlockSelection {
    /// Replaces the selection with one face.
    pub(crate) fn only(&mut self, entity: Entity, face: u32) {
        self.entity = Some(entity);
        self.faces.clear();
        self.faces.push(face);
    }

    /// Adds a face, or removes it when it was already selected.
    ///
    /// Switching entity clears rather than merges: the faces held are
    /// indices into one mesh, and keeping another block's would address
    /// faces that do not exist.
    pub(crate) fn toggle(&mut self, entity: Entity, face: u32) {
        if self.entity != Some(entity) {
            self.only(entity, face);
            return;
        }
        match self.faces.iter().position(|held| *held == face) {
            Some(at) => {
                self.faces.remove(at);
            }
            None => self.faces.push(face),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.entity = None;
        self.faces.clear();
    }

    pub(crate) fn holds(&self, entity: Entity, face: u32) -> bool {
        self.entity == Some(entity) && self.faces.contains(&face)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }
}

/// Applies a click in face mode.
///
/// Pulled out of the click handler because that one reads a camera, a
/// component registry and a transform, and the decision it makes is
/// four lines that none of those affect. The version living inside it
/// went unreached for a whole session — the handler asked `Resources`
/// for an overlay the caller was already holding by reference — and
/// nothing could have caught that, but this can catch the rest.
pub(crate) fn apply_click(
    selection: &mut BlockSelection,
    entity: Entity,
    face: Option<u32>,
    ctrl_held: bool,
) {
    match (face, ctrl_held) {
        (Some(face), true) => selection.toggle(entity, face),
        (Some(face), false) => selection.only(entity, face),
        // Clicking empty space clears, the way it does for entities.
        (None, false) => selection.clear(),
        // Ctrl+click on nothing is a miss, not "deselect everything".
        (None, true) => {}
    }
}

/// The face of `entity`'s block under the cursor.
///
/// The ray is built in world space and then pushed into the mesh's own
/// space by the inverse of the entity's transform — one inversion,
/// rather than transforming every corner of every face on every mouse
/// move.
pub(crate) fn face_under(
    resources: &Resources,
    entity: Entity,
    cursor: Vec2,
    viewport_size: Vec2,
) -> Option<u32> {
    let (camera, camera_transform) = crate::gizmos::active_camera(resources)?;
    let ray = kooch_render::projection::viewport_cursor_to_ray(
        cursor,
        viewport_size,
        camera_transform.matrix,
        camera.fov.to_radians(),
        camera.near,
    )?;

    let mesh = mesh_of(resources, entity)?;
    let to_world = resources
        .get::<ComponentRegistry>()?
        .get_cpu::<GlobalTransform>()?
        .get(entity)?
        .matrix;
    // A degenerate transform — a zero scale on some axis — has no
    // inverse, and `inverse()` answers with NaNs rather than saying so.
    let to_local = to_world.inverse();
    if !to_local.is_finite() {
        return None;
    }

    let origin = to_local.transform_point3(ray.origin);
    // A direction is transformed without the translation, and left
    // unnormalised on purpose: a scaled block's `t` then stays
    // comparable with the world-space one it came from.
    let direction = to_local.transform_vector3(ray.direction);
    kooch_blockmesh::face_at(&mesh, origin, direction).map(|hit| hit.face)
}

/// The block mesh an entity is built from, if it has one and it is
/// built.
pub(crate) fn mesh_of(resources: &Resources, entity: Entity) -> Option<BlockMesh> {
    let source = resources
        .get::<ComponentRegistry>()?
        .get_cpu::<Block>()?
        .get(entity)?
        .source?;
    let handle = resources.get::<BuiltBlocks>()?.handle(source)?;
    resources.get::<Assets<BlockMesh>>()?.get(handle).cloned()
}

#[cfg(test)]
mod tests;
