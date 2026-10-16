//! Handles that drag a [`Spline`]'s knots and tangents in the viewport (#1261).
//!
//! 🔴 Adding, removing and reordering knots are NOT here: the Inspector's list widget already does
//! all three for any reflected list (#1201), and returns the whole list so an edit is one
//! `SetField`. A second way to do it in the viewport would be a second place for it to go wrong.
//!
//! A drag writes the component live for feedback and emits the edit on release, so one drag is one
//! undo step — the shape `shape_handles` established in #1150.

use glam::{Mat4, Vec3, Vec4};

use kooch_core::resource::Resources;
use kooch_ecs::GlobalTransform;
use kooch_ecs::component::{ComponentNames, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::Reflect;
use kooch_ecs::spline::{
    Knot, RADIUS_MAX, RADIUS_MIN, Spline, TANGENT_ALIGNED, TANGENT_AUTO, TANGENT_BROKEN, eval,
};
use kooch_gizmos_handles::{HandleSet, SnapSettings};

use crate::actions::EditorAction;
use crate::editor_camera::input::ViewportInputDelta;

/// Half the side of a grip's cube, as a share of the screen-scale reference unit — so it holds its
/// size however far the camera is (#1433). Matches `shape_handles` so one viewport does not have two
/// sizes of the same affordance.
///
/// ⚠️ This sizes the cube and the pick tolerance, never [`HANDLE_SCALE`], which is **where** a
/// tangent grip sits: that offset is the value being edited, and scaling it by camera distance
/// would make a drag from far away change the tangent by a different amount.
pub(super) const GRIP_SIZE: f32 = 0.05;

/// The grip's half-size in world units at `at`.
pub(super) fn grip_size(resources: &Resources, at: Vec3) -> f32 {
    GRIP_SIZE * super::screen_scale::factor(resources, at)
}

/// How far off the cursor ray a grip still counts as under it, as a fraction of its distance.
const PICK_SLOPE: f32 = 0.02;

/// A Hermite tangent spans its segment, so a handle is drawn at a third of it. Shared with the
/// visualizer: a grip has to sit exactly where the line is drawn or it cannot be grabbed.
pub(super) const HANDLE_SCALE: f32 = 1.0 / 3.0;

/// Which part of a knot a grip moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Part {
    /// The knot itself.
    Knot,
    /// The handle pointing the way the curve leaves.
    Leaving,
    /// The handle pointing back the way it arrived.
    Arriving,
    /// How thick the curve is here: the grip sits on the knot's ring, a radius out along its up.
    Radius,
}

/// One grabbable point: which knot, and which part of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Grip {
    pub(super) index: usize,
    pub(super) part: Part,
}

#[derive(Default)]
pub(crate) struct SplineHandleState {
    drag: Option<Drag>,
    pub(super) hovered: Option<(Entity, Grip)>,
    /// The knot the transform gizmo acts on, if any (#1441). Set by clicking one.
    pub(super) selected: Option<(Entity, usize)>,
    /// The spline as the gizmo drag found it, so the whole gesture is one undo step.
    gizmo_start: Option<Spline>,
}

struct Drag {
    entity: Entity,
    grip: Grip,
    /// The spline as it was at the press, so the emitted edit records where the drag began and the
    /// whole gesture undoes at once.
    start: Spline,
    /// The plane the drag runs in: through the grip, facing the camera.
    plane: Vec3,
    /// Where the cursor met that plane at the press, in world space.
    grab: Vec3,
    /// Where the grip itself was at the press, in world space. Kept beside `grab` because a press
    /// lands anywhere inside the pick tolerance: moving the grip TO the cursor would jump it by
    /// that slack before the drag even began.
    origin: Vec3,
}

