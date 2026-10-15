//! The transform gizmo's side of a knot (#1441, #1469): where it stands, what it offers, and what
//! a drag on it writes.
//!
//! 🔴 A knot accepts a MOVE on all three axes and a ROTATE on exactly one — its `roll`, about the
//! curve's tangent. Scale it accepts on none: a knot has no size until #1447 gives it a radius.
//! The declaration lives here, next to the code that applies it, so a capability cannot be
//! spelled in two places and disagree with itself.

use glam::{Mat3, Quat, Vec3};

use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentNames;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::Reflect;
use kooch_ecs::spline::{Spline, eval};
use kooch_gizmos_handles::AxisMask;

use super::{SplineHandleState, inverse_of, read, write};
use crate::actions::EditorAction;

/// What a selected knot offers, per mode: Translate, Rotate, Scale. `None` when no knot of
/// `entity` is selected, which leaves the entity's own offer in place.
pub(crate) fn accepts(resources: &Resources, entity: Entity) -> Option<[AxisMask; 3]> {
    selected_origin(resources, entity)?;
    // X alone for the rotate: the basis below puts world X on the tangent, so the one ring the
    // author sees is the one number a knot can turn.
    Some([AxisMask::ALL, AxisMask::X, AxisMask::NONE])
}

/// The frame the knot's handles are drawn in: world +X along the curve's tangent, +Y along its up.
///
/// 🔴 This is what turns the generic X ring into the roll ring. Drawing it about a world axis would
/// put the ring at an angle to the rotation it performs, and an author cannot aim a gizmo that
/// lies about its own axis.
pub(crate) fn selected_basis(resources: &Resources, entity: Entity) -> Option<Mat3> {
    let (tangent, up) = frame(resources, entity)?;
    Some(Mat3::from_cols(tangent, up, tangent.cross(up)))
}

/// The curve's tangent and up at the selected knot, in world space.
fn frame(resources: &Resources, entity: Entity) -> Option<(Vec3, Vec3)> {
    let (held, index) = resources.get::<SplineHandleState>()?.selected?;
    if held != entity {
        return None;
    }
    let (spline, to_world) = read(resources, entity)?;
    let segments = eval::segments(&spline.points, spline.closed);
    if segments == 0 {
        return None;
    }
    let t = index as f32 / segments as f32;
    let tangent = to_world
        .transform_vector3(eval::tangent(&spline.points, spline.closed, t))
        .normalize_or_zero();
    let up = to_world
        .transform_vector3(eval::up(&spline.points, spline.closed, t))
        .normalize_or_zero();
    // A degenerate curve answers with nothing usable, and a basis built from a zero column is a
    // basis that collapses every handle onto a point.
    if !(tangent.length_squared() > 0.5) || !(up.length_squared() > 0.5) {
        return None;
    }
    Some((tangent, up.reject_from(tangent).normalize_or_zero()))
}

/// Turns the selected knot's `roll` by a gizmo rotation, in degrees. Returns whether it wrote.
///
/// The gizmo hands back a quaternion about the tangent, which is one number wearing four: the
/// signed angle about that same axis is what `roll` is measured in.
pub(crate) fn roll_selected(resources: &mut Resources, entity: Entity, by: Quat) -> bool {
    let Some((tangent, _)) = frame(resources, entity) else {
        return false;
    };
    let Some((held, index)) = resources
        .get::<SplineHandleState>()
        .and_then(|state| state.selected)
    else {
        return false;
    };
    if held != entity {
        return false;
    }
    let degrees = about(by, tangent).to_degrees();
    if !(degrees.abs() > 0.0) {
        return false;
    }
    let Some((mut spline, _)) = read(resources, entity) else {
        return false;
    };
    let Some(knot) = spline.points.get_mut(index) else {
        return false;
    };
    // Clamped to the range the Inspector's slider offers, so the two ways to write this field
    // cannot disagree about what a legal value is.
    knot.roll = (knot.roll + degrees).clamp(ROLL_MIN, ROLL_MAX);
    write(resources, entity, spline);
    true
}

