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
