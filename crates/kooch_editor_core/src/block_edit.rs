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
    /// Clicks select single corners.
    Vertex,
    /// Clicks select edges, moving the two corners at their ends.
    Edge,
    /// Clicks select faces.
    Face,
}

impl ElementMode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Object => "Object",
            Self::Vertex => "Vertex",
            Self::Edge => "Edge",
            Self::Face => "Face",
        }
    }

    /// The glyph for the toolbar. Icons over words because these four
    /// are a row you switch between constantly, and four labels is a
    /// sentence you have to read every time.
    pub(crate) fn icon(self) -> &'static str {
        match self {
            Self::Object => crate::icons::CUBE,
            Self::Vertex => crate::icons::DOTS_NINE,
            Self::Edge => crate::icons::LINE_SEGMENT,
            Self::Face => crate::icons::POLYGON,
        }
    }

    /// Every mode, in the order the toolbar shows them.
    pub(crate) const ALL: [Self; 4] = [Self::Object, Self::Vertex, Self::Edge, Self::Face];

    /// Whether clicks select parts of a block rather than entities.
    pub(crate) fn edits_elements(self) -> bool {
        self != Self::Object
    }
}

/// Which parts of which block are selected.
///
/// One entity at a time. Editing elements across several blocks is a
/// different feature and mostly a different UI — the handle would have
/// to straddle two transforms.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct BlockSelection {
    pub(crate) entity: Option<Entity>,
    /// Indices into whatever `mode` names: faces of the mesh, edges of
    /// its `Adjacency`, or corners.
    ///
    /// 🔴 Which is why switching mode clears them. A face index and an
    /// edge index are both `u32` and neither is the other; keeping them
    /// across a switch selects unrelated geometry, silently.
    pub(crate) elements: Vec<u32>,
    /// What is being edited, mirrored here from the overlay.
    ///
    /// 🔴 The gizmo that draws the block reads `Resources`, and the
    /// overlay is taken OUT of `Resources` for the whole frame that
    /// draws it. Anything the drawing needs has to live somewhere the
    /// drawing can reach — this is the resource it already reads.
    pub(crate) mode: ElementMode,
}

impl BlockSelection {
    /// Replaces the selection with one element.
    pub(crate) fn only(&mut self, entity: Entity, element: u32) {
        self.entity = Some(entity);
        self.elements.clear();
        self.elements.push(element);
    }