/// Matches `Knot::roll`'s declared range.
const ROLL_MIN: f32 = -180.0;
const ROLL_MAX: f32 = 180.0;

/// The signed angle `q` turns about `axis`.
///
/// `to_axis_angle` reports an unsigned angle about an axis it may have flipped, which loses the
/// direction a drag went. Taking it from the quaternion's own components keeps the sign.
fn about(q: Quat, axis: Vec3) -> f32 {
    2.0 * Vec3::new(q.x, q.y, q.z).dot(axis).atan2(q.w)
}

/// Where the selected knot is, in world space — the point the transform gizmo stands on (#1441).
///
/// `None` unless a knot of `entity` is selected and still exists: a knot removed from the list
/// leaves an index pointing at nothing, and a gizmo floating over a deleted point is a gizmo that
/// edits whatever took its place.
pub(crate) fn selected_origin(resources: &Resources, entity: Entity) -> Option<Vec3> {
    let (held, index) = resources.get::<SplineHandleState>()?.selected?;
    if held != entity {
        return None;
    }
    let (spline, to_world) = read(resources, entity)?;
    Some(to_world.transform_point3(spline.points.get(index)?.position))
}

/// Moves the selected knot by a world-space translation, in the spline's own space.
///
/// 🔴 Translation only. A knot is a position: it has no rotation to turn and no scale to grow, and
/// letting those fall through to the entity would move the whole spline while the gizmo claimed to
/// be editing one point of it.
pub(crate) fn translate_selected(resources: &mut Resources, entity: Entity, by: Vec3) -> bool {
    let Some((held, index)) = resources
        .get::<SplineHandleState>()
        .and_then(|state| state.selected)
    else {
        return false;
    };
    if held != entity {
        return false;
    }
    let Some(to_local) = inverse_of(resources, entity) else {
        return false;
    };
    // A direction, not a point: the entity's own translation must not be added to the delta.
    let local = to_local.transform_vector3(by);
    let Some((mut spline, _)) = read(resources, entity) else {
        return false;
    };
    let Some(knot) = spline.points.get_mut(index) else {
        return false;
    };
    knot.position += local;
    write(resources, entity, spline);
    true
}

/// Records what a gizmo drag on the selected knot began from. Call on the frame the drag starts.
pub(crate) fn began_gizmo_drag(resources: &mut Resources) {
    let Some(entity) = resources
        .get::<SplineHandleState>()
        .and_then(|state| state.selected)
        .map(|(entity, _)| entity)
    else {
        return;
    };
    let started = read(resources, entity).map(|(spline, _)| spline);
    if let Some(state) = resources.get_mut::<SplineHandleState>() {
        state.gizmo_start = started;
    }
}

/// Emits the edit for a finished gizmo drag, if the knot actually moved.
pub(crate) fn ended_gizmo_drag(resources: &mut Resources, actions: &mut Vec<EditorAction>) {
    let Some((entity, before)) = resources.get_mut::<SplineHandleState>().and_then(|state| {
        let entity = state.selected.map(|(entity, _)| entity)?;
        Some((entity, state.gizmo_start.take()?))
    }) else {
        return;
    };
    let Some(after) = read(resources, entity).map(|(spline, _)| spline) else {
        return;
    };
    // Clicking a handle without moving it is a click, not an edit, and a history of no-ops is what
    // makes undo untrustworthy.
    if after.points == before.points {
        return;
    }
    let Some(component) = resources
        .get::<ComponentNames>()
        .and_then(|names| names.id(std::any::type_name::<Spline>()))
    else {
        return;
    };
    let Some(value) = after.reflect_get("points") else {
        return;
    };
    // Put the start back, so the command records the value the drag began from and the whole
    // gesture is one undo step — the same shape the grip drag follows.
    write(resources, entity, before);
    actions.push(EditorAction::SetField {
        entity,
        component,
        field: "points".to_owned(),
        value,
    });
}
