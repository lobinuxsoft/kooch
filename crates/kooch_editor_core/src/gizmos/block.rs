//! Painting the faces of a block that are selected.

use glam::{Vec3, Vec4};
use kooch_blockmesh::Block;
use kooch_core::resource::Resources;
use kooch_ecs::GlobalTransform;
use kooch_ecs::entity::Entity;
use kooch_gizmos::{Gizmos, Visualizer};

use crate::block_edit::{BlockSelection, mesh_of};

/// Orange, and translucent so the face under it still reads.
const SELECTED: Vec4 = Vec4::new(1.0, 0.55, 0.12, 0.45);
/// The same hue, opaque, for the outline.
const OUTLINE: Vec3 = Vec3::new(1.0, 0.55, 0.12);
/// The topology, in the blue every block-out tool uses for it.
const WIRE: Vec3 = Vec3::new(0.25, 0.45, 1.0);
/// A corner marker, sized in world units. Small enough not to hide the
/// face it sits on, big enough to aim at.
const CORNER: f32 = 0.03;

#[derive(Default)]
pub(crate) struct BlockVisualizer;

impl Visualizer<Block> for BlockVisualizer {
    fn draw_with(
        &self,
        _block: &Block,
        transform: &GlobalTransform,
        entity: Entity,
        resources: &Resources,
        gizmos: &mut Gizmos<'_>,
    ) {
        let Some(selection) = resources.get::<BlockSelection>() else {
            return;
        };
        // 🔴 Drawn whenever an element mode is on, not only when
        // something is selected. The wireframe is how you see WHERE to
        // click; requiring a click first is the wrong way round.
        if selection.mode == crate::block_edit::ElementMode::Object {
            return;
        }
        let Some(mesh) = mesh_of(resources, entity) else {
            return;
        };

        let to_world = |corner: u32| {
            transform
                .matrix
                .transform_point3(mesh.positions()[corner as usize])
        };

        // Every edge once. Walking faces would draw each interior edge
        // twice, which reads brighter and makes a shared edge look like
        // a seam that is not there.
        let adjacency = kooch_blockmesh::Adjacency::of(&mesh);
        for edge in 0..adjacency.edge_count() as u32 {
            if let Some([a, b]) = adjacency.edge_corners(edge) {
                gizmos.line(to_world(a), to_world(b), WIRE);
            }
        }
        for corner in 0..mesh.positions().len() as u32 {
            gizmos.filled_aabb(to_world(corner), Vec3::splat(CORNER), WIRE.extend(1.0));
        }

        if selection.entity != Some(entity) || selection.is_empty() {
            return;
        }

        for face in &selection.faces {
            let Some(corners) = mesh.face(*face as usize) else {
                continue;
            };
            // Drawn in world space: the gizmo batch has no per-entity
            // transform, and a face painted in local space would sit at
            // the origin for every block that is not there.
            let world: Vec<Vec3> = corners.iter().map(|corner| to_world(*corner)).collect();

            // A fan, because the face is convex and `filled_quad` is the
            // widest primitive the batch has.
            for step in 1..world.len() - 1 {
                gizmos.filled_quad(world[0], world[step], world[step + 1], world[0], SELECTED);
            }
            // The outline is what makes a face read as *one* face rather
            // than as a bright patch — two coplanar neighbours selected
            // together are otherwise indistinguishable from one.
            for step in 0..world.len() {
                gizmos.line(world[step], world[(step + 1) % world.len()], OUTLINE);
            }
        }
    }

    fn draw(&self, _block: &Block, _transform: &GlobalTransform, _gizmos: &mut Gizmos<'_>) {}
}
