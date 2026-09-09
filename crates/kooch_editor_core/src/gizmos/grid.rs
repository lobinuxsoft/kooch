//! Which grid planes the viewport should draw this frame.

use glam::Vec3;
use kooch_core::resource::Resources;
use kooch_gizmos::GridPlane;
use kooch_gizmos_handles::HandleSet;

/// The ground, in the grey every block-out tool uses.
const CELL: Vec3 = Vec3::new(0.34, 0.34, 0.38);
const COUNTING: Vec3 = Vec3::new(0.52, 0.52, 0.58);
/// The guide, warmer, so it does not read as the ground having moved.
const GUIDE_CELL: Vec3 = Vec3::new(0.44, 0.38, 0.28);
const GUIDE_COUNTING: Vec3 = Vec3::new(0.60, 0.52, 0.36);

/// The planes to draw: the world's, and the one being worked on.
///
/// # Two grids, because they answer different questions
///
/// The world grid says **where the ground is**. It does not move with a
/// drag, since a reference that swings every time somebody grabs a
/// handle has stopped being one.
///
/// The guide says **where this is**. A wall being built eight metres up
/// has nothing under it to judge against — the ground is a plane you
/// are looking down at, not one you are working on. It appears at the
/// selection's own height while a handle is held, and goes with the
/// gesture: left up afterwards it is a second horizon competing with
/// the real one.
pub(crate) fn grid_planes(resources: &Resources) -> Vec<GridPlane> {
    let visible = resources
        .get::<super::GizmoVisibility>()
        .is_none_or(|visibility| visibility.enabled && visibility.grid);
    let playing = resources
        .get::<crate::remote_session::RemoteState>()
        .is_some_and(|state| state.playing);
    if !visible || playing {
        return Vec::new();
    }

    let step = resources
        .get::<crate::state::EditorOverlay>()
        .map(|overlay| overlay.snap_settings.translate)
        .unwrap_or(0.5);
    if step <= 0.0 {
        return Vec::new();
    }

    let mut planes = vec![GridPlane {
        height: 0.0,
        step,
        cell: CELL,
        counting: COUNTING,
        axes: true,
        scales: true,
    }];

    if let Some(origin) = resources
        .get::<HandleSet>()
        .filter(|handles| handles.is_dragging())
        .map(|handles| handles.origin())
        // Already on the ground: the world grid is drawing this exact
        // lattice, and two at one height is a moiré.
        .filter(|origin| origin.y.abs() > 1e-3)
    {
        planes.push(GridPlane {
            height: origin.y,
            step,
            cell: GUIDE_CELL,
            counting: GUIDE_COUNTING,
            // The axes belong to the world, and drawing a second set at
            // an arbitrary height says the origin moved.
            axes: false,
            // 🔴 Fixed on purpose. This grid is the ruler the drag moves
            // by, so every cell has to BE the snap step — one that
            // coarsened as you pulled the camera back would be showing
            // a distance the handle cannot land on.
            scales: false,
        });
    }
    planes
}
