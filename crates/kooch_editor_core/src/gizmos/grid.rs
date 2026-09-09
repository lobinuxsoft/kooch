//! The grid the handles snap to, drawn so the snap can be aimed.

use glam::Vec3;
use kooch_core::resource::Resources;
use kooch_gizmos::{GizmoBatch, Gizmos, MeshBatch};
use kooch_gizmos_handles::HandleSet;

/// How many cells reach out from the centre in each direction.
///
/// Fixed, so the cost does not depend on where the camera is. The grid
/// follows the view instead of tiling to the horizon, which is what
/// stops it aliasing into moiré at grazing angles.
const REACH: i32 = 40;

/// A darker line every this many, so the eye counts without tracing
/// single cells.
const COARSE: i32 = 10;

/// One segment of a grid, and how strongly to draw it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GridLine {
    pub from: Vec3,
    pub to: Vec3,
    /// 1.0 at the centre, 0.0 at the edge of the reach.
    pub shade: f32,
    /// Whether this is one of the counting lines.
    pub coarse: bool,
}

/// Where the grid centres itself: the point, snapped to the step.
///
/// Snapped so the lines sit on multiples of the step rather than on
/// wherever the camera happens to be. A grid whose lines are not on the
/// values the handles snap to is decoration that lies.
pub(crate) fn centre_on(at: Vec3, step: f32) -> Vec3 {
    if step <= 0.0 {
        return at;
    }
    (at / step).round() * step
}

/// How strongly a line that far from the centre is drawn.
///
/// Squared rather than linear: a linear ramp leaves the far half of the
/// grid at half strength, which is still a wall of lines behind whatever
/// is being built.
pub(crate) fn shade_at(distance: f32, reach: f32) -> f32 {
    if reach <= 0.0 {
        return 0.0;
    }
    let near = (1.0 - (distance / reach).clamp(0.0, 1.0)).max(0.0);
    near * near
}

/// The lines of a grid on the plane spanned by `across` and `along`.
///
/// Takes the two axes rather than a normal: a plane has infinitely many
/// rotations about its normal, and the caller knows which one it means —
/// world for the ground, the drag's own for a guide.
pub(crate) fn lines(
    centre: Vec3,
    across: Vec3,
    along: Vec3,
    step: f32,
) -> impl Iterator<Item = GridLine> {
    let reach = REACH as f32 * step;
    let span = along * reach;

    // Both directions from one loop: line `n` across, then line `n`
    // along, so a caller drawing them in order gets a lattice rather
    // than one axis and then the other.
    (-REACH..=REACH).flat_map(move |n| {
        let offset = n as f32 * step;
        let shade = shade_at(offset.abs(), reach);
        let coarse = n % COARSE == 0;
        [
            GridLine {
                from: centre + across * offset - span,
                to: centre + across * offset + span,
                shade,
                coarse,
            },
            GridLine {
                from: centre + along * offset - across * reach,
                to: centre + along * offset + across * reach,
                shade,
                coarse,
            },
        ]
    })
}

/// The cell colour at full strength, and what it fades toward.
const CELL: Vec3 = Vec3::new(0.32, 0.32, 0.36);
const COUNTING: Vec3 = Vec3::new(0.46, 0.46, 0.52);
/// The viewport's own background. Fading toward it rather than to black
/// is what makes a line disappear instead of turning into a dark one.
const BACKDROP: Vec3 = Vec3::new(0.16, 0.16, 0.18);
/// Where the world's axes cross, in the colours every editor uses.
const AXIS_X: Vec3 = Vec3::new(0.75, 0.25, 0.28);
const AXIS_Z: Vec3 = Vec3::new(0.25, 0.42, 0.78);
/// The guide's own colour — warmer, so it does not read as the ground
/// having moved.
const GUIDE: Vec3 = Vec3::new(0.52, 0.44, 0.30);

