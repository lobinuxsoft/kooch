//! Shared fixtures for the TLAS GPU tests: 16-chunk dataset, buffer
//! preparation, GPU readback helpers, and the role_idx convention
//! mirror that pins what `tlas_internal.wgsl` writes.

use crate::aabb::Aabb;
use crate::accel::descriptor::ChunkDescriptor;
use crate::gpu::types::GpuSceneBounds;
use crate::morton::MortonCode;
use crate::node::BvhNode;
use glam::Vec3;
use wgpu::util::DeviceExt;

pub(super) fn descriptor_for(centre: Vec3, half: f32) -> ChunkDescriptor {
    let aabb = Aabb::from_centre(centre, Vec3::splat(half));
    ChunkDescriptor {
        aabb_min: aabb.min.into(),
        first_node: 0,
        aabb_max: aabb.max.into(),
        node_count: 0,
        first_leaf: 0,
        leaf_count: 0,
        first_primitive: 0,
        primitive_count: 0,
        max_smoothness_radius: 0.0,
        _pad: [0.0; 3],
    }
}

pub(super) fn readback_pod<T: bytemuck::Pod>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    src: &wgpu::Buffer,
    count: u32,
) -> Vec<T> {
    let bytes = (count as u64) * std::mem::size_of::<T>() as u64;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tlas_test_readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("tlas_test_readback_encoder"),
    });
    encoder.copy_buffer_to_buffer(src, 0, &staging, 0, bytes);
    queue.submit(std::iter::once(encoder.finish()));
    let slice = staging.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        sender.send(res).ok();
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("device poll failed");
    receiver
        .recv()
        .expect("map_async sender dropped")
        .expect("map_async failed");
    let data = slice.get_mapped_range();
    let v: Vec<T> = bytemuck::cast_slice::<u8, T>(&data).to_vec();
    drop(data);
    staging.unmap();
    v
}

pub(super) fn readback_u32(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    src: &wgpu::Buffer,
    n: u32,
) -> Vec<u32> {
    readback_pod::<u32>(device, queue, src, n)
}

/// 16 hand-picked chunk centres across a 10×10×10 box. Re-used by
/// every test below so divergence between tests can only come from
/// dispatch logic, not from input drift.
pub(super) const TEST_CENTRES: [Vec3; 16] = [
    Vec3::new(0.5, 0.5, 0.5),
    Vec3::new(9.5, 0.5, 0.5),
    Vec3::new(0.5, 9.5, 0.5),
    Vec3::new(0.5, 0.5, 9.5),
    Vec3::new(9.5, 9.5, 9.5),
    Vec3::new(2.7, 3.3, 4.1),
    Vec3::new(7.1, 1.9, 6.4),
    Vec3::new(1.2, 8.8, 2.5),
    Vec3::new(5.5, 5.5, 5.5),
    Vec3::new(3.0, 6.0, 9.0),
    Vec3::new(0.1, 0.2, 0.3),
    Vec3::new(9.9, 9.8, 9.7),
    Vec3::new(4.4, 4.4, 4.4),
    Vec3::new(6.6, 2.2, 8.8),
    Vec3::new(2.5, 7.5, 5.0),
    Vec3::new(8.0, 1.0, 3.0),
];

pub(super) struct TlasTestInputs {
    pub descs: Vec<ChunkDescriptor>,
    pub aabbs: Vec<Aabb>,
    pub chunk_descs_buf: wgpu::Buffer,
    pub mortons_buf: wgpu::Buffer,
    pub sorted_indices_buf: wgpu::Buffer,
    /// Identity mapping `[0, 1, ..., n-1]` — every chunk in
    /// `TEST_CENTRES` is live. `tlas_live_chunk_indices` lives next to
    /// `chunk_descriptors` in production, so the test fixture mirrors
    /// that pairing.
    pub live_chunk_indices_buf: wgpu::Buffer,
    pub n: u32,
}

pub(super) fn prepare_inputs(device: &wgpu::Device) -> TlasTestInputs {
    let descs: Vec<ChunkDescriptor> = TEST_CENTRES
        .iter()
        .map(|c| descriptor_for(*c, 0.4))
        .collect();
    let aabbs: Vec<Aabb> = TEST_CENTRES
        .iter()
        .map(|c| Aabb::from_centre(*c, Vec3::splat(0.4)))
        .collect();
    let n = descs.len() as u32;
    let chunk_descs_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("tlas_test_chunk_descriptors"),
        contents: bytemuck::cast_slice(&descs),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });
    let mortons_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tlas_test_mortons"),
        size: (n as u64) * 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let sorted_indices_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tlas_test_sorted_indices"),
        size: (n as u64) * 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let live_indices: Vec<u32> = (0..n).collect();
    let live_chunk_indices_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("tlas_test_live_chunk_indices"),
        contents: bytemuck::cast_slice(&live_indices),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });
    TlasTestInputs {
        descs,
        aabbs,
        chunk_descs_buf,
        mortons_buf,
        sorted_indices_buf,
        live_chunk_indices_buf,
        n,
    }
}

pub(super) fn prepare_leaf_outputs(
    device: &wgpu::Device,
    n: u32,
) -> (wgpu::Buffer, wgpu::Buffer) {
    let tlas_nodes_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tlas_test_tlas_nodes"),
        size: 2 * (n as u64) * std::mem::size_of::<BvhNode>() as u64,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let tlas_done_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tlas_test_tlas_done"),
        size: 2 * (n as u64) * 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    (tlas_nodes_buf, tlas_done_buf)
}

pub(super) fn prepare_parents(device: &wgpu::Device, n: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tlas_test_tlas_parents"),
        size: 2 * (n as u64) * 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

pub(super) fn cpu_mortons(scene: &GpuSceneBounds, aabbs: &[Aabb]) -> Vec<u32> {
    let inv = Vec3::from_array(scene.inv_extent);
    let scene_min = Vec3::from_array(scene.min);
    aabbs
        .iter()
        .map(|a| {
            let centre = a.center();
            let normalized = (centre - scene_min) * inv;
            MortonCode::from_normalized(normalized).0
        })
        .collect()
}

/// Translate a `tlas_nodes` index to its role_idx in
/// `tlas_done` / `tlas_parents` (TLAS convention: leaves at `[0, N)`,
/// internals at `[N, 2N - 1)`). Mirror of the WGSL `role_idx` helper
/// in `tlas_internal.wgsl` so the test asserts the exact convention
/// the shader writes to.
pub(super) fn role_idx(node_idx: u32, n: u32) -> u32 {
    let leaf_offset = n - 1;
    if node_idx >= leaf_offset {
        node_idx - leaf_offset
    } else {
        n + node_idx
    }
}
