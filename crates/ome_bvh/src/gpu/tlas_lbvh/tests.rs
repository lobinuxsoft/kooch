//! GPU integration tests for the TLAS Karras LBVH pipeline.
//!
//! Every test acquires a wgpu device through
//! [`crate::gpu::builder::test_device::try_acquire`] and skips itself
//! gracefully when no adapter is available — CI without a display
//! falls into that path. The 16 hand-picked chunk centres in
//! [`TEST_CENTRES`] are shared across tests so any divergence is
//! attributable to the dispatch under test, not to input drift.

use super::*;
use crate::aabb::Aabb;
use crate::accel::descriptor::ChunkDescriptor;
use crate::gpu::builder::test_device;
use crate::morton::MortonCode;
use glam::Vec3;
use wgpu::util::DeviceExt;

fn descriptor_for(centre: Vec3, half: f32) -> ChunkDescriptor {
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

fn readback_u32(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    src: &wgpu::Buffer,
    n: u32,
) -> Vec<u32> {
    let bytes = (n as u64) * 4;
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
    let v: Vec<u32> = bytemuck::cast_slice::<u8, u32>(&data).to_vec();
    drop(data);
    staging.unmap();
    v
}

/// 16 hand-picked chunk centres across a 10×10×10 box. Re-used by
/// every test below so divergence between tests can only come from
/// dispatch logic, not from input drift.
const TEST_CENTRES: [Vec3; 16] = [
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

fn prepare_inputs(
    device: &wgpu::Device,
) -> (Vec<ChunkDescriptor>, Vec<Aabb>, wgpu::Buffer, wgpu::Buffer, wgpu::Buffer, u32) {
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
    (descs, aabbs, chunk_descs_buf, mortons_buf, sorted_indices_buf, n)
}

fn cpu_mortons(scene: &GpuSceneBounds, aabbs: &[Aabb]) -> Vec<u32> {
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

#[test]
fn tlas_morton_byte_identical_to_cpu() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("ome_bvh::gpu::tlas_lbvh: no GPU adapter — skipping");
        return;
    };
    let (_descs, aabbs, chunk_descs_buf, mortons_buf, _sorted_indices_buf, n) =
        prepare_inputs(&device);

    let scene = GpuSceneBounds::from_aabbs(&aabbs);
    let builder = TlasGpuBuilder::new(&device, None);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("tlas_morton_test_encoder"),
    });
    builder.dispatch_morton(
        &device,
        &queue,
        &mut encoder,
        &chunk_descs_buf,
        &mortons_buf,
        scene,
        n,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let gpu_mortons = readback_u32(&device, &queue, &mortons_buf, n);
    let cpu_mortons = cpu_mortons(&scene, &aabbs);

    assert_eq!(gpu_mortons.len(), cpu_mortons.len());
    for (i, (g, c)) in gpu_mortons.iter().zip(cpu_mortons.iter()).enumerate() {
        assert_eq!(
            g, c,
            "GPU/CPU Morton diverge at chunk[{i}]: gpu={g:#010x} cpu={c:#010x}",
        );
    }
}

#[test]
fn tlas_sort_permutation_byte_identical_to_cpu() {
    // Pins determinism: the GPU onesweep and the CPU stdlib stable
    // sort must produce *the same* permutation when keys tie. AC6
    // of epic #370 (scene hash byte-identical across runs) depends
    // on this — any divergence here would surface as a non-deterministic
    // TLAS topology under churn.
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("ome_bvh::gpu::tlas_lbvh: no GPU adapter — skipping");
        return;
    };
    let (_descs, aabbs, chunk_descs_buf, mortons_buf, sorted_indices_buf, n) =
        prepare_inputs(&device);

    let scene = GpuSceneBounds::from_aabbs(&aabbs);
    let mut builder = TlasGpuBuilder::new(&device, None);
    builder.ensure_capacity(&device, n as u64);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("tlas_sort_test_encoder"),
    });
    builder.dispatch_morton(
        &device,
        &queue,
        &mut encoder,
        &chunk_descs_buf,
        &mortons_buf,
        scene,
        n,
    );
    builder.dispatch_sort(
        &device,
        &queue,
        &mut encoder,
        &mortons_buf,
        &sorted_indices_buf,
        n,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let gpu_sorted_keys = readback_u32(&device, &queue, &mortons_buf, n);
    let gpu_sorted_indices = readback_u32(&device, &queue, &sorted_indices_buf, n);

    // CPU reference: stable sort of (morton, original_idx) pairs by
    // morton ascending. Index ties broken by original index, which
    // is what `sort_by_key` does (stable sort preserves relative
    // order). The onesweep contract is also stable in the same way.
    let cpu_unsorted = cpu_mortons(&scene, &aabbs);
    let mut cpu_indexed: Vec<(u32, u32)> = cpu_unsorted
        .iter()
        .enumerate()
        .map(|(i, k)| (*k, i as u32))
        .collect();
    cpu_indexed.sort_by_key(|(k, _)| *k);
    let cpu_sorted_keys: Vec<u32> = cpu_indexed.iter().map(|(k, _)| *k).collect();
    let cpu_sorted_indices: Vec<u32> = cpu_indexed.iter().map(|(_, i)| *i).collect();

    assert_eq!(
        gpu_sorted_keys, cpu_sorted_keys,
        "GPU sorted keys must byte-match CPU stable sort",
    );
    assert_eq!(
        gpu_sorted_indices, cpu_sorted_indices,
        "GPU sorted permutation must byte-match CPU stable sort \
         (determinism is AC6 of epic #370)",
    );
}