/// Grips of one spline, in its own local space.
pub(super) fn grips(spline: &Spline) -> Vec<(Grip, Vec3)> {
    let count = spline.points.len();
    let mut found = Vec::with_capacity(count * 3);
    for (index, knot) in spline.points.iter().enumerate() {
        found.push((
            Grip {
                index,
                part: Part::Knot,
            },
            knot.position,
        ));
        if count < 2 {
            continue;
        }
        // Read through the evaluator, so a grip sits on the line the gizmo drew rather than on a
        // second derivation of it.
        let last = count - 1;
        if spline.closed || index != last {
            let leaving =
                eval::tangents(&spline.points, spline.closed, index, (index + 1) % count).0;
            found.push((
                Grip {
                    index,
                    part: Part::Leaving,
                },
                knot.position + leaving * HANDLE_SCALE,
            ));
        }
        // On the ring the visualizer draws, which is why both read `ring_frame`: a grip derived a
        // second time would sit beside the circle it is supposed to size.
        // 🔴 `knot.radius > 0.0`: a ring of nothing puts its grip exactly on the knot's, where
        // the picker would hand the author whichever it found first. The Inspector and the scale
        // gizmo are how a radius authored to zero comes back.
        if let Some((up, _)) = ring_frame(spline, index).filter(|_| knot.radius > 0.0) {
            found.push((
                Grip {
                    index,
                    part: Part::Radius,
                },
                knot.position + up * knot.radius,
            ));
        }
        if spline.closed || index != 0 {
            let arriving = -eval::tangents(
                &spline.points,
                spline.closed,
                (index + count - 1) % count,
                index,
            )
            .1;
            found.push((
                Grip {
                    index,
                    part: Part::Arriving,
                },
                knot.position + arriving * HANDLE_SCALE,
            ));
        }
    }
    found
}

/// The plane a knot's radius ring lies in, in the spline's own space: the curve's up there, and
/// the side square to both. `None` where the curve is too degenerate to have a frame.
///
/// ⚠️ One rotation-minimising transport per call, and both the grips and the drawing ask per knot.
/// The first thing to cache if the editor's frame time ever points at splines.
pub(super) fn ring_frame(spline: &Spline, index: usize) -> Option<(Vec3, Vec3)> {
    let segments = eval::segments(&spline.points, spline.closed);
    if segments == 0 {
        return None;
    }
    let t = index as f32 / segments as f32;
    let tangent = eval::tangent(&spline.points, spline.closed, t);
    let up = eval::up(&spline.points, spline.closed, t);
    (tangent.length_squared() > 1e-12 && up.length_squared() > 1e-12)
        .then(|| (up, tangent.cross(up).normalize_or_zero()))
}

/// Where a grip moves the knot to, given the local position the cursor dragged it to.
///
/// 🔴 Dragging an `Auto` tangent promotes the knot to `Aligned`, as Blender and Unity both do:
/// otherwise the handle springs back and the author concludes the editor is broken. `Aligned` moves
/// the pair together — breaking the pair is a mode, chosen in the Inspector, not a side effect of a
/// drag nobody asked to be destructive.
pub(super) fn moved(knot: Knot, part: Part, to: Vec3) -> Knot {
    match part {
        Part::Knot => Knot {
            position: to,
            ..knot
        },
        // 🔴 The distance from the knot, not a signed offset along the ring's up: the drag runs in
        // the plane facing the camera, so dragging the grip anywhere around its circle means the
        // same thickness. Clamped to the field's own range, so a drag and the Inspector's slider
        // cannot disagree about a legal radius.
        Part::Radius => Knot {
            radius: (to - knot.position).length().clamp(RADIUS_MIN, RADIUS_MAX),
            ..knot
        },
        Part::Leaving | Part::Arriving => {
            let vector = (to - knot.position) / HANDLE_SCALE;
            // A handle dragged onto its own knot has no direction left to report; keeping the old
            // one is better than a zero tangent, which flattens the segment to a straight line.
            if vector.length_squared() < 1.0e-12 {
                return knot;
            }
            let mode = match knot.mode {
                TANGENT_AUTO => TANGENT_ALIGNED,
                held => held,
            };
            match (part, mode) {
                // Broken is the only mode where the two are independent.
                (Part::Arriving, TANGENT_BROKEN) => Knot {
                    mode,
                    arriving: vector,
                    ..knot
                },
                // Arriving points back, so moving it along its own direction moves `leaving` the
                // other way.
                (Part::Arriving, _) => Knot {
                    mode,
                    leaving: -vector,
                    ..knot
                },
                (_, _) => Knot {
                    mode,
                    leaving: vector,
                    ..knot
                },
            }
        }
    }
}

