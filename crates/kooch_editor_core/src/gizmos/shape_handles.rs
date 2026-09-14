//! Handles that drag a [`BlockShape`] field in the viewport (#1150). A drag is one field edit, so
//! the block regenerates through `shape_sync`, and undo and the wire come with the `SetField`.

use glam::{Vec3, Vec4};
use kooch_blockmesh::BlockShape;
use kooch_blockmesh::block_shape::{
    KIND_ARCH, KIND_CONE, KIND_CUBE, KIND_CYLINDER, KIND_DOOR, KIND_PLANE, KIND_RAMP, KIND_STAIRS,
};
use kooch_core::resource::Resources;
use kooch_ecs::GlobalTransform;
use kooch_ecs::component::{ComponentNames, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::ReflectValue;
use kooch_gizmos::{Gizmos, Visualizer};
use kooch_gizmos_handles::SnapSettings;

use crate::actions::EditorAction;
use crate::editor_camera::input::ViewportInputDelta;

/// Smallest value a handle can drag a field to, matching the shapes' own floor.
const MIN_VALUE: f32 = 0.01;
/// Half the side of a handle's cube, in world units.
const HANDLE_SIZE: f32 = 0.05;
/// How far off the cursor ray a handle still counts as under it, as a fraction of its distance.
const PICK_SLOPE: f32 = 0.02;

/// A length field one handle drags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Field {
    SizeX,
    SizeY,
    SizeZ,
    Width,
    Rise,
    Run,
    Inner,
    Outer,
    Depth,
    Radius,
    Height,
    Extent,
    Thickness,
    DoorWidth,
    DoorHeight,
    Frame,
}

impl Field {
    pub(crate) fn get(self, shape: &BlockShape) -> f32 {
        match self {
            Field::SizeX => shape.size.x,
            Field::SizeY => shape.size.y,
            Field::SizeZ => shape.size.z,
            Field::Width => shape.width,
            Field::Rise => shape.rise,
            Field::Run => shape.run,
            Field::Inner => shape.inner,
            Field::Outer => shape.outer,
            Field::Depth => shape.depth,
            Field::Radius => shape.radius,
            Field::Height => shape.height,
            Field::Extent => shape.extent,
            Field::Thickness => shape.thickness,
            Field::DoorWidth => shape.door_width,
            Field::DoorHeight => shape.door_height,
            Field::Frame => shape.frame,
        }
    }

    pub(crate) fn set(self, shape: &mut BlockShape, value: f32) {
        match self {
            Field::SizeX => shape.size.x = value,
            Field::SizeY => shape.size.y = value,
            Field::SizeZ => shape.size.z = value,
            Field::Width => shape.width = value,
            Field::Rise => shape.rise = value,
            Field::Run => shape.run = value,
            Field::Inner => shape.inner = value,
            Field::Outer => shape.outer = value,
            Field::Depth => shape.depth = value,
            Field::Radius => shape.radius = value,
            Field::Height => shape.height = value,
            Field::Extent => shape.extent = value,
            Field::Thickness => shape.thickness = value,
            Field::DoorWidth => shape.door_width = value,
            Field::DoorHeight => shape.door_height = value,
            Field::Frame => shape.frame = value,
        }
    }

    /// The reflected field this writes, and its whole value — `size` is one `Vec3`.
    fn reflected(self, shape: &BlockShape) -> (&'static str, ReflectValue) {
        let f32_field = |name| (name, ReflectValue::F32(self.get(shape)));
        match self {
            Field::SizeX | Field::SizeY | Field::SizeZ => ("size", ReflectValue::Vec3(shape.size)),
            Field::Width => f32_field("width"),
            Field::Rise => f32_field("rise"),
            Field::Run => f32_field("run"),
            Field::Inner => f32_field("inner"),
            Field::Outer => f32_field("outer"),
            Field::Depth => f32_field("depth"),
            Field::Radius => f32_field("radius"),
            Field::Height => f32_field("height"),
            Field::Extent => f32_field("extent"),
            Field::Thickness => f32_field("thickness"),
            Field::DoorWidth => f32_field("door_width"),
            Field::DoorHeight => f32_field("door_height"),
            Field::Frame => f32_field("frame"),
        }
    }
}