    /// Adds an element, or removes it when it was already selected.
    ///
    /// Switching entity clears rather than merges: the indices held
    /// address one mesh, and keeping another block's would name
    /// geometry that does not exist.
    pub(crate) fn toggle(&mut self, entity: Entity, element: u32) {
        if self.entity != Some(entity) {
            self.only(entity, element);
            return;
        }
        match self.elements.iter().position(|held| *held == element) {
            Some(at) => {
                self.elements.remove(at);
            }
            None => self.elements.push(element),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.entity = None;
        self.elements.clear();
    }

    pub(crate) fn holds(&self, entity: Entity, element: u32) -> bool {
        self.entity == Some(entity) && self.elements.contains(&element)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// The elements selected on `entity`, or nothing.
    fn of(&self, entity: Entity) -> Option<Vec<u32>> {
        match self.entity == Some(entity) && !self.is_empty() {
            true => Some(self.elements.clone()),
            false => None,
        }
    }
}

/// The corners a selection moves, each once.
///
/// 🔴 Where the three modes stop being three things. A face is its
/// corners, an edge is two of them, a vertex is one — and every edit
/// below this point works on a corner list, so vertex and edge editing
/// need no new transform code at all.
///
/// Deduplicated, because a corner shared by two selected faces that
/// moved twice is the tear the shared positions exist to prevent.
pub(crate) fn corners_of(mesh: &BlockMesh, mode: ElementMode, elements: &[u32]) -> Vec<u32> {
    match mode {
        ElementMode::Face => mesh.corners_of(elements),
        ElementMode::Vertex => {
            let mut corners: Vec<u32> = Vec::new();
            for corner in elements {
                if (*corner as usize) < mesh.positions().len() && !corners.contains(corner) {
                    corners.push(*corner);
                }
            }
            corners
        }
        ElementMode::Edge => {
            let adjacency = kooch_blockmesh::Adjacency::of(mesh);
            let mut corners: Vec<u32> = Vec::new();
            for edge in elements {
                let Some(ends) = adjacency.edge_corners(*edge) else {
                    continue;
                };
                for corner in ends {
                    if !corners.contains(&corner) {
                        corners.push(corner);
                    }
                }
            }
            corners
        }
        ElementMode::Object => Vec::new(),
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
    let editing = mode.edits_elements() && !playing;
    let wanted = match editing {
        true => mode,
        false => ElementMode::Object,
    };
    if let Some(mut selection) = resources.get_mut::<BlockSelection>() {
        // 🔴 A switch between element modes clears too, not just a
        // switch to Object. Face 3 and edge 3 are both `3`; carrying
        // them across would leave unrelated geometry lit and draggable.
        if selection.mode != wanted && !selection.is_empty() {
            selection.clear();
        }
        // Mirrored every frame, whether or not anything is selected:
        // the wireframe has to appear the moment the mode changes, not
        // once something is clicked.
        selection.mode = wanted;
    }
}

/// How near the cursor an edge or a corner counts as clicked.
///
/// Twelve physical pixels. A corner has no area and an edge no width,
/// so without a radius neither is ever hit; too wide and a corner steals
/// every click meant for the edge running out of it.
const REACH: f32 = 12.0;

/// The element of `entity`'s block under the cursor.
///
/// Faces are found with a ray in the mesh's own space — one inverse of
/// the entity's transform, rather than transforming every corner on
/// every mouse move. Vertices and edges are found in screen space,
/// because "close enough" for something with no area is a count of
/// pixels.
pub(crate) fn element_under(
    resources: &Resources,
    entity: Entity,
    cursor: Vec2,
    viewport_size: Vec2,
    mode: ElementMode,
) -> Option<u32> {
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

    match mode {
        ElementMode::Face => {
            let (camera, camera_transform) = crate::gizmos::active_camera(resources)?;
            let ray = kooch_render::projection::viewport_cursor_to_ray(
                cursor,
                viewport_size,
                camera_transform.matrix,
                camera.fov.to_radians(),
                camera.near,
            )?;
            let origin = to_local.transform_point3(ray.origin);
            // A direction is transformed without the translation, and
            // left unnormalised on purpose: a scaled block's `t` then
            // stays comparable with the world-space one it came from.
            let direction = to_local.transform_vector3(ray.direction);
            kooch_blockmesh::face_at(&mesh, origin, direction).map(|hit| hit.element)
        }
        ElementMode::Vertex => {
            let screen = screen_of(resources, to_world, viewport_size)?;
            kooch_blockmesh::vertex_at(&mesh, screen, cursor, REACH).map(|hit| hit.element)
        }
        ElementMode::Edge => {
            let screen = screen_of(resources, to_world, viewport_size)?;
            let adjacency = kooch_blockmesh::Adjacency::of(&mesh);
            kooch_blockmesh::edge_at(&mesh, &adjacency, screen, cursor, REACH)
                .map(|hit| hit.element)
        }
        ElementMode::Object => None,
    }
}

/// Mesh space straight to viewport pixels, for the picks that need it.
fn screen_of(
    resources: &Resources,
    to_world: glam::Mat4,
    viewport_size: Vec2,
) -> Option<kooch_blockmesh::Screen> {
    let (camera, camera_transform) = crate::gizmos::active_camera(resources)?;
    if viewport_size.x < 1.0 || viewport_size.y < 1.0 {
        return None;
    }
    let projection = kooch_render::projection::perspective_infinite_rh_reverse_z(
        camera.fov.to_radians(),
        (viewport_size.x / viewport_size.y).max(0.001),
        camera.near.max(0.001),
    );
    Some(kooch_blockmesh::Screen {
        clip: projection * camera_transform.matrix.inverse() * to_world,
        size: viewport_size,
    })
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
    let elements = selection.of(entity)?;
    let mesh = mesh_of(resources, entity)?;
    let centre = mesh.centre(&corners_of(&mesh, selection.mode, &elements))?;
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
    let elements = selection.of(entity)?;
    let mesh = mesh_of(resources, entity)?;
    let corners = corners_of(&mesh, selection.mode, &elements);
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
    let Some((mode, elements)) = resources
        .get::<BlockSelection>()
        .and_then(|selection| Some((selection.mode, selection.of(entity)?)))
    else {
        return false;
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

    let corners = corners_of(mesh, mode, &elements);
    // 🔴 The pivot is the SELECTION's centre, not the entity's origin.
    // Turning a face about a point it does not contain swings it away
    // rather than turning it.
    let Some(pivot) = mesh.centre(&corners) else {
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

    write_block(resources, &path, &mesh);
}

/// Serialises a block to its file and tells the database it moved.
fn write_block(resources: &mut Resources, path: &std::path::Path, mesh: &BlockMesh) {
    match ron::ser::to_string_pretty(mesh, ron::ser::PrettyConfig::default()) {
        Ok(text) => match std::fs::write(path, text) {
            Ok(()) => {
                tracing::debug!(
                    target: "kooch_editor_core::block_edit",
                    path = %path.display(), "block written",
                );
                crate::actions::handlers::asset_saved(resources, path);
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

/// Extrudes the selected faces and answers the edit for the history.
///
/// The direction is the selection's averaged normal — see
/// [`BlockMesh::extrude_direction`] for why it is not per face — and
/// the distance is one snap step, so a wall comes out at a size the
/// grid agrees with rather than at whatever the mouse was doing.
///
/// The selection follows the extruded faces, so pressing it again
/// continues the wall rather than starting one beside it.
pub(crate) fn extrude_selection(
    resources: &mut Resources,
    entity: Entity,
    distance: f32,
) -> Option<(BlockMesh, BlockMesh)> {
    let source = source_of(resources, entity)?;
    // Face-only, and it stays that way. Extruding an edge or a vertex
    // is a different operation with a different result, not this one
    // applied to fewer corners.
    let faces = resources
        .get::<BlockSelection>()
        .filter(|selection| selection.mode == ElementMode::Face)
        .and_then(|selection| selection.of(entity))?;

    let before = shape_for(resources, source)?;
    let mut after = before.clone();
    let by = after.extrude_direction(&faces, distance)?;
    let extruded = after.extrude(&faces, by)?;

    if !set_shape(resources, source, &after) {
        return None;
    }
    if let Some(mut selection) = resources.get_mut::<BlockSelection>() {
        selection.elements = extruded.faces;
    }
    announce(resources, source);
    save(resources, entity);
    Some((before, after))
}

/// Every corner of the block `entity` is built from.
///
/// What a drag snapshots, so an undo has something to put back.
pub(crate) fn shape_of(resources: &Resources, entity: Entity) -> Option<BlockMesh> {
    mesh_of(resources, entity)
}

/// Every corner of the block behind `source`.
///
/// By source rather than by entity: an undo names the shape, and the
/// entity that made the edit may not even be selected any more.
pub(crate) fn shape_for(resources: &Resources, source: kooch_core::Guid) -> Option<BlockMesh> {
    let handle = resources.get::<BuiltBlocks>()?.handle(source)?;
    resources.get::<Assets<BlockMesh>>()?.get(handle).cloned()
}

/// Writes a block's shape to its own file, found by GUID.
///
/// The by-source twin of [`save`], for an undo that has a shape and no
/// entity to ask.
pub(crate) fn save_source(resources: &mut Resources, source: kooch_core::Guid) {
    let Some(mesh) = shape_for(resources, source) else {
        return;
    };
    let Some(path) = resources
        .get::<kooch_core::asset_database::AssetDatabase>()
        .and_then(|database| database.entry(source).map(|entry| entry.path.clone()))
    else {
        return;
    };
    write_block(resources, &path, &mesh);
}

/// Replaces a block's whole shape, answering whether it landed.
///
/// The whole mesh because an extrude changes the topology: there are
/// faces after it that had no before, and putting positions back would
/// leave those faces indexing corners that are no longer there.
pub(crate) fn set_shape(
    resources: &mut Resources,
    source: kooch_core::Guid,
    shape: &BlockMesh,
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
    *mesh = shape.clone();
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