/// Draws the world grid on the ground plane.
///
/// 🔴 Called before the selection gate, like the physics overlay: it
/// describes where the world IS, and that question is asked precisely
/// when nothing is selected.
///
/// Centred on the camera's focus rather than on the origin, so it is
/// under whatever is being built instead of under the middle of a level
/// somebody has walked away from.
pub(crate) fn draw_world(resources: &Resources, batch: &mut GizmoBatch) {
    let visible = resources
        .get::<super::GizmoVisibility>()
        .is_none_or(|visibility| visibility.enabled && visibility.grid);
    if !visible || playing(resources) {
        return;
    }

    let step = resources
        .get::<crate::state::EditorOverlay>()
        .map(|overlay| overlay.snap_settings.translate)
        .unwrap_or(0.5);
    if step <= 0.0 {
        return;
    }

    // The ground, at the origin's height. Not at the camera's: a grid
    // that rides up with the view answers no question about height.
    let focus = resources
        .get::<crate::editor_camera::EditorCameraController>()
        .map(|controller| controller.focus_point)
        .unwrap_or(Vec3::ZERO);
    let centre = centre_on(Vec3::new(focus.x, 0.0, focus.z), step);

    // A mesh batch it never writes to: the grid is lines only, and
    // `Gizmos` wants both halves.
    let mut meshes = MeshBatch::default();
    let mut gizmos = Gizmos::new(batch, &mut meshes);
    for line in lines(centre, Vec3::X, Vec3::Z, step) {
        let base = match line.coarse {
            true => COUNTING,
            false => CELL,
        };
        gizmos.line(line.from, line.to, BACKDROP.lerp(base, line.shade));
    }

    // The axes on top, so they read through the cells crossing them.
    let reach = REACH as f32 * step;
    gizmos.line(Vec3::NEG_X * reach, Vec3::X * reach, AXIS_X);
    gizmos.line(Vec3::NEG_Z * reach, Vec3::Z * reach, AXIS_Z);
}

/// Draws a guide grid at the height of whatever is being dragged.
///
/// # Why a second grid, and why at that height
///
/// The world grid answers *where is the ground*. It cannot answer
/// *where is this*, because a wall being built eight metres up has
/// nothing under it to judge against — the ground is a plane you are
/// looking down at, not one you are working on.
///
/// So this one is the same lattice at the selection's own height. Two
/// grids, not one that moves: a reference that swings every time
/// somebody grabs a handle has stopped being a reference.
///
/// # Only while dragging
///
/// It answers a question that is only being asked mid-gesture. Left up
/// afterwards it is a second horizon competing with the real one.
pub(crate) fn draw_guide(resources: &Resources, batch: &mut GizmoBatch) {
    let visible = resources
        .get::<super::GizmoVisibility>()
        .is_none_or(|visibility| visibility.enabled && visibility.grid);
    if !visible || playing(resources) {
        return;
    }

    let Some(origin) = resources
        .get::<HandleSet>()
        .filter(|handles| handles.is_dragging())
        .map(|handles| handles.origin())
    else {
        return;
    };
    // Already on the ground: the world grid is drawing this exact
    // lattice, and two of them at one height is a moiré.
    if origin.y.abs() < 1e-3 {
        return;
    }

    let step = resources
        .get::<crate::state::EditorOverlay>()
        .map(|overlay| overlay.snap_settings.translate)
        .unwrap_or(0.5);
    if step <= 0.0 {
        return;
    }

    let centre = centre_on(Vec3::new(origin.x, origin.y, origin.z), step);
    let mut meshes = MeshBatch::default();
    let mut gizmos = Gizmos::new(batch, &mut meshes);
    for line in lines(centre, Vec3::X, Vec3::Z, step) {
        // Dimmer than the world's, and only the counting lines: at a
        // height nobody is looking straight down at, a full lattice is
        // noise over the thing being moved.
        if line.coarse {
            gizmos.line(line.from, line.to, BACKDROP.lerp(GUIDE, line.shade));
        }
    }
}

/// Whether the project is running, in which case nothing is being
/// authored and nothing should offer to help.
fn playing(resources: &Resources) -> bool {
    resources
        .get::<crate::remote_session::RemoteState>()
        .is_some_and(|state| state.playing)
}

#[cfg(test)]
mod tests;
