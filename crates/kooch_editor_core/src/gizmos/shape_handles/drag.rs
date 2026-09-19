//! Dragging a handle: which one, how far along its axis or around its centre, and the shape it writes.

use super::*;

#[derive(Default)]
pub(crate) struct ShapeHandleState {
    pub(super) drag: Option<Drag>,
    pub(super) hovered: Option<(Entity, Field)>,
}

pub(super) struct Drag {
    pub(super) entity: Entity,
    pub(super) field: Field,
    pub(super) start: BlockShape,
    /// The handle's world position and unit axis at the press.
    pub(super) origin: Vec3,
    pub(super) axis: Vec3,
    /// World length of one local unit along the axis — the block's scale.
    pub(super) scale: f32,
    /// Where along the axis the cursor grabbed.
    pub(super) grab: f32,
    /// Set for a handle dragged around an axis.
    pub(super) around: Option<Around>,
}

/// A drag around an axis, counted in whole turns so a spiral can wind past 360 degrees.
pub(super) struct Around {
    pub(super) centre: Vec3,
    pub(super) to_local: glam::Mat4,
    pub(super) last: f32,
    pub(super) turned: f32,
}

/// The cursor's angle around `centre`, on the local horizontal plane through it.
pub(super) fn angle_at(
    to_local: glam::Mat4,
    centre: Vec3,
    origin: Vec3,
    direction: Vec3,
) -> Option<f32> {
    let (origin, direction) = (
        to_local.transform_point3(origin),
        to_local.transform_vector3(direction),
    );
    if direction.y.abs() < 1.0e-6 {
        return None;
    }
    let hit = origin + direction * ((centre.y - origin.y) / direction.y);
    Some((hit.z - centre.z).atan2(hit.x - centre.x))
}

/// Drives the shape handles for this frame; `true` while one is hovered or dragged, so the transform
/// handles and picking leave the click alone.
pub(crate) fn apply_shape_handles(
    delta: ViewportInputDelta,
    resources: &mut Resources,
    selected: &[Entity],
    snap: SnapSettings,
    actions: &mut Vec<EditorAction>,
) -> bool {
    let mut state = resources.remove::<ShapeHandleState>().unwrap_or_default();
    let active = drive(delta, resources, selected, snap, actions, &mut state);
    resources.insert(state);
    active
}

