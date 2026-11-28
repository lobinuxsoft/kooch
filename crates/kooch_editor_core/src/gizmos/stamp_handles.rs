//! A [`Terrain`]'s stamps in the viewport (#1954): click a stamp's centre to select it, and the
//! transform gizmo moves it, turns it about the ground's up and sizes its shape.
//!
//! Adding, removing and reordering stay in the Inspector's list, as for a spline's knots. A gizmo
//! drag writes the component live and emits one `SetFieldFrom` on release, so it undoes in one step.

use glam::{Mat3, Mat4, Quat, Vec3};

use kooch_core::resource::Resources;
use kooch_ecs::GlobalTransform;
use kooch_ecs::component::{ComponentNames, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::Reflect;
use kooch_gizmos_handles::{AxisMask, HandleSet};
use kooch_terrain::{
    Base, PlanetTerrain, SHAPE_BOX, SHAPE_CAPSULE, SHAPE_CONE, SHAPE_TORUS, Stamp, Terrain,
    stamp_frame,
};

use crate::actions::EditorAction;
use crate::editor_camera::input::ViewportInputDelta;

/// How far off the cursor ray a grip still counts as under it, as a fraction of its distance.
const PICK_SLOPE: f32 = 0.02;

/// The smallest a drag sizes a stamp to, in metres.
const SIZE_FLOOR: f32 = 0.01;

#[derive(Default)]
pub(crate) struct StampHandleState {
    pub(super) hovered: Option<(Entity, usize)>,
    /// The stamp the transform gizmo acts on.
    pub(super) selected: Option<(Entity, usize)>,
    /// The list as the gizmo drag found it, so the gesture is one undo step.
    start: Option<Vec<Stamp>>,
    /// The list while a drag runs. Kept here, not on the component, so the ground is regenerated
    /// once on release rather than every frame of the drag.
    live: Option<(Entity, Vec<Stamp>)>,
}

/// A stamp's centre and axes in world space: x and z along the ground, y its up.
pub(super) fn world_frame(matrix: &Mat4, stamp: &Stamp, base: Base) -> (Vec3, Mat3) {
    let (centre, axes) = stamp_frame(stamp, base);
    let turn = Mat3::from_mat4(*matrix) * axes.as_mat3();
    let axes = Mat3::from_cols(
        turn.x_axis.normalize_or(Vec3::X),
        turn.y_axis.normalize_or(Vec3::Y),
        turn.z_axis.normalize_or(Vec3::Z),
    );
    (matrix.transform_point3(centre.as_vec3()), axes)
}

/// The base `entity`'s generator picked; a sphere until it is known.
pub(super) fn base_of(resources: &Resources, entity: Entity) -> Base {
    resources
        .get::<PlanetTerrain>()
        .and_then(|terrain| terrain.base(entity))
        .unwrap_or(Base::Sphere)
}

/// Picks a stamp under the cursor; `true` while one is hovered, so the click is the stamp's.
pub(crate) fn apply_stamp_handles(
    delta: ViewportInputDelta,
    resources: &mut Resources,
    selected: &[Entity],
) -> bool {
    let mut state = resources.remove::<StampHandleState>().unwrap_or_default();
    state.hovered = None;
    if state
        .selected
        .is_some_and(|(entity, _)| !selected.contains(&entity))
    {
        state.selected = None;
    }
    let under = super::shape_handles::cursor_ray(resources, delta)
        .and_then(|(origin, direction)| pick(resources, selected, origin, direction));
    let active = match under {
        // The selected stamp's grip sits under the gizmo's centre: leave that click to the gizmo.
        Some(grip) if state.selected == Some(grip) => false,
        Some(grip) => {
            state.hovered = Some(grip);
            if delta.lmb_pressed {
                state.selected = Some(grip);
            }
            true
        }
        None => {
            // The gizmo's own handles sit away from the stamp, so a click on them is not empty space.
            let gizmo = resources
                .get::<HandleSet>()
                .is_some_and(HandleSet::is_active);
            if delta.lmb_pressed && !gizmo {
                state.selected = None;
            }
            false
        }
    };
    resources.insert(state);
    active
}

/// The nearest stamp centre on the ray, among the selected terrains.
fn pick(
    resources: &Resources,
    selected: &[Entity],
    origin: Vec3,
    direction: Vec3,
) -> Option<(Entity, usize)> {
    let mut best: Option<(f32, Entity, usize)> = None;
    for &entity in selected {
        let Some((terrain, matrix)) = read(resources, entity) else {
            continue;
        };
        let base = base_of(resources, entity);
        for (index, stamp) in terrain.stamps.iter().enumerate() {
            let (at, _) = world_frame(&matrix, stamp, base);
            let along = (at - origin).dot(direction);
            let off = (at - (origin + direction * along)).length();
            let reach = super::shape_handles::handle_size(resources, at);
            if along > 0.0
                && off <= along * PICK_SLOPE + reach
                && best.is_none_or(|(nearest, _, _)| along < nearest)
            {
                best = Some((along, entity, index));
            }
        }
    }
    best.map(|(_, entity, index)| (entity, index))
}

/// The stamp the gizmo acts on, when it belongs to `entity` and still exists.
fn chosen(resources: &Resources, entity: Entity) -> Option<(Terrain, Mat4, usize)> {
    let (held, index) = resources.get::<StampHandleState>()?.selected?;
    if held != entity {
        return None;
    }
    let (terrain, matrix) = read(resources, entity)?;
    terrain.stamps.get(index)?;
    Some((terrain, matrix, index))
}

/// What a selected stamp offers: move on every axis, turn about its up only, size on every axis
/// (a sphere's radius takes whichever moved most).
pub(crate) fn accepts(resources: &Resources, entity: Entity) -> Option<[AxisMask; 3]> {
    chosen(resources, entity)?;
    Some([AxisMask::ALL, AxisMask::Y, AxisMask::ALL])
}

/// Where the selected stamp's centre is, in world space.
pub(crate) fn selected_origin(resources: &Resources, entity: Entity) -> Option<Vec3> {
    let (terrain, matrix, index) = chosen(resources, entity)?;
    let (at, _) = world_frame(
        &matrix,
        terrain.stamps.get(index)?,
        base_of(resources, entity),
    );
    Some(at)
}

/// The selected stamp's axes in world space, which its rotate ring and size cubes follow.
pub(crate) fn selected_basis(resources: &Resources, entity: Entity) -> Option<Mat3> {
    let (terrain, matrix, index) = chosen(resources, entity)?;
    let (_, axes) = world_frame(
        &matrix,
        terrain.stamps.get(index)?,
        base_of(resources, entity),
    );
    Some(axes)
}

/// Moves the selected stamp by a world-space translation.
pub(crate) fn translate_selected(resources: &mut Resources, entity: Entity, by: Vec3) {
    let Some((mut terrain, matrix, index)) = chosen(resources, entity) else {
        return;
    };
    if !(matrix.determinant().abs() > 1.0e-12) {
        return;
    }
    let local = matrix.inverse().transform_vector3(by);
    if let Some(stamp) = terrain.stamps.get_mut(index) {
        stamp.position += local;
    }
    write(resources, entity, terrain.stamps);
}

/// Turns the selected stamp about its up by a gizmo rotation.
pub(crate) fn turn_selected(resources: &mut Resources, entity: Entity, by: Quat) {
    let Some(up) = selected_basis(resources, entity).map(|axes| axes.y_axis) else {
        return;
    };
    let Some((mut terrain, _, index)) = chosen(resources, entity) else {
        return;
    };
    // The signed angle about `up`: `to_axis_angle` may flip the axis and lose the sign.
    let degrees = (2.0 * Vec3::new(by.x, by.y, by.z).dot(up).atan2(by.w)).to_degrees();
    if let Some(stamp) = terrain.stamps.get_mut(index) {
        stamp.turn = (stamp.turn + degrees + 180.0).rem_euclid(360.0) - 180.0;
    }
    write(resources, entity, terrain.stamps);
}

/// Sizes the selected stamp by a gizmo scale along its own axes.
pub(crate) fn scale_selected(resources: &mut Resources, entity: Entity, by: Vec3) {
    let Some((mut terrain, _, index)) = chosen(resources, entity) else {
        return;
    };
    let Some(stamp) = terrain.stamps.get_mut(index) else {
        return;
    };
    match stamp.shape {
        SHAPE_BOX => {
            stamp.size = (stamp.size.max(Vec3::splat(SIZE_FLOOR)) * by).max(Vec3::splat(SIZE_FLOOR))
        }
        // Round sideways, sized up: the ground's axes size the radius, its up the other length.
        SHAPE_CAPSULE | SHAPE_CONE => {
            stamp.radius = grown(stamp.radius, furthest(&[by.x, by.z]));
            stamp.height = grown(stamp.height, by.y);
        }
        SHAPE_TORUS => {
            stamp.radius = grown(stamp.radius, furthest(&[by.x, by.z]));
            stamp.tube = grown(stamp.tube, by.y);
        }
        _ => stamp.radius = grown(stamp.radius, furthest(&[by.x, by.y, by.z])),
    }
    write(resources, entity, terrain.stamps);
}

/// A radius is one number: the axis that moved furthest from 1 is the factor.
fn furthest(by: &[f32]) -> f32 {
    by.iter()
        .copied()
        .max_by(|a, b| (a - 1.0).abs().total_cmp(&(b - 1.0).abs()))
        .unwrap_or(1.0)
}

fn grown(length: f32, by: f32) -> f32 {
    (length.max(SIZE_FLOOR) * by).max(SIZE_FLOOR)
}

/// Records the list a gizmo drag begins from. Call on the frame the drag starts.
pub(crate) fn began_gizmo_drag(resources: &mut Resources, entity: Entity) {
    let start = read(resources, entity).map(|(terrain, _)| terrain.stamps);
    if let Some(state) = resources.get_mut::<StampHandleState>() {
        state.live = start.clone().map(|stamps| (entity, stamps));
        state.start = start;
    }
}

/// Drops the selected stamp where the cursor meets its terrain's ground, when the author asks for
/// a surface snap (Ctrl+Shift). The ray skips the stamp itself, so it cannot climb its own hill.
pub(crate) fn snap_selected(
    resources: &mut Resources,
    delta: ViewportInputDelta,
    entity: Entity,
) -> bool {
    if !crate::surface_snap::engaged(delta) {
        return false;
    }
    let Some((mut terrain, matrix, index)) = chosen(resources, entity) else {
        return false;
    };
    let Some((origin, direction)) = super::shape_handles::cursor_ray(resources, delta) else {
        return false;
    };
    let Some(hit) = kooch_terrain::ground_hit_without(resources, origin, direction, entity, index)
    else {
        return false;
    };
    if !(matrix.determinant().abs() > 1.0e-12) {
        return false;
    }
    let point = matrix
        .inverse()
        .transform_point3(origin + direction * hit.distance);
    if let Some(stamp) = terrain.stamps.get_mut(index) {
        stamp.position = point;
    }
    write(resources, entity, terrain.stamps);
    true
}

/// The stamps a drag holds for `entity`, which the visualizer draws in place of the component's.
pub(super) fn live_stamps(resources: &Resources, entity: Entity) -> Option<Vec<Stamp>> {
    let (held, stamps) = resources.get::<StampHandleState>()?.live.as_ref()?;
    (*held == entity).then(|| stamps.clone())
}

/// Emits the edit for a finished gizmo drag, when a stamp actually changed.
pub(crate) fn ended_gizmo_drag(
    resources: &mut Resources,
    entity: Entity,
    actions: &mut Vec<EditorAction>,
) {
    let Some(state) = resources.get_mut::<StampHandleState>() else {
        return;
    };
    let (Some(start), live) = (state.start.take(), state.live.take()) else {
        return;
    };
    // The ground regenerates now, once, from what the drag left.
    if let Some((held, stamps)) = live.filter(|(held, _)| *held == entity) {
        store(resources, held, stamps);
    }
    let Some((after, _)) = read(resources, entity) else {
        return;
    };
    if after.stamps == start {
        return;
    }
    let Some(component) = resources
        .get::<ComponentNames>()
        .and_then(|names| names.id(std::any::type_name::<Terrain>()))
    else {
        return;
    };
    let before = Terrain {
        stamps: start,
        ..after.clone()
    };
    let (Some(after), Some(before)) = (after.reflect_get("stamps"), before.reflect_get("stamps"))
    else {
        return;
    };
    actions.push(EditorAction::SetFieldFrom {
        entity,
        component,
        field: "stamps".to_owned(),
        before,
        after,
    });
}

/// The stamp the gizmo is on, for the visualizer to mark.
pub(super) fn selected_index(resources: &Resources, entity: Entity) -> Option<usize> {
    let (held, index) = resources.get::<StampHandleState>()?.selected?;
    (held == entity).then_some(index)
}

pub(super) fn hovered_index(resources: &Resources, entity: Entity) -> Option<usize> {
    let (held, index) = resources.get::<StampHandleState>()?.hovered?;
    (held == entity).then_some(index)
}

/// The terrain with the stamps a running drag holds, if any.
fn read(resources: &Resources, entity: Entity) -> Option<(Terrain, Mat4)> {
    let registry = resources.get::<ComponentRegistry>()?;
    let mut terrain = registry.get_cpu::<Terrain>()?.get(entity)?.clone();
    let matrix = registry.get_cpu::<GlobalTransform>()?.get(entity)?.matrix;
    if let Some(stamps) = live_stamps(resources, entity) {
        terrain.stamps = stamps;
    }
    Some((terrain, matrix))
}

/// Into the running drag when there is one, else onto the component.
fn write(resources: &mut Resources, entity: Entity, stamps: Vec<Stamp>) {
    if let Some(state) = resources.get_mut::<StampHandleState>()
        && let Some((held, live)) = state.live.as_mut()
        && *held == entity
    {
        *live = stamps;
        return;
    }
    store(resources, entity, stamps);
}

fn store(resources: &mut Resources, entity: Entity, stamps: Vec<Stamp>) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(mut storage) = registry.get_cpu_mut::<Terrain>()
        && let Some(held) = storage.get_mut(entity)
    {
        held.stamps = stamps;
    }
}
