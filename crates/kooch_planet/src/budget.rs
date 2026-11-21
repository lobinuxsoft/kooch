//! How many blocks the meshlet pool's bindings can hold (#1968): the block count is only a proxy,
//! the bytes a block costs grow with relief and finer voxels.

use kooch_render::meshlet::PoolUsage;

/// Share of each binding the blocks may fill. The rest absorbs a swap holding two meshes per block
/// and blocks denser than the average.
const FILL: f64 = 0.85;

/// Blocks that keep the fullest array under [`FILL`] of its binding, at what the resident ones cost
/// on average. Every mesh in the pool is charged to the planets, so other meshes only make it
/// stricter.
pub(crate) fn blocks_within(resident: u32, usage: PoolUsage, capacity: u32) -> u32 {
    if resident == 0 {
        return capacity;
    }
    let (used, room) = (usage.used, usage.room);
    [
        (used.vertices, room.vertices),
        (used.meshlet_vertices, room.meshlet_vertices),
        (used.triangle_bytes, room.triangle_bytes),
        (used.meshlets, room.meshlets),
    ]
    .into_iter()
    .filter(|&(used, _)| used > 0)
    .map(|(used, room)| (f64::from(resident) * FILL * f64::from(room) / f64::from(used)) as u32)
    .min()
    .map_or(capacity, |blocks| blocks.min(capacity))
}

#[cfg(test)]
mod tests {
    use kooch_render::meshlet::{GpuMeshSizes, PoolUsage};

    use super::blocks_within;

    fn usage(vertices: u32) -> PoolUsage {
        PoolUsage {
            used: GpuMeshSizes {
                vertices,
                ..Default::default()
            },
            room: GpuMeshSizes {
                vertices: 4_194_304,
                ..Default::default()
            },
        }
    }

    /// Twice `earth_test`'s density: 6144 blocks holding 4.37 M vertices against a 4.19 M room.
    #[test]
    fn a_dense_planet_is_limited() {
        let blocks = blocks_within(6144, usage(4_367_052), 6144);
        assert_eq!(blocks, 5015);
    }

    #[test]
    fn a_light_planet_keeps_capacity() {
        assert_eq!(blocks_within(6144, usage(1_000_000), 6144), 6144);
    }
}
