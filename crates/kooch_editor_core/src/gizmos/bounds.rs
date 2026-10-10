//! [`MeshBoundsVisualizer`] — the box a model occupies (#1455).

use glam::Vec3;

use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_gizmos::{Gizmos, Visualizer};

/// Cool and dim: the box is a reference, and it is drawn around whatever else is selected.
const BOUNDS: Vec3 = Vec3::new(0.45, 0.55, 0.7);

/// The twelve edges of a cube, as index pairs into the corner array. Two corners share an edge
/// when exactly one axis bit differs.
const EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

#[derive(Default)]
pub(crate) struct MeshBoundsVisualizer;

impl Visualizer<MeshRenderer> for MeshBoundsVisualizer {
    fn draw_with(
        &self,
        renderer: &MeshRenderer,
        transform: &GlobalTransform,
        _entity: Entity,
        resources: &Resources,
        gizmos: &mut Gizmos<'_>,
    ) {
        if !renderer.visible {
            return;
        }
        let Some(bounds) = renderer
            .mesh
            .and_then(|mesh| crate::picking::mesh_bounds(resources, mesh))
        else {
            return;
        };

        // 🔴 The LOCAL box through the matrix, not the world box around it. A rotated prop's world
        // box grows until it swallows the model; this one keeps the model's own axes and stays
        // wrapped around it.
        let corners: [Vec3; 8] = std::array::from_fn(|index| {
            transform.matrix.transform_point3(Vec3::new(
                if index & 1 == 0 {
                    bounds.min.x
                } else {
                    bounds.max.x
                },
                if index & 2 == 0 {
                    bounds.min.y
                } else {
                    bounds.max.y
                },
                if index & 4 == 0 {
                    bounds.min.z
                } else {
                    bounds.max.z
                },
            ))
        });
        for (from, to) in EDGES {
            gizmos.line(corners[from], corners[to], BOUNDS);
        }
    }

    /// The mesh lives in the asset store, not in the component, so there is nothing to draw
    /// without `Resources`.
    fn draw(
        &self,
        _renderer: &MeshRenderer,
        _transform: &GlobalTransform,
        _gizmos: &mut Gizmos<'_>,
    ) {
    }
}
