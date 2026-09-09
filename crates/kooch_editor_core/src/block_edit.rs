//! Selecting parts of a block, rather than the whole entity.

use glam::{Vec2, Vec3};
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

/// Drops the face selection when nothing should be editing faces.
///
/// 🔴 Clearing, not gating. Gating the handle left the highlight
/// painted and the gizmo grabbable, and a drag that reaches neither the
/// history nor the file is worse than one that does nothing — it looks
/// like it worked.
///
/// Two ways to stop: switching to Object, and pressing Play. The second
/// matters more, because the world Play restores is not the one the
/// selection's face indices were read from.
///
/// Reconciled every frame rather than hooked to each transition: there
/// are several ways to reach both — a toolbar, a chord, the project
/// stopping on its own — and catching them one at a time is how one
/// stays live.
pub(crate) fn drop_selection_unless_editing(
    resources: &mut Resources,
    mode: ElementMode,
    playing: bool,
) {
    if mode == ElementMode::Face && !playing {
        return;
    }
    if let Some(mut selection) = resources.get_mut::<BlockSelection>()
        && !selection.is_empty()
    {
        selection.clear();
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

/// Where a handle for the current face selection belongs, in world
/// space.
///
/// The selection's centre, not the entity's origin: a handle at the
/// origin while the face you grabbed is a metre away reads as a gizmo
/// for the wrong thing, and the drag axes would be right for an object
/// nobody is moving.
pub(crate) fn selection_origin(resources: &Resources, entity: Entity) -> Option<Vec3> {
    let selection = resources.get::<BlockSelection>()?;
    if selection.entity != Some(entity) || selection.is_empty() {
        return None;
    }
    let mesh = mesh_of(resources, entity)?;
    let centre = mesh.centre_of(&selection.faces)?;
    let to_world = resources
        .get::<ComponentRegistry>()?
        .get_cpu::<GlobalTransform>()?
        .get(entity)?
        .matrix;
    Some(to_world.transform_point3(centre))
}

/// The world-space box the current face selection occupies.
///
/// What F frames in face mode. The selection, not the block: pressing
/// F after clicking one face of a wall should show you that face, and
/// framing the whole wall is what F already did from object mode.
pub(crate) fn selection_bounds(resources: &Resources, entity: Entity) -> Option<(Vec3, Vec3)> {
    let selection = resources.get::<BlockSelection>()?;
    if selection.entity != Some(entity) || selection.is_empty() {
        return None;
    }
    let mesh = mesh_of(resources, entity)?;
    let corners = mesh.corners_of(&selection.faces);
    if corners.is_empty() {
        return None;
    }
    let to_world = resources
        .get::<ComponentRegistry>()?
        .get_cpu::<GlobalTransform>()?
        .get(entity)?
        .matrix;

    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for corner in corners {
        let world = to_world.transform_point3(mesh.positions()[corner as usize]);
        min = min.min(world);
        max = max.max(world);
    }
    Some((min, max))
}

/// Moves the selected faces by a world-space delta.
///
/// Answers whether anything moved, so the caller knows to leave the
/// entity's own `Transform` alone.
///
/// The mesh is edited in the asset itself rather than in a copy: a
/// block's shape IS the asset, and every entity naming that source is
/// the same shape by definition.
pub(crate) fn edit_selection(
    resources: &mut Resources,
    entity: Entity,
    delta: kooch_gizmos_handles::TransformDelta,
) -> bool {
    let Some(source) = source_of(resources, entity) else {
        return false;
    };
    let faces = match resources.get::<BlockSelection>() {
        Some(selection) if selection.entity == Some(entity) && !selection.is_empty() => {
            selection.faces.clone()
        }
        _ => return false,
    };

    // World to local. A translation is a direction, so it loses the
    // matrix's translation; a rotation is expressed in the entity's own
    // basis; a scale factor is already unitless and passes through.
    let Some(to_local) = resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<GlobalTransform>()?.get(entity))
        .map(|transform| transform.matrix.inverse())
        .filter(glam::Mat4::is_finite)
    else {
        return false;
    };

    let Some(handle) = resources
        .get::<BuiltBlocks>()
        .and_then(|built| built.handle(source))
    else {
        return false;
    };
    let Some(mesh) = resources
        .get_mut::<Assets<BlockMesh>>()
        .and_then(|assets| assets.get_mut(handle))
    else {
        return false;
    };

    let corners = mesh.corners_of(&faces);
    // 🔴 The pivot is the SELECTION's centre, not the entity's origin.
    // Turning a face about a point it does not contain swings it away
    // rather than turning it.
    let Some(pivot) = mesh.centre_of(&faces) else {
        return false;
    };
    match delta {
        kooch_gizmos_handles::TransformDelta::Translation(by) => {
            mesh.move_corners(&corners, to_local.transform_vector3(by));
        }
        kooch_gizmos_handles::TransformDelta::Rotation(by) => {
            let basis = glam::Quat::from_mat4(&to_local).normalize();
            mesh.turn_corners(&corners, pivot, basis * by * basis.inverse());
        }
        kooch_gizmos_handles::TransformDelta::Scale(by) => {
            mesh.scale_corners(&corners, pivot, by);
        }
    }

    // The render mesh and the collider are generated from this, and
    // both are cached under the source's GUID. Forgetting is what makes
    // the next frame rebuild them; without it the block keeps the shape
    // it had when it was first built.
    if let Some(mut built) = resources.get_mut::<BuiltBlocks>() {
        built.forget(source);
    }
    true
}

/// Tells every consumer of this source that its bytes moved.
///
/// The project derives its own render mesh and collider from the same
/// file, and a reload overwrites the value under the existing handle —
/// so without this its collider stays the shape the block was born
/// with, however far this side moved it.
pub(crate) fn announce(resources: &mut Resources, source: kooch_core::Guid) {
    if let Some(mut reloaded) = resources.get_mut::<kooch_core::asset_loader::ReloadedAssets>() {
        reloaded.bump(source);
    }
}

/// Writes the edited shape back to its `.block` file.
///
/// The asset is the shape: an edit that lives only in `Assets` is one
/// the next load throws away. Called on release rather than per frame —
/// a drag is one edit, and rewriting the file each frame is a rescan of
/// the project each frame.
pub(crate) fn save(resources: &mut Resources, entity: Entity) {
    let Some(source) = source_of(resources, entity) else {
        return;
    };
    let Some(mesh) = mesh_of(resources, entity) else {
        return;
    };
    let Some(path) = resources
        .get::<kooch_core::asset_database::AssetDatabase>()
        .and_then(|database| database.entry(source).map(|entry| entry.path.clone()))
    else {
        return;
    };

    match ron::ser::to_string_pretty(&mesh, ron::ser::PrettyConfig::default()) {
        Ok(text) => match std::fs::write(&path, text) {
            Ok(()) => {
                tracing::debug!(
                    target: "kooch_editor_core::block_edit",
                    path = %path.display(), "block written",
                );
                announce(resources, source);
                crate::actions::handlers::asset_saved(resources, &path);
            }
            Err(error) => tracing::error!(
                target: "kooch_editor_core::block_edit",
                path = %path.display(), %error, "could not write the block",
            ),
        },
        Err(error) => tracing::error!(
            target: "kooch_editor_core::block_edit",
            %error, "could not serialise the block",
        ),
    }
}

/// Every corner of the block `entity` is built from.
///
/// What a drag snapshots, so an undo has something to put back.
pub(crate) fn corners_of(resources: &Resources, entity: Entity) -> Option<Vec<Vec3>> {
    Some(mesh_of(resources, entity)?.positions().to_vec())
}

/// Puts a whole set of corners back, answering whether it landed.
///
/// Refuses a count that does not match rather than writing what fits:
/// an undo whose snapshot is one corner short would silently reshape
/// the block into something nobody authored.
pub(crate) fn set_corners(
    resources: &mut Resources,
    source: kooch_core::Guid,
    corners: &[Vec3],
) -> bool {
    let Some(handle) = resources
        .get::<BuiltBlocks>()
        .and_then(|built| built.handle(source))
    else {
        return false;
    };
    let Some(mesh) = resources
        .get_mut::<Assets<BlockMesh>>()
        .and_then(|assets| assets.get_mut(handle))
    else {
        return false;
    };
    if mesh.positions().len() != corners.len() {
        tracing::warn!(
            target: "kooch_editor_core::block_edit",
            held = corners.len(), now = mesh.positions().len(),
            "the block gained or lost corners since this edit; not undoing it",
        );
        return false;
    }
    mesh.set_positions(corners);
    true
}

/// The source a block names.
pub(crate) fn source_of(resources: &Resources, entity: Entity) -> Option<kooch_core::Guid> {
    resources
        .get::<ComponentRegistry>()?
        .get_cpu::<Block>()?
        .get(entity)?
        .source
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
