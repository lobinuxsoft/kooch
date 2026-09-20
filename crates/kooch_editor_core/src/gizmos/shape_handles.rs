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
    Turn,
    Core,
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
            Field::Turn => shape.turn,
            Field::Core => shape.core,
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
            Field::Turn => shape.turn = value,
            Field::Core => shape.core = value,
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
            Field::Turn => f32_field("turn"),
            Field::Core => f32_field("core"),
        }
    }
}

/// One handle: the field it drags, where it sits and the local axis it moves along.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Handle {
    pub(crate) field: Field,
    pub(crate) at: Vec3,
    pub(crate) axis: Vec3,
    /// For a handle dragged around an axis instead of along one: the axis's point at the handle's
    /// height, in local space. The stairs' `turn`.
    pub(crate) around: Option<Vec3>,
}

/// Every handle of a shape, in the block's local space after the pivot — placed on the built
/// mesh's bounds, so a pivot that moves the mesh moves its handles with it.
pub(crate) fn handles(shape: &BlockShape) -> Vec<Handle> {
    let mesh = shape.build();
    let Some(&first) = mesh.positions().first() else {
        return Vec::new();
    };
    let (min, max) = bounds(mesh.positions(), first);
    let c = (min + max) / 2.0;
    // 🔴 The face away from the pivot on each axis: the pivot pins its own side to the origin, so a
    // handle there never moves however far the field is dragged.
    let far = |i: usize| {
        let mut axis = Vec3::ZERO;
        match shape.pivot[i] > 0.0 {
            true => {
                axis[i] = -1.0;
                (min[i], axis)
            }
            false => {
                axis[i] = 1.0;
                (max[i], axis)
            }
        }
    };
    let ((fx, ax), (fy, ay), (fz, az)) = (far(0), far(1), far(2));
    let near_y = if fy == max.y { min.y } else { max.y };
    let linear = |field, at, axis| Handle {
        field,
        at,
        axis,
        around: None,
    };
    let v = Vec3::new;
    match shape.kind {
        KIND_CUBE => vec![
            linear(Field::SizeX, v(fx, c.y, c.z), ax),
            linear(Field::SizeY, v(c.x, fy, c.z), ay),
            linear(Field::SizeZ, v(c.x, c.y, fz), az),
        ],
        KIND_RAMP => vec![
            linear(Field::Width, v(fx, c.y, c.z), ax),
            linear(Field::Rise, v(c.x, fy, fz), ay),
            linear(Field::Run, v(c.x, near_y, fz), az),
        ],
        KIND_STAIRS if shape.turn.abs() < 1.0e-3 => {
            let core = shape.core.max(MIN_VALUE);
            vec![
                linear(Field::Width, v(fx, c.y, c.z), ax),
                linear(Field::Rise, v(c.x, fy, fz), ay),
                linear(Field::Run, v(c.x, near_y, fz), az),
                // Where the flight's axis will be once it turns: `core` beside its left edge, level
                // with its front.
                Handle {
                    field: Field::Turn,
                    at: v(fx, max.y, fz),
                    axis: Vec3::ZERO,
                    around: Some(v(min.x - core, max.y, min.z)),
                },
            ]
        }
        KIND_STAIRS => turning(shape, &mesh, fy, ay),
        KIND_ARCH => {
            let (inner, outer) = (shape.inner.max(MIN_VALUE), shape.outer);
            vec![
                linear(Field::Outer, v(c.x, fy, fz), ay),
                linear(Field::Inner, v(c.x, min.y + inner, fz), Vec3::Y),
                linear(
                    Field::Depth,
                    v(c.x + (inner + outer.max(inner)) / 2.0, min.y, fz),
                    az,
                ),
            ]
        }
        KIND_CYLINDER | KIND_CONE => vec![
            linear(Field::Radius, v(fx, c.y, c.z), ax),
            linear(Field::Height, v(c.x, fy, c.z), ay),
        ],
        KIND_PLANE => vec![
            linear(Field::Extent, v(fx, fy, c.z), ax),
            linear(Field::Thickness, v(c.x, fy, c.z), ay),
        ],
        KIND_DOOR => {
            let (w, h) = (
                shape.door_width.max(MIN_VALUE),
                shape.door_height.max(MIN_VALUE),
            );
            // The jamb and the lintel-or-sill on the far side too: with the top pinned, a taller
            // opening moves the feet, not the lintel.
            let lintel = match shape.pivot.y > 0.0 {
                true => min.y,
                false => min.y + h,
            };
            vec![
                linear(
                    Field::DoorWidth,
                    v(c.x + ax.x * w / 2.0, min.y + h / 2.0, fz),
                    ax,
                ),
                linear(Field::DoorHeight, v(c.x, lintel, fz), ay),
                linear(Field::Frame, v(fx, min.y + h / 2.0, fz), ax),
                linear(Field::Depth, v(c.x, max.y, fz), az),
            ]
        }
        _ => Vec::new(),
    }
}

