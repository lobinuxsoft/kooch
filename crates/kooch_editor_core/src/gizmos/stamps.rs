//! [`StampVisualizer`]: each of a terrain's stamps drawn as its shape, with the grip that selects it.

use glam::{Vec3, Vec4};

use kooch_core::resource::Resources;
use kooch_ecs::GlobalTransform;
use kooch_ecs::entity::Entity;
use kooch_gizmos::{Gizmos, Visualizer};
use kooch_terrain::{OPERATION_INTERSECT, OPERATION_SUBTRACT, SHAPE_BOX, Terrain};

use super::stamp_handles::{base_of, hovered_index, selected_index, world_frame};

const ADD: Vec3 = Vec3::new(0.35, 0.9, 0.4);
const SUBTRACT: Vec3 = Vec3::new(0.95, 0.35, 0.3);
const INTERSECT: Vec3 = Vec3::new(0.4, 0.6, 1.0);
const OFF: Vec3 = Vec3::new(0.5, 0.5, 0.5);

#[derive(Default)]
pub(crate) struct StampVisualizer;

impl Visualizer<Terrain> for StampVisualizer {
    /// Everything is in `draw_with`, which knows the terrain's base and the selected stamp.
    fn draw(&self, _terrain: &Terrain, _transform: &GlobalTransform, _gizmos: &mut Gizmos<'_>) {}

    fn draw_with(
        &self,
        terrain: &Terrain,
        transform: &GlobalTransform,
        entity: Entity,
        resources: &Resources,
        gizmos: &mut Gizmos<'_>,
    ) {
        let matrix = &transform.matrix;
        let base = base_of(resources, entity);
        let scale = matrix.x_axis.truncate().length();
        let (hovered, chosen) = (
            hovered_index(resources, entity),
            selected_index(resources, entity),
        );
        let stamps = super::stamp_handles::live_stamps(resources, entity)
            .unwrap_or_else(|| terrain.stamps.clone());
        for (index, stamp) in stamps.iter().enumerate() {
            let (at, axes) = world_frame(matrix, stamp, base);
            let colour = match (stamp.enabled, stamp.operation) {
                (false, _) => OFF,
                (true, OPERATION_SUBTRACT) => SUBTRACT,
                (true, OPERATION_INTERSECT) => INTERSECT,
                _ => ADD,
            };
            match stamp.shape {
                SHAPE_BOX => {
                    let half = stamp.size * scale;
                    let corner = |i: u32| {
                        let sign = |bit: u32| if i & bit == 0 { -1.0 } else { 1.0 };
                        at + axes * (half * Vec3::new(sign(1), sign(2), sign(4)))
                    };
                    for i in 0..8u32 {
                        for bit in [1, 2, 4] {
                            if i & bit == 0 {
                                gizmos.line(corner(i), corner(i | bit), colour);
                            }
                        }
                    }
                }
                _ => gizmos.wire_sphere(at, axes, stamp.radius * scale, colour),
            }
            let grip = match (hovered == Some(index), chosen == Some(index)) {
                (true, _) => Vec4::new(1.0, 0.85, 0.1, 1.0),
                (false, true) => Vec4::new(0.3, 0.9, 1.0, 1.0),
                _ => colour.extend(1.0),
            };
            let half = super::shape_handles::handle_size(resources, at);
            gizmos.filled_aabb(at, Vec3::splat(half), grip);
        }
    }
}