/// Snaps a position to `step` on every axis. Zero or less leaves it alone.
fn snapped(at: Vec3, step: f32) -> Vec3 {
    // 🔴 `!(step > 0.0)`, not `step <= 0.0`: a NaN step would pass the `<=` and divide into NaN.
    if !(step > 0.0) {
        return at;
    }
    (at / step).round() * step
}

/// Drives the spline handles for this frame; `true` while one is hovered or dragged, so the
/// transform handles and picking leave the click alone.
pub(crate) fn apply_spline_handles(
    delta: ViewportInputDelta,
    resources: &mut Resources,
    selected: &[Entity],
    snap: SnapSettings,
    actions: &mut Vec<EditorAction>,
) -> bool {
    let mut state = resources.remove::<SplineHandleState>().unwrap_or_default();
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
    state: &mut SplineHandleState,
) -> bool {
    state.hovered = None;
    let ray = cursor_ray(resources, delta);

    if let Some(drag) = state.drag.as_ref() {
        let entity = drag.entity;
        if delta.lmb_held {
            // 🔴 This is the drag the snap was asked for: a grip moves in the plane facing the
            // camera, which has no depth at all, and the scene is the only thing that can supply
            // one (#1435). Absolute — the grip goes TO the surface, it does not move BY the cursor.
            let onto_surface = match drag.grip.part {
                // 🔴 A radius is a length from its own knot, not a place in the scene: landing it
                // on the geometry under the cursor would set the thickness to however far the wall
                // behind it happens to be.
                Part::Radius => None,
                _ => crate::surface_snap::target(resources, delta, entity).map(|hit| hit.point),
            };
            let dragged_to = onto_surface.or_else(|| {
                let (origin, direction) = ray?;
                // The grip moves by what the cursor moved, not to where the cursor is.
                on_plane(drag.grab, drag.plane, origin, direction)
                    .map(|hit| drag.origin + (hit - drag.grab))
            });
            if let Some(world) = dragged_to {
                let Some(to_local) = inverse_of(resources, entity) else {
                    return true;
                };
                let local = to_local.transform_point3(world);
                // 🔴 No grid snap on top of a surface snap: rounding a point that is ON the floor
                // to the nearest half metre lifts it back off. The modifier includes Ctrl, so
                // without this the two snaps fight every frame.
                let local = match delta.ctrl_held && onto_surface.is_none() {
                    true => snapped(local, snap.translate),
                    false => local,
                };
                let mut live = drag.start.clone();
                if let Some(knot) = live.points.get_mut(drag.grip.index) {
                    *knot = moved(*knot, drag.grip.part, local);
                }
                write(resources, entity, live);
            }
            state.hovered = Some((entity, drag.grip));
            return true;
        }
        // Released: put the start back and emit the edit, so the command records the value the drag
        // began from and the whole gesture is one undo step.
        let Some(drag) = state.drag.take() else {
            return true;
        };
        if let Some(live) = read(resources, entity).map(|(spline, _)| spline)
            && live.points != drag.start.points
            && let Some(component) = resources
                .get::<ComponentNames>()
                .and_then(|names| names.id(std::any::type_name::<Spline>()))
            && let Some(value) = live.reflect_get("points")
        {
            write(resources, entity, drag.start);
            // The whole list, because that is what the field is — and what the Inspector's own list
            // edits emit, so both paths undo the same way.
            actions.push(EditorAction::SetField {
                entity,
                component,
                field: "points".to_owned(),
                value,
            });
        }
        return true;
    }

    let Some((origin, direction)) = ray else {
        return false;
    };
    for &entity in selected {
        let Some((spline, to_world)) = read(resources, entity) else {
            continue;
        };
        let under = grips(&spline)
            .into_iter()
            .filter_map(|(grip, local)| {
                let world = to_world.transform_point3(local);
                let along = (world - origin).dot(direction);
                let off = (world - (origin + direction * along)).length();
                // 🔴 The tolerance scales with the drawn size, or the fix makes picking worse: a
                // cube drawn ten times bigger with an unchanged tolerance is visible and ungrabbable.
                let reach = grip_size(resources, world);
                (along > 0.0 && off <= along * PICK_SLOPE + reach).then_some((along, grip, world))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let Some((_, grip, world)) = under else {
            continue;
        };
        state.hovered = Some((entity, grip));
        if delta.lmb_pressed {
            // Clicking a knot also selects it, so the transform gizmo moves there. A tangent is not
            // a position and has no axes worth constraining, so it does not take the selection.
            if grip.part == Part::Knot {
                state.selected = Some((entity, grip.index));
            }
            // Facing the camera, so the grip follows the cursor rather than needing an axis chosen
            // first. Depth comes from orbiting the view, which is the gesture a level designer
            // already has.
            let plane = -direction;
            let Some(grab) = on_plane(world, plane, origin, direction) else {
                return true;
            };
            state.drag = Some(Drag {
                entity,
                grip,
                start: spline,
                plane,
                grab,
                origin: world,
            });
        }
        return true;
    }
    // Reached only when no grip was under the cursor — the loop returns as soon as one is. A click
    // out here gives the gizmo back to the entity; without it a knot stays selected for ever and the
    // entity itself can never be moved again.
    //
    // 🔴 Unless the transform gizmo is the thing being clicked. This runs BEFORE the gizmo in
    // `edits.rs`, and a gizmo handle sits far from the knot it moves, so grabbing an axis looks
    // exactly like clicking empty space from here — it cleared the selection, and the gizmo found
    // nothing to move by the time it ran. Its hover is last frame's, which is enough: the cursor
    // reaches a handle before the button goes down.
    if delta.lmb_pressed && !gizmo_under_cursor(resources) {
        state.selected = None;
    }
    false
}

/// Whether the transform gizmo is hovered or being dragged.
fn gizmo_under_cursor(resources: &Resources) -> bool {
    resources
        .get::<HandleSet>()
        .is_some_and(HandleSet::is_active)
}

/// Where a ray meets the plane through `at` with normal `normal`.
fn on_plane(at: Vec3, normal: Vec3, origin: Vec3, direction: Vec3) -> Option<Vec3> {
    let facing = direction.dot(normal);
    // Edge-on: the ray runs along the plane and meets it everywhere or nowhere.
    if facing.abs() < 1.0e-6 {
        return None;
    }
    Some(origin + direction * ((at - origin).dot(normal) / facing))
}

fn read(resources: &Resources, entity: Entity) -> Option<(Spline, Mat4)> {
    let registry = resources.get::<ComponentRegistry>()?;
    let spline = registry.get_cpu::<Spline>()?.get(entity)?.clone();
    let matrix = registry.get_cpu::<GlobalTransform>()?.get(entity)?.matrix;
    Some((spline, matrix))
}

/// World-to-local for an entity, so a cursor hit becomes a knot position.
fn inverse_of(resources: &Resources, entity: Entity) -> Option<Mat4> {
    let matrix = resources
        .get::<ComponentRegistry>()?
        .get_cpu::<GlobalTransform>()?
        .get(entity)?
        .matrix;
    // A degenerate transform — a zero scale on some axis — has no inverse, and inverting it yields
    // NaN that would land the knot nowhere.
    (matrix.determinant().abs() > 1.0e-12).then(|| matrix.inverse())
}

fn write(resources: &mut Resources, entity: Entity, spline: Spline) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<Spline>()
        && let Some(held) = storage.get_mut(entity)
    {
        *held = spline;
    }
}

fn cursor_ray(resources: &Resources, delta: ViewportInputDelta) -> Option<(Vec3, Vec3)> {
    super::shape_handles::cursor_ray(resources, delta)
}

/// The colour a grip draws in: the hovered one warm, the rest by what they are.
pub(super) fn grip_colour(hovered: bool, selected: bool, part: Part) -> Vec4 {
    match (hovered, selected, part) {
        (true, _, _) => Vec4::new(1.0, 0.85, 0.1, 1.0),
        // The one the transform gizmo is standing on, so it is clear which point the axes move.
        (false, true, _) => Vec4::new(0.3, 0.9, 1.0, 1.0),
        (false, false, Part::Knot) => Vec4::new(0.95, 0.95, 0.95, 1.0),
        // The ring's own yellow, so the grip reads as part of the circle it sizes rather than as a
        // third tangent handle.
        (false, false, Part::Radius) => RING_GRIP,
        (false, false, _) => Vec4::new(1.0, 0.6, 0.15, 1.0),
    }
}

/// The radius grip at rest. Dimmer than the hover yellow above, which is how an author sees which
/// of the two they are about to grab.
const RING_GRIP: Vec4 = Vec4::new(0.85, 0.72, 0.15, 1.0);

/// Whether a grip is being dragged right now. What tells a live edit from a finished one: a drag
/// writes the component every frame, and anything expensive hanging off that wants to wait.
pub(crate) fn is_dragging(resources: &Resources) -> bool {
    resources
        .get::<SplineHandleState>()
        .is_some_and(|state| state.drag.is_some())
}

/// The knot the gizmo is on, for the visualizer to mark.
pub(super) fn selected_index(resources: &Resources, entity: Entity) -> Option<usize> {
    let (held, index) = resources.get::<SplineHandleState>()?.selected?;
    (held == entity).then_some(index)
}

mod gizmo;

pub(crate) use gizmo::{
    accepts, began_gizmo_drag, ended_gizmo_drag, roll_selected, scale_selected, selected_basis,
    selected_origin, translate_selected,
};

#[cfg(test)]
mod tests;

/// Puts a knot where the cursor points, at the end of the curve (#1436).
///
/// 🔴 The SAME `SetField` on `points` the Inspector's list emits. The Inspector is the exhaustive,
/// precise path and this is the fast one; the moment they write the component by two different
/// routes, one of them becomes a second source of truth and undo stops describing either.
///
/// Appended rather than inserted beside whatever is selected: a path is drawn by extending it, and
/// one command that sometimes appends and sometimes inserts is one nobody can predict.
///
/// Where it lands is [`crate::viewport_pick::resolve`] — the geometry under the cursor, then the
/// ground plane, then the camera's distance. The same resolver a spawn and a prefab drop use, so
/// three gestures that mean "here" cannot disagree about where "here" is.
pub(crate) fn add_knot(
    resources: &mut Resources,
    delta: ViewportInputDelta,
    entity: Entity,
    actions: &mut Vec<EditorAction>,
) -> bool {
    let Some(cursor) = delta.cursor_local else {
        return false;
    };
    let Some(before) = read(resources, entity).map(|(spline, _)| spline) else {
        return false;
    };
    let Some(world) = crate::viewport_pick::resolve(
        resources,
        crate::viewport_pick::DropPoint::Viewport {
            cursor,
            viewport_size: delta.viewport_size,
        },
    ) else {
        return false;
    };
    let Some(to_local) = inverse_of(resources, entity) else {
        return false;
    };

    let mut after = before.clone();
    after.points.push(kooch_ecs::spline::Knot {
        position: to_local.transform_point3(world),
        ..Default::default()
    });

    let Some(component) = resources
        .get::<ComponentNames>()
        .and_then(|names| names.id(std::any::type_name::<Spline>()))
    else {
        return false;
    };
    let Some(value) = after.reflect_get("points") else {
        return false;
    };
    // The component is left holding `before` — never written here — so the command carries the
    // whole change and one Ctrl+Z takes the knot back off.
    actions.push(EditorAction::SetField {
        entity,
        component,
        field: "points".to_owned(),
        value,
    });
    true
}
