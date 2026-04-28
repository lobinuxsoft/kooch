//! Shared GPU test harness for the frustum cull suite. Helpers are
//! `pub(super)` so the sibling `bench` module reuses them through the
//! `tests` module re-export.

use bytemuck::cast_slice;
use glam::{Vec3, Vec4};
use ome_bvh::{Aabb, IS_VISIBLE_MESH, LeafAabb, SharedBvhState};

use crate::frustum::cull::{DrawIndexedIndirectArgs, FrustumCull, FrustumPlanes};

/// Headless GPU acquisition matching `ome_bvh::test_device::try_acquire`
/// and `raymarch::bvh::gpu_tests::harness::try_acquire_device`. Skipped
/// when no adapter has the timestamp features the BvhGpuBuilder needs.
pub(crate) fn try_acquire_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    pollster::block_on(async {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let needs =
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES;
        if !adapter.features().contains(needs) {
            return None;
        }
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("frustum_cull::test_device"),
                required_features: needs,
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::default(),
            })
            .await
            .ok()?;
        Some((device, queue))
    })
}

/// Pump the device until the kicked build resolves into the shared
/// state. Spins on `poll_swap` while submitting a `Wait` poll between
/// attempts so wgpu fires the `map_async` callbacks. Test-only.
pub(crate) fn drive_build_to_completion(
    shared: &mut SharedBvhState,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) {
    loop {
        match shared.poll_swap(device, queue) {
            Some(Ok(_)) => return,
            Some(Err(e)) => panic!("SharedBvhState build failed: {e:?}"),
            None => {
                let _ = device.poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(std::time::Duration::from_secs(30)),
                });
            }
        }
    }
}

/// Build a (`items`, `leaf_aabbs`) pair where every leaf is flagged
/// `IS_VISIBLE_MESH` — broadphase-only flags would never reach the
/// frustum cull path. `entity_id == original_index` so the indirect
/// args buffer aligns 1:1 with the input ordering.
pub(crate) fn visible_mesh_scene(
    centres: &[Vec3],
    half: f32,
) -> (Vec<(u32, Aabb)>, Vec<LeafAabb>) {
    let items: Vec<(u32, Aabb)> = centres
        .iter()
        .enumerate()
        .map(|(i, c)| (i as u32, Aabb::from_centre(*c, Vec3::splat(half))))
        .collect();
    let leaves: Vec<LeafAabb> = items
        .iter()
        .map(|(i, a)| LeafAabb {
            aabb_min: a.min.to_array(),
            flags: IS_VISIBLE_MESH,
            aabb_max: a.max.to_array(),
            entity_id: *i,
        })
        .collect();
    (items, leaves)
}

/// CPU mirror of `aabb_in_frustum` from `frustum_cull.wgsl`. The shader
/// and this function MUST emit identical decisions for every AABB or
/// the byte-level test will fail.
pub(crate) fn cpu_aabb_in_frustum(
    aabb_min: [f32; 3],
    aabb_max: [f32; 3],
    planes: &[Vec4; 6],
) -> bool {
    for plane in planes {
        let n = plane.truncate();
        let pv = Vec3::new(
            if n.x >= 0.0 { aabb_max[0] } else { aabb_min[0] },
            if n.y >= 0.0 { aabb_max[1] } else { aabb_min[1] },
            if n.z >= 0.0 { aabb_max[2] } else { aabb_min[2] },
        );
        if n.dot(pv) + plane.w < 0.0 {
            return false;
        }
    }
    true
}

/// Axis-aligned box frustum: inside iff `min <= p <= max`. Convenient
/// reference shape: every plane has a single non-zero normal component
/// so manual hand-calculations stay tractable.
pub(crate) fn axis_aligned_box_frustum(min: Vec3, max: Vec3) -> FrustumPlanes {
    FrustumPlanes([
        Vec4::new(1.0, 0.0, 0.0, -min.x), // x >= min.x  →  x - min.x >= 0
        Vec4::new(-1.0, 0.0, 0.0, max.x), // x <= max.x  → -x + max.x >= 0
        Vec4::new(0.0, 1.0, 0.0, -min.y),
        Vec4::new(0.0, -1.0, 0.0, max.y),
        Vec4::new(0.0, 0.0, 1.0, -min.z),
        Vec4::new(0.0, 0.0, -1.0, max.z),
    ])
}

/// Run the cull dispatch and read back the `n` indirect args.
pub(crate) fn dispatch_and_readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cull: &mut FrustumCull,
    shared: &SharedBvhState,
    planes: &FrustumPlanes,
    n: u32,
) -> Vec<DrawIndexedIndirectArgs> {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("frustum_cull_test_encoder"),
    });
    cull.cull(device, queue, &mut encoder, shared, planes, /* index_count */ 36);

    let bytes = n as u64 * std::mem::size_of::<DrawIndexedIndirectArgs>() as u64;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("frustum_cull_test_staging"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(cull.indirect_buffer(), 0, &staging, 0, bytes);
    queue.submit(std::iter::once(encoder.finish()));

    let (tx, rx) = std::sync::mpsc::channel();
    staging.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    rx.recv()
        .expect("staging map sender dropped")
        .expect("staging map failed");

    let data = staging.slice(..).get_mapped_range();
    let args: Vec<DrawIndexedIndirectArgs> =
        cast_slice::<u8, DrawIndexedIndirectArgs>(&data).to_vec();
    drop(data);
    staging.unmap();
    args
}