pub(super) fn drive(
    delta: ViewportInputDelta,
    resources: &mut Resources,
    selected: &[Entity],
    snap: SnapSettings,
    actions: &mut Vec<EditorAction>,
    state: &mut ShapeHandleState,
) -> bool {
    state.hovered = None;
    let [entity] = selected else {
        state.drag = None;
        return false;
    };
    let entity = *entity;
    let Some((shape, to_world)) = shape_and_matrix(resources, entity) else {
        state.drag = None;
        return false;
    };
    let ray = cursor_ray(resources, delta);

    if let Some(drag) = state.drag.as_ref().filter(|drag| drag.entity == entity) {
        if delta.lmb_held {
            if let Some(around) = state.drag.as_mut().and_then(|drag| drag.around.as_mut())
                && let Some((origin, direction)) = ray
                && let Some(angle) = angle_at(around.to_local, around.centre, origin, direction)
            {
                // Accumulated from wrapped steps, so dragging past half a turn keeps winding.
                let step = (angle - around.last + std::f32::consts::PI)
                    .rem_euclid(std::f32::consts::TAU)
                    - std::f32::consts::PI;
                around.turned += step;
                around.last = angle;
                let drag = state.drag.as_ref().expect("dragging");
                let mut turn = drag.start.turn + around_degrees(drag);
                if delta.ctrl_held && snap.rotate_deg > 0.0 {
                    turn = (turn / snap.rotate_deg).round() * snap.rotate_deg;
                }
                let mut live = drag.start;
                live.turn = turn;
                write_shape(resources, entity, live);
            } else if let Some(drag) = state.drag.as_ref()
                && drag.around.is_none()
                && let Some((origin, direction)) = ray
                && let Some(along) = closest_along(drag.origin, drag.axis, origin, direction)
            {
                let moved = (along - drag.grab) / drag.scale;
                let step = delta.ctrl_held.then_some(snap.translate);
                if let Some(value) = dragged(&drag.start, drag.field, moved, step) {
                    let mut live = drag.start;
                    drag.field.set(&mut live, value);
                    write_shape(resources, entity, live);
                }
            }
            state.hovered = state.drag.as_ref().map(|drag| (entity, drag.field));
            return true;
        }
        // Released: put the start back and emit the edit, so the command records the value the drag
        // began from and a drag is one undo step.
        let field = drag.field;
        let start = drag.start;
        state.drag = None;
        if field.get(&shape) != field.get(&start)
            && let Some(component) = resources
                .get::<ComponentNames>()
                .and_then(|names| names.id(std::any::type_name::<BlockShape>()))
        {
            write_shape(resources, entity, start);
            let (name, value) = field.reflected(&shape);
            actions.push(EditorAction::SetField {
                entity,
                component,
                field: name.to_owned(),
                value,
            });
        }
        return true;
    }
    state.drag = None;

    let Some((origin, direction)) = ray else {
        return false;
    };
    let under = handles(&shape)
        .into_iter()
        .filter_map(|handle| {
            let world = to_world.transform_point3(handle.at);
            let along = (world - origin).dot(direction);
            let off = (world - (origin + direction * along)).length();
            (along > 0.0 && off <= along * PICK_SLOPE + HANDLE_SIZE)
                .then_some((along, handle, world))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0));
    let Some((_, handle, world)) = under else {
        return false;
    };
    state.hovered = Some((entity, handle.field));

    if delta.lmb_pressed
        && let Some(centre) = handle.around
    {
        let to_local = to_world.inverse();
        if to_local.is_finite()
            && let Some(last) = angle_at(to_local, centre, origin, direction)
        {
            state.drag = Some(Drag {
                entity,
                field: handle.field,
                start: shape,
                origin: world,
                axis: Vec3::ZERO,
                scale: 1.0,
                grab: 0.0,
                around: Some(Around {
                    centre,
                    to_local,
                    last,
                    turned: 0.0,
                }),
            });
        }
        return true;
    }
    if delta.lmb_pressed {
        let axis = to_world.transform_vector3(handle.axis);
        let scale = axis.length();
        if scale > 1.0e-6
            && let Some(grab) = closest_along(world, axis / scale, origin, direction)
        {
            state.drag = Some(Drag {
                entity,
                field: handle.field,
                start: shape,
                origin: world,
                axis: axis / scale,
                scale,
                grab,
                around: None,
            });
        }
    }
    true
}

pub(super) fn shape_and_matrix(
    resources: &Resources,
    entity: Entity,
) -> Option<(BlockShape, glam::Mat4)> {
    let registry = resources.get::<ComponentRegistry>()?;
    let shape = *registry.get_cpu::<BlockShape>()?.get(entity)?;
    let matrix = registry.get_cpu::<GlobalTransform>()?.get(entity)?.matrix;
    Some((shape, matrix))
}

pub(super) fn write_shape(resources: &mut Resources, entity: Entity, shape: BlockShape) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<BlockShape>()
        && let Some(held) = storage.get_mut(entity)
    {
        *held = shape;
    }
}

pub(super) fn cursor_ray(resources: &Resources, delta: ViewportInputDelta) -> Option<(Vec3, Vec3)> {
    let cursor = delta.cursor_local?;
    let (camera, transform) = super::active_camera(resources)?;
    let ray = kooch_render::projection::viewport_cursor_to_ray(
        cursor,
        delta.viewport_size,
        transform.matrix,
        camera.fov.to_radians(),
        camera.near,
    )?;
    Some((ray.origin, ray.direction.normalize_or_zero()))
}

/// How far along the line `origin + s * axis` the cursor ray passes closest; `None` when the two
/// are parallel and every point is as close as any other.
pub(crate) fn closest_along(origin: Vec3, axis: Vec3, ray_origin: Vec3, ray: Vec3) -> Option<f32> {
    let b = axis.dot(ray);
    let denominator = 1.0 - b * b;
    if denominator.abs() < 1.0e-6 {
        return None;
    }
    let w = origin - ray_origin;
    Some((b * ray.dot(w) - axis.dot(w)) / denominator)
}

pub(super) fn around_degrees(drag: &Drag) -> f32 {
    drag.around
        .as_ref()
        .map_or(0.0, |around| around.turned.to_degrees())
}