/// One handle: the field it drags, where it sits and the local axis it moves along.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Handle {
    pub(crate) field: Field,
    pub(crate) at: Vec3,
    pub(crate) axis: Vec3,
}

/// Every handle of a shape, in the block's local space after the pivot — placed on the built
/// mesh's bounds, so a pivot that moves the mesh moves its handles with it.
pub(crate) fn handles(shape: &BlockShape) -> Vec<Handle> {
    let mesh = shape.build();
    let Some(&first) = mesh.positions().first() else {
        return Vec::new();
    };
    let (min, max) = mesh
        .positions()
        .iter()
        .fold((first, first), |(min, max), p| (min.min(*p), max.max(*p)));
    let c = (min + max) / 2.0;
    let at = |field, at: Vec3, axis| Handle { field, at, axis };
    let (x, y, z) = (Vec3::X, Vec3::Y, Vec3::Z);
    let straight = shape.turn.abs() < 1.0e-3;
    match shape.kind {
        KIND_CUBE => vec![
            at(Field::SizeX, Vec3::new(max.x, c.y, c.z), x),
            at(Field::SizeY, Vec3::new(c.x, max.y, c.z), y),
            at(Field::SizeZ, Vec3::new(c.x, c.y, max.z), z),
        ],
        KIND_RAMP => slope(min, max, c),
        KIND_STAIRS if straight => slope(min, max, c),
        KIND_STAIRS => vec![at(Field::Rise, Vec3::new(c.x, max.y, c.z), y)],
        KIND_ARCH => {
            let (inner, outer) = (shape.inner.max(MIN_VALUE), shape.outer);
            vec![
                at(Field::Outer, Vec3::new(c.x, max.y, max.z), y),
                at(Field::Inner, Vec3::new(c.x, min.y + inner, max.z), y),
                at(
                    Field::Depth,
                    Vec3::new(c.x + (inner + outer.max(inner)) / 2.0, min.y, max.z),
                    z,
                ),
            ]
        }
        KIND_CYLINDER | KIND_CONE => vec![
            at(Field::Radius, Vec3::new(max.x, c.y, c.z), x),
            at(Field::Height, Vec3::new(c.x, max.y, c.z), y),
        ],
        KIND_PLANE => vec![
            at(Field::Extent, Vec3::new(max.x, max.y, c.z), x),
            at(Field::Thickness, Vec3::new(c.x, max.y, c.z), y),
        ],
        KIND_DOOR => {
            let (w, h) = (
                shape.door_width.max(MIN_VALUE),
                shape.door_height.max(MIN_VALUE),
            );
            vec![
                at(
                    Field::DoorWidth,
                    Vec3::new(c.x + w / 2.0, min.y + h / 2.0, max.z),
                    x,
                ),
                at(Field::DoorHeight, Vec3::new(c.x, min.y + h, max.z), y),
                at(Field::Frame, Vec3::new(max.x, min.y + h / 2.0, max.z), x),
                at(Field::Depth, Vec3::new(c.x, max.y, max.z), z),
            ]
        }
        _ => Vec::new(),
    }
}

/// The three handles of a straight flight or a ramp: side, top at the back, and back at the base.
fn slope(min: Vec3, max: Vec3, c: Vec3) -> Vec<Handle> {
    vec![
        Handle {
            field: Field::Width,
            at: Vec3::new(max.x, c.y, c.z),
            axis: Vec3::X,
        },
        Handle {
            field: Field::Rise,
            at: Vec3::new(c.x, max.y, max.z),
            axis: Vec3::Y,
        },
        Handle {
            field: Field::Run,
            at: Vec3::new(c.x, min.y, max.z),
            axis: Vec3::Z,
        },
    ]
}

