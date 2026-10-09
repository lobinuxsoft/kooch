//! Landing a drag on the geometry under the cursor instead of on a plane or an axis (#1435).
//!
//! 🔴 One predicate and one query, shared by every consumer. The gesture is the same whether what
//! moves is an entity, a block face or a spline knot, and three copies of "is the modifier held"
//! are three chances for one of them to disagree — the shape that produced four ordering bugs in
//! #1261.
//!
//! The snap is **absolute**: the dragged thing goes *to* the hit, not *by* the cursor's movement.
//! That is the whole point of the gesture — a knot authored over a level has no depth until the
//! scene gives it one.

use glam::Vec3;

use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;
use kooch_gizmos::Gizmos;

use crate::editor_camera::input::ViewportInputDelta;

/// Colour of the mark drawn where a drag would land. Warm, like every other thing a hand moves.
const MARK: Vec3 = Vec3::new(1.0, 0.65, 0.2);

/// The mark's radius, in screen-scale reference units, so it reads the same at any distance.
const MARK_UNITS: f32 = 0.09;

/// How far the normal is drawn out of the surface, in the same units.
const NORMAL_UNITS: f32 = 0.3;

/// Where the last engaged drag would land. Written while input is applied, read when the gizmo
/// batch is built.
///
/// ⚠️ Those are two different stages — the batch is built in `PreRender` and input lands in
/// `Render` — so the mark trails the cursor by one frame, exactly as the handles themselves do.
/// During a drag the frames are continuous and it does not read as lag.
#[derive(Default)]
pub(crate) struct SurfaceSnapState {
    pub(crate) hit: Option<(Vec3, Vec3)>,
}

/// Whether this frame's input asks for a surface snap.
///
/// **Ctrl+Shift**, as Unity binds it. Ctrl alone is already grid snap everywhere in this editor,
/// Shift alone pans the camera and extrudes, and Alt is the window-drag modifier on half the
/// desktops this runs on.
pub(crate) fn engaged(delta: ViewportInputDelta) -> bool {
    delta.ctrl_held && delta.shift_held
}

/// Where the cursor lands on the scene, and which way that surface faces.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SnapHit {
    pub(crate) point: Vec3,
    /// Unit world-space normal of the surface struck.
    pub(crate) normal: Vec3,
}

/// Where the cursor lands on the scene this frame, ignoring `dragged`.
///
/// Returns `None` when the modifier is not held, the cursor is off the viewport, or nothing is
/// under it — and in that last case the caller keeps its ordinary drag, so a snap over empty sky
/// leaves the gesture alone rather than teleporting it.
pub(crate) fn target(
    resources: &mut Resources,
    delta: ViewportInputDelta,
    dragged: Entity,
) -> Option<SnapHit> {
    if !engaged(delta) {
        return None;
    }
    let Some(cursor) = delta.cursor_local else {
        return None;
    };
    // The dragged entity AND its children: a model's mesh usually sits on a child, and a snap that
    // can see its own geometry sticks to it. The same walk `rest_on` measures, so what is excluded
    // and what is weighed are the same set.
    let skip = subtree(resources, dragged);
    let hit = crate::picking::surface_at(resources, cursor, delta.viewport_size, &skip);
    record(resources, hit.map(|hit| (hit.point, hit.normal)));
    hit.map(|hit| SnapHit {
        point: hit.point,
        normal: hit.normal,
    })
}

/// Where `entity`'s pivot goes so the model RESTS on the surface instead of sinking into it.
///
/// 🔴 A pivot on the floor is not a model on the floor. Put a sphere's centre on the ground and
/// half the sphere is underground — which is the whole of what makes a snap feel wrong even when
/// the hit is exact.
///
/// The measure is the entity's world bounding box: how far its deepest corner reaches past the
/// pivot, against the surface normal. Cheap and never wrong in the direction that matters — the
/// box contains the model, so resting the box never buries the model. On a slope it rests on a
/// corner and leaves a gap, which is the price of a box and the right price for a blockout.
///
/// No mesh, no volume, no lift: a spline's entity or an empty gets its pivot on the surface,
/// which for a thing with no extent is the same answer.
///
/// 🔴 Children count. An imported model usually carries its mesh on a child, and the raycast
/// already skips the whole subtree — measuring only the parent would leave the two halves of one
/// gesture disagreeing about what is being dragged, and a glTF prop would never lift at all.
pub(crate) fn rest_on(resources: &mut Resources, entity: Entity, hit: SnapHit) -> Vec3 {
    let Some(pivot) = crate::gizmos::entity_world_position(resources, entity) else {
        return hit.point;
    };

    let mut deepest = 0.0f32;
    let mut found = false;
    for part in subtree(resources, entity) {
        let Some((min, max)) = crate::picking::entity_bounds(resources, part) else {
            continue;
        };
        found = true;
        for index in 0..8u32 {
            let corner = Vec3::new(
                if index & 1 == 0 { min.x } else { max.x },
                if index & 2 == 0 { min.y } else { max.y },
                if index & 4 == 0 { min.z } else { max.z },
            );
            // How far this corner hangs below the pivot, measured along the normal. Negative for
            // every corner above it, so a pivot already at the model's base lifts by nothing.
            deepest = deepest.max((pivot - corner).dot(hit.normal));
        }
    }
    match found {
        true => hit.point + hit.normal * deepest,
        false => hit.point,
    }
}

/// An entity and everything parented under it.
fn subtree(resources: &Resources, entity: Entity) -> Vec<Entity> {
    match resources.get::<kooch_ecs::component::ComponentRegistry>() {
        Some(registry) => kooch_ecs::hierarchy::collect_descendants(entity, registry),
        None => vec![entity],
    }
}

/// Forgets the last hit.
///
/// 🔴 Called ONCE per frame, before any handle runs, and never by a consumer. The three handle
/// systems are chained with `||` and short-circuit each other, so every one of them is a path
/// where the one that would have cleared the mark never ran — and the mark would sit on screen
/// over a drag that had already ended.
pub(crate) fn clear(resources: &mut Resources) {
    record(resources, None);
}

fn record(resources: &mut Resources, hit: Option<(Vec3, Vec3)>) {
    if let Some(state) = resources.get_mut::<SurfaceSnapState>() {
        state.hit = hit;
        return;
    }
    resources.insert(SurfaceSnapState { hit });
}

/// Draws where the drag would land and which way that surface faces.
///
/// The normal is drawn even though nothing is aligned to it yet: it is what says *which* surface
/// answered, and a mark on a wall and a mark on the floor are otherwise the same dot.
pub(crate) fn draw(resources: &Resources, gizmos: &mut Gizmos<'_>) {
    let Some((at, normal)) = resources
        .get::<SurfaceSnapState>()
        .and_then(|state| state.hit)
    else {
        return;
    };
    let scale = crate::gizmos::screen_scale::factor(resources, at);
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        gizmos.line(
            at - axis * MARK_UNITS * scale,
            at + axis * MARK_UNITS * scale,
            MARK,
        );
    }
    gizmos.arrow(at, at + normal * NORMAL_UNITS * scale, MARK.extend(1.0));
}
