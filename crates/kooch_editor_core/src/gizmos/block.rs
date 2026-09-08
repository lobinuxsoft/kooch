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
        if selection.entity != Some(entity) || selection.is_empty() {
            return;
        }
        let Some(mesh) = mesh_of(resources, entity) else {
            return;
        };

        for face in &selection.faces {
            let Some(corners) = mesh.face(*face as usize) else {
                continue;
            };
            // Drawn in world space: the gizmo batch has no per-entity
            // transform, and a face painted in local space would sit at
            // the origin for every block that is not there.
            let world: Vec<Vec3> = corners
                .iter()
                .map(|corner| {
                    transform
                        .matrix
                        .transform_point3(mesh.positions()[*corner as usize])
                })
                .collect();

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
