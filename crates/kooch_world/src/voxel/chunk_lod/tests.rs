//! Tests for the chunk-LOD pass — drive `record` with deterministic
//! distances and read back the bitmask. Skip cleanly when no GPU is
//! available.

use super::*;
use crate::voxel::{MAX_SUBGRIDS_DEFAULT, SparseGrid, test_device};
use glam::Vec3;

fn make_grid(device: &wgpu::Device, queue: &wgpu::Queue, centre: Vec3) -> SparseGrid {
    // 64 m cube around `centre` so chunk_centre = `centre` exactly.
    let half = Vec3::splat(32.0);
    let bounds = Aabb::new(centre - half, centre + half);
    SparseGrid::new(device, queue, bounds, MAX_SUBGRIDS_DEFAULT)
}

fn read_mask(device: &wgpu::Device, queue: &wgpu::Queue, grid: &SparseGrid) -> u32 {
    let bytes = test_device::readback(device, queue, grid.chunk_lod_mask_buffer());
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

#[test]
fn chunk_lod_pass_writes_expected_mask_for_distance() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping chunk_lod_pass_writes_expected_mask_for_distance: no GPU");
        return;
    };
    let pass = ChunkLodPass::new(&device);

    let cases: [(Vec3, u32, &str); 4] = [
        (Vec3::splat(0.0), 0b0001, "centre coincident → only LOD 0"),
        (Vec3::new(200.0, 0.0, 0.0), 0b0011, "200 m → LOD 0 + 1"),
        (Vec3::new(1000.0, 0.0, 0.0), 0b0101, "1 km → LOD 0 + 2"),
        (Vec3::new(5000.0, 0.0, 0.0), 0b1001, "5 km → LOD 0 + 3"),
    ];
    for (centre, expected, label) in cases {
        let grid = make_grid(&device, &queue, centre);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("chunk_lod_test_encoder"),
        });
        pass.record(
            &device,
            &queue,
            &mut encoder,
            &grid,
            Vec3::ZERO,
            DEFAULT_LOD_DISTANCE_THRESHOLDS,
        );
        queue.submit(std::iter::once(encoder.finish()));
        let mask = read_mask(&device, &queue, &grid);
        assert_eq!(mask, expected, "{label}: distance test");
    }
}

#[test]
fn chunk_lod_mask_always_includes_lod_zero() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping chunk_lod_mask_always_includes_lod_zero: no GPU");
        return;
    };
    let pass = ChunkLodPass::new(&device);
    // Even at extreme distance, bit 0 should be set — downstream
    // downsample passes assume LOD 0 is populated as the cascade
    // source.
    let grid = make_grid(&device, &queue, Vec3::splat(1.0e6));
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("chunk_lod_test_encoder"),
    });
    pass.record(
        &device,
        &queue,
        &mut encoder,
        &grid,
        Vec3::ZERO,
        DEFAULT_LOD_DISTANCE_THRESHOLDS,
    );
    queue.submit(std::iter::once(encoder.finish()));
    let mask = read_mask(&device, &queue, &grid);
    assert!(
        (mask & 0x1) != 0,
        "mask {mask:#06b} must include bit 0 even at extreme distance",
    );
}
