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
use kooch_ecs::spline::{Knot, Spline, TANGENT_ALIGNED, TANGENT_AUTO, TANGENT_BROKEN, eval};
use kooch_gizmos_handles::SnapSettings;

use crate::actions::EditorAction;
use crate::editor_camera::input::ViewportInputDelta;

/// Half the side of a grip's cube, in world units. Matches `shape_handles` so one viewport does not
/// have two sizes of the same affordance.
pub(super) const GRIP_SIZE: f32 = 0.05;

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
            if let Some((origin, direction)) = ray
                && let Some(hit) = on_plane(drag.grab, drag.plane, origin, direction)
            {
                let Some(to_local) = inverse_of(resources, entity) else {
                    return true;
                };
                // The grip moves by what the cursor moved, not to where the cursor is.
                let local = to_local.transform_point3(drag.origin + (hit - drag.grab));
                let local = match delta.ctrl_held {
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
                (along > 0.0 && off <= along * PICK_SLOPE + GRIP_SIZE)
                    .then_some((along, grip, world))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let Some((_, grip, world)) = under else {
            continue;
        };
        state.hovered = Some((entity, grip));
        if delta.lmb_pressed {
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
    false
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
pub(super) fn grip_colour(hovered: bool, part: Part) -> Vec4 {
    match (hovered, part) {
        (true, _) => Vec4::new(1.0, 0.85, 0.1, 1.0),
        (false, Part::Knot) => Vec4::new(0.95, 0.95, 0.95, 1.0),
        (false, _) => Vec4::new(1.0, 0.6, 0.15, 1.0),
    }
}

#[cfg(test)]
mod tests;
