//! Selecting parts of a block, rather than the whole entity.

use glam::{Vec2, Vec3};
use kooch_blockmesh::{Block, BlockMesh, BuiltBlocks};
use kooch_core::resource::Resources;
use kooch_ecs::GlobalTransform;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_gizmos_handles::AxisMask;

/// What a click in the viewport is aiming at.
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
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct BlockSelection {
    pub(crate) entity: Option<Entity>,
    /// Indices into whatever `mode` names: faces of the mesh, edges of its `Adjacency`, or corners.
    pub(crate) elements: Vec<u32>,
    /// What is being edited, mirrored here from the overlay.
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

    #[cfg(test)]
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

/// What a click in element mode lands on.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ElementClick {
    /// An element of the edited block, or a miss that stays with it.
    Element,
    /// Another entity's geometry: select it and keep the element mode.
    Switch(Entity),
}

/// 🔴 #1118: another entity wins when no element was hit, or when it is nearer along the cursor
/// ray than the edited block — a 12 px vertex or edge pick ignores what covers it.
/// `hit` is the nearest entity and its distance; `block` is the edited block's distance.
pub(crate) fn resolve_click(
    editing: Entity,
    element: Option<u32>,
    hit: Option<(Entity, f32)>,
    block: Option<f32>,
) -> ElementClick {
    match (hit, element, block) {
        (Some((other, _)), None, _) if other != editing => ElementClick::Switch(other),
        (Some((other, distance)), Some(_), Some(block)) if other != editing && distance < block => {
            ElementClick::Switch(other)
        }
        _ => ElementClick::Element,
    }
}

/// How far along the cursor ray `entity`'s block is struck, in world units.
pub(crate) fn block_distance(
    resources: &Resources,
    entity: Entity,
    cursor: Vec2,
    viewport_size: Vec2,
) -> Option<f32> {
    let mesh = mesh_of(resources, entity)?;
    let to_world = resources
        .get::<ComponentRegistry>()?
        .get_cpu::<GlobalTransform>()?
        .get(entity)?
        .matrix;
    let to_local = to_world.inverse();
    if !to_local.is_finite() {
        return None;
    }
    let (camera, camera_transform) = crate::gizmos::active_camera(resources)?;
    let ray = kooch_render::projection::viewport_cursor_to_ray(
        cursor,
        viewport_size,
        camera_transform.matrix,
        camera.fov.to_radians(),
        camera.near,
    )?;
    let origin = to_local.transform_point3(ray.origin);
    let direction = to_local.transform_vector3(ray.direction);
    kooch_blockmesh::face_at(&mesh, origin, direction).map(|hit| hit.distance)
}

/// Drops the face selection when nothing should be editing faces.
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
    if let Some(selection) = resources.get_mut::<BlockSelection>() {
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
const REACH: f32 = 12.0;

/// The element of `entity`'s block under the cursor.
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

/// What a block element offers the transform gizmo, per mode: Translate, Rotate, Scale (#1469).
/// `None` when nothing is selected, which leaves the entity's own offer in place.
///
/// 🔴 Moving only. A face has an orientation of its own in the mesh and no size apart from the
/// corners around it, so turning or growing one is a mesh operation and not a transform — the
/// rings were drawn and `edit_selection` dropped their delta on the floor.
pub(crate) fn accepts(resources: &Resources, entity: Entity) -> Option<[AxisMask; 3]> {
    selection_origin(resources, entity)?;
    Some([AxisMask::ALL, AxisMask::NONE, AxisMask::NONE])
}

/// Where a handle for the current face selection belongs, in world space.
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
pub(crate) fn edit_selection(
    resources: &mut Resources,
    entity: Entity,
    delta: kooch_gizmos_handles::TransformDelta,
) -> bool {
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

    let Some(mesh) = resources
        .get_mut::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu_mut::<Block>())
        .and_then(|storage| storage.get_mut(entity))
        .map(|block| &mut block.mesh)
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

    // The render mesh and the collider are generated from this, and both are cached per entity.
    // Forgetting is what makes the next frame rebuild them.
    if let Some(built) = resources.get_mut::<BuiltBlocks>() {
        built.forget(entity);
    }
    true
}

/// Marks the scene holding `entity` as having unsaved work.
///
/// 🔴 What writing a `.block` used to be (#1452). The geometry is a component now, so an edit is
/// saved when the scene is — there is no second file to keep in step and no moment where the two
/// disagree.
pub(crate) fn touch(resources: &mut Resources, entity: Entity) {
    let Some(scene) = resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<kooch_ecs::SceneMember>()?.get(entity))
        .map(|member| member.scene)
    else {
        return;
    };
    if let Some(manager) = resources.get_mut::<kooch_ecs::SceneManager>() {
        manager.mark_scene_dirty(scene);
    }
}

/// Extrudes the selected faces and answers the edit for the history.
pub(crate) fn extrude_selection(
    resources: &mut Resources,
    entity: Entity,
    distance: f32,
) -> Option<(BlockMesh, BlockMesh)> {
    // Face-only, and it stays that way. Extruding an edge or a vertex
    // is a different operation with a different result, not this one
    // applied to fewer corners.
    let faces = resources
        .get::<BlockSelection>()
        .filter(|selection| selection.mode == ElementMode::Face)
        .and_then(|selection| selection.of(entity))?;

    let before = mesh_of(resources, entity)?;
    let mut after = before.clone();
    let by = after.extrude_direction(&faces, distance)?;
    let extruded = after.extrude(&faces, by)?;

    if !set_shape(resources, entity, &after) {
        return None;
    }
    if let Some(selection) = resources.get_mut::<BlockSelection>() {
        selection.elements = extruded.faces;
    }
    touch(resources, entity);
    Some((before, after))
}

/// Every corner of the block `entity` is built from.
///
/// What a drag snapshots, so an undo has something to put back.
pub(crate) fn shape_of(resources: &Resources, entity: Entity) -> Option<BlockMesh> {
    mesh_of(resources, entity)
}

/// Replaces a block's whole shape, answering whether it landed.
pub(crate) fn set_shape(resources: &mut Resources, entity: Entity, shape: &BlockMesh) -> bool {
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return false;
    };
    let Some(block) = registry
        .get_cpu_mut::<Block>()
        .and_then(|storage| storage.get_mut(entity))
    else {
        return false;
    };
    block.mesh = shape.clone();
    true
}

/// The block mesh an entity is built from.
///
/// 🔴 Straight off the component (#1452). There is no asset to resolve, no load to wait for and no
/// handle to go stale: the geometry is in the scene, which is why each block can now diverge from
/// the one it was copied from.
pub(crate) fn mesh_of(resources: &Resources, entity: Entity) -> Option<BlockMesh> {
    Some(
        resources
            .get::<ComponentRegistry>()?
            .get_cpu::<Block>()?
            .get(entity)?
            .mesh
            .clone(),
    )
}

pub(crate) mod shape_sync;
pub(crate) mod sweep_sync;

#[cfg(test)]
mod tests;