/// Local units a handle moves per unit of its field. Measured rather than derived, so every kind
/// and every pivot answers the same way; `None` when the field does not move its handle.
fn slope_of(shape: &BlockShape, field: Field) -> Option<f32> {
    const STEP: f32 = 0.1;
    let find = |s: &BlockShape| handles(s).into_iter().find(|h| h.field == field);
    let before = find(shape)?;
    let mut moved = *shape;
    field.set(&mut moved, field.get(shape) + STEP);
    let after = find(&moved)?;
    let slope = (after.at - before.at).dot(before.axis) / STEP;
    (slope.abs() > 1.0e-4).then_some(slope)
}

/// The value `field` reaches when its handle has moved `moved` local units along its axis from where
/// it was on `start`, snapped to `snap` when given.
pub(crate) fn dragged(
    start: &BlockShape,
    field: Field,
    moved: f32,
    snap: Option<f32>,
) -> Option<f32> {
    let value = field.get(start) + moved / slope_of(start, field)?;
    let value = match snap.filter(|step| *step > 0.0) {
        Some(step) => (value / step).round() * step,
        None => value,
    };
    Some(value.max(MIN_VALUE))
}

/// The drag in progress, and the handle under the cursor.
#[derive(Default)]
pub(crate) struct ShapeHandleState {
    drag: Option<Drag>,
    hovered: Option<(Entity, Field)>,
}

struct Drag {
    entity: Entity,
    field: Field,
    start: BlockShape,
    /// The handle's world position and unit axis at the press.
    origin: Vec3,
    axis: Vec3,
    /// World length of one local unit along the axis — the block's scale.
    scale: f32,
    /// Where along the axis the cursor grabbed.
    grab: f32,
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

fn drive(
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
            if let Some((origin, direction)) = ray
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
            state.hovered = Some((entity, drag.field));
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
            });
        }
    }
    true
}

fn shape_and_matrix(resources: &Resources, entity: Entity) -> Option<(BlockShape, glam::Mat4)> {
    let registry = resources.get::<ComponentRegistry>()?;
    let shape = *registry.get_cpu::<BlockShape>()?.get(entity)?;
    let matrix = registry.get_cpu::<GlobalTransform>()?.get(entity)?.matrix;
    Some((shape, matrix))
}

fn write_shape(resources: &mut Resources, entity: Entity, shape: BlockShape) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<BlockShape>()
        && let Some(held) = storage.get_mut(entity)
    {
        *held = shape;
    }
}

fn cursor_ray(resources: &Resources, delta: ViewportInputDelta) -> Option<(Vec3, Vec3)> {
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

/// Draws the handles of a selected shaped block, the hovered or dragged one highlighted.
#[derive(Default)]
pub(crate) struct ShapeHandleVisualizer;

impl Visualizer<BlockShape> for ShapeHandleVisualizer {
    /// Everything is in `draw_with`, which can see the hovered handle.
    fn draw(&self, _shape: &BlockShape, _transform: &GlobalTransform, _gizmos: &mut Gizmos<'_>) {}

    fn draw_with(
        &self,
        shape: &BlockShape,
        transform: &GlobalTransform,
        entity: Entity,
        resources: &Resources,
        gizmos: &mut Gizmos<'_>,
    ) {
        // Element modes edit the mesh by hand; the shape's handles would compete with it.
        if resources
            .get::<crate::block_edit::BlockSelection>()
            .is_some_and(|selection| selection.mode.edits_elements())
        {
            return;
        }
        let hovered = resources
            .get::<ShapeHandleState>()
            .and_then(|state| state.hovered)
            .filter(|(held, _)| *held == entity)
            .map(|(_, field)| field);
        for handle in handles(shape) {
            let colour = match (hovered == Some(handle.field), handle.axis) {
                (true, _) => Vec4::new(1.0, 0.85, 0.1, 1.0),
                (false, axis) if axis == Vec3::X => Vec4::new(0.9, 0.25, 0.25, 1.0),
                (false, axis) if axis == Vec3::Y => Vec4::new(0.3, 0.85, 0.3, 1.0),
                _ => Vec4::new(0.3, 0.5, 1.0, 1.0),
            };
            let at = transform.matrix.transform_point3(handle.at);
            gizmos.filled_aabb(at, Vec3::splat(HANDLE_SIZE), colour);
        }
    }
}

#[cfg(test)]
mod tests;
