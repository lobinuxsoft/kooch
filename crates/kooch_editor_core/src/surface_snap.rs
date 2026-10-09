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

/// Where the cursor lands on the scene this frame, ignoring `dragged`.
///
/// Returns `None` when the modifier is not held, the cursor is off the viewport, or nothing is
/// under it — and in that last case the caller keeps its ordinary drag, so a snap over empty sky
/// leaves the gesture alone rather than teleporting it.
pub(crate) fn target(
    resources: &mut Resources,
    delta: ViewportInputDelta,
    dragged: Entity,
) -> Option<Vec3> {
    if !engaged(delta) {
        return None;
    }
    let Some(cursor) = delta.cursor_local else {
        return None;
    };
    // The dragged entity AND its children: a model's mesh usually sits on a child, and a snap
    // that can see its own geometry sticks to it.
    let skip = match resources.get::<kooch_ecs::component::ComponentRegistry>() {
        Some(registry) => kooch_ecs::hierarchy::collect_descendants(dragged, registry),
        None => vec![dragged],
    };
    let hit = crate::picking::surface_at(resources, cursor, delta.viewport_size, &skip);
    record(resources, hit.map(|hit| (hit.point, hit.normal)));
    hit.map(|hit| hit.point)
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