/// A turning flight's handles: its height, its inner radius at the first step, and its turn at the
/// last step's outer top corner, dragged around the axis.
fn turning(
    shape: &BlockShape,
    mesh: &kooch_blockmesh::BlockMesh,
    fy: f32,
    ay: Vec3,
) -> Vec<Handle> {
    // The pivot's shift, read off the one vertex both meshes share, so the axis and the corners here
    // land where the pivoted geometry drew them.
    let raw = shape.shape().build();
    let (Some(pivoted), Some(unpivoted)) = (mesh.positions().first(), raw.positions().first())
    else {
        return Vec::new();
    };
    let offset = *pivoted - *unpivoted;
    let (min, max) = bounds(mesh.positions(), *pivoted);
    let c = (min + max) / 2.0;
    let sweep = shape.turn.to_radians();
    let inner = shape.core.max(MIN_VALUE);
    let outer = inner + shape.width.max(MIN_VALUE);
    let rise = shape.rise.max(MIN_VALUE);
    vec![
        Handle {
            field: Field::Rise,
            at: Vec3::new(c.x, fy, c.z),
            axis: ay,
            around: None,
        },
        core_handle(shape, offset, inner, sweep),
        Handle {
            field: Field::Turn,
            at: offset + Vec3::new(outer * sweep.cos(), rise, outer * sweep.sin()),
            axis: Vec3::ZERO,
            around: Some(offset + Vec3::new(0.0, rise, 0.0)),
        },
    ]
}

/// The core handle, on the inner edge at whichever of the first, middle or last step moves most when
/// the core grows — the pivot pins one side, and a handle on that side would not move.
fn core_handle(shape: &BlockShape, offset: Vec3, inner: f32, sweep: f32) -> Handle {
    const STEP: f32 = 0.1;
    let grown = BlockShape {
        core: inner + STEP,
        ..*shape
    };
    let grown_offset = match (
        grown.build().positions().first(),
        grown.shape().build().positions().first(),
    ) {
        (Some(pivoted), Some(raw)) => *pivoted - *raw,
        _ => offset,
    };
    let (angle, _) = [0.0, sweep / 2.0, sweep]
        .into_iter()
        .map(|angle| {
            let radial = Vec3::new(angle.cos(), 0.0, angle.sin());
            let moved = (grown_offset + radial * (inner + STEP)) - (offset + radial * inner);
            (angle, moved.dot(radial).abs())
        })
        .fold((0.0, -1.0), |best, candidate| {
            if candidate.1 > best.1 {
                candidate
            } else {
                best
            }
        });
    let radial = Vec3::new(angle.cos(), 0.0, angle.sin());
    Handle {
        field: Field::Core,
        at: offset + radial * inner,
        axis: radial,
        around: None,
    }
}

fn bounds(positions: &[Vec3], first: Vec3) -> (Vec3, Vec3) {
    positions
        .iter()
        .fold((first, first), |(min, max), p| (min.min(*p), max.max(*p)))
}

/// Local units a handle moves per unit of its field. Measured rather than derived, so every kind
/// and every pivot answers the same way; `None` when the field does not move its handle.
fn slope_of(shape: &BlockShape, field: Field) -> Option<f32> {
    const STEP: f32 = 0.1;
    let find = |s: &BlockShape| handles(s).into_iter().find(|h| h.field == field);
    let before = find(shape).filter(|h| h.around.is_none())?;
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
            let colour = match (hovered == Some(handle.field), handle.axis.abs()) {
                (true, _) => Vec4::new(1.0, 0.85, 0.1, 1.0),
                (false, axis) if axis == Vec3::X => Vec4::new(0.9, 0.25, 0.25, 1.0),
                (false, axis) if axis == Vec3::Y => Vec4::new(0.3, 0.85, 0.3, 1.0),
                (false, axis) if axis == Vec3::Z => Vec4::new(0.3, 0.5, 1.0, 1.0),
                // Around an axis: white, with a line to the axis so what it turns around is visible.
                _ => Vec4::new(0.95, 0.95, 0.95, 1.0),
            };
            if let Some(centre) = handle.around {
                let axis = transform.matrix.transform_point3(centre);
                gizmos.line(
                    axis,
                    transform.matrix.transform_point3(handle.at),
                    Vec3::splat(0.95),
                );
            }
            let at = transform.matrix.transform_point3(handle.at);
            gizmos.filled_aabb(at, Vec3::splat(HANDLE_SIZE), colour);
        }
    }
}

mod drag;

use super::active_camera;
#[cfg(test)]
use drag::closest_along;
pub(crate) use drag::{ShapeHandleState, apply_shape_handles};

#[cfg(test)]
mod tests;
