use super::buffers::LbvhBuffers;
use crate::node::BvhNode;

/// Test-only readback of the nodes buffer. Returns `2n-1` BvhNodes
/// in a CPU `Vec`. Production code never roundtrips this — the GPU
/// builder owns the buffer for downstream traversal kernels.
pub fn readback_nodes_for_test(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffers: &LbvhBuffers,
    n: u32,
) -> Vec<BvhNode> {
    let total = (2 * n - 1) as u64;
    let bytes = total * std::mem::size_of::<BvhNode>() as u64;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_bvh::lbvh_nodes_readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ome_bvh::lbvh_nodes_readback_encoder"),
    });
    encoder.copy_buffer_to_buffer(&buffers.nodes_buffer, 0, &staging, 0, bytes);
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
    let v: Vec<BvhNode> = bytemuck::cast_slice::<u8, BvhNode>(&data).to_vec();
    drop(data);
    staging.unmap();
    v
}

#[cfg(test)]
mod tests {
    use super::readback_nodes_for_test;
    use crate::aabb::Aabb;
    use crate::bvh::Bvh;
    use crate::gpu::builder::test_device;
    use crate::gpu::lbvh::buffers::LbvhBuffers;
    use crate::gpu::lbvh::dispatch::dispatch_lbvh_build;
    use crate::gpu::lbvh::pipelines::LbvhPipelines;
    use crate::gpu::types::GpuAabb;
    use crate::morton::MortonCode;
    use crate::node::BvhNode;
    use glam::Vec3;
    use wgpu::util::DeviceExt;

    /// Replicate the CPU's morton encoding + stable sort to produce
    /// `(sorted_morton, sorted_indices, original_aabbs)` triples for
    /// the GPU dispatch. Decouples LBVH testing from sort correctness.
    fn cpu_prepare_inputs(
        aabbs: &[Aabb],
    ) -> (Vec<u32>, Vec<u32>, Vec<GpuAabb>) {
        let scene = aabbs.iter().fold(Aabb::EMPTY, |acc, a| acc.union(a));
        let extent = scene.max - scene.min;
        let inv = Vec3::new(
            if extent.x > 0.0 { 1.0 / extent.x } else { 0.0 },
            if extent.y > 0.0 { 1.0 / extent.y } else { 0.0 },
            if extent.z > 0.0 { 1.0 / extent.z } else { 0.0 },
        );
        let mut indexed: Vec<(MortonCode, u32)> = aabbs
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let normalized = (a.center() - scene.min) * inv;
                (MortonCode::from_normalized(normalized), i as u32)
            })
            .collect();
        indexed.sort_by_key(|(c, _)| *c);
        let sorted_morton: Vec<u32> = indexed.iter().map(|(c, _)| c.0).collect();
        let sorted_indices: Vec<u32> = indexed.iter().map(|(_, i)| *i).collect();
        let originals: Vec<GpuAabb> = aabbs.iter().copied().map(GpuAabb::from).collect();
        (sorted_morton, sorted_indices, originals)
    }

    fn upload_storage<T: bytemuck::Pod>(
        device: &wgpu::Device,
        data: &[T],
        label: &str,
    ) -> wgpu::Buffer {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(data),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn run_gpu_lbvh(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        aabbs: &[Aabb],
    ) -> Vec<BvhNode> {
        let n = aabbs.len() as u32;
        let pipelines = LbvhPipelines::new(device, None);
        let mut buffers = LbvhBuffers::new(device);
        buffers.ensure_capacity(device, n as u64);

        let (sorted_morton, sorted_indices, originals) = cpu_prepare_inputs(aabbs);
        let morton_buf = upload_storage(device, &sorted_morton, "test_sorted_morton");
        let indices_buf = upload_storage(device, &sorted_indices, "test_sorted_indices");
        let aabbs_buf = upload_storage(device, &originals, "test_original_aabbs");

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ome_bvh::test_lbvh_encoder"),
        });
        dispatch_lbvh_build(
            device,
            queue,
            &mut encoder,
            &pipelines,
            &buffers,
            &aabbs_buf,
            &morton_buf,
            &indices_buf,
            n,
        );
        queue.submit(std::iter::once(encoder.finish()));
        readback_nodes_for_test(device, queue, &buffers, n)
    }

    fn aabb_at(centre: Vec3, half: f32) -> Aabb {
        Aabb::from_centre(centre, Vec3::splat(half))
    }

    fn assert_gpu_matches_cpu(gpu: &[BvhNode], cpu: &[BvhNode]) {
        if gpu == cpu {
            return;
        }
        assert_eq!(gpu.len(), cpu.len(), "node count mismatch");
        for (i, (g, c)) in gpu.iter().zip(cpu.iter()).enumerate() {
            if g != c {
                panic!(
                    "GPU/CPU diverge at node[{i}]:\n  gpu: {:?}\n  cpu: {:?}",
                    g, c
                );
            }
        }
    }

    #[test]
    fn gpu_lbvh_matches_cpu_single_leaf() {
        let Some((device, queue)) = test_device::try_acquire() else {
            eprintln!("ome_bvh::gpu::lbvh: no GPU adapter — skipping");
            return;
        };
        let aabbs = vec![aabb_at(Vec3::ZERO, 1.0)];
        let gpu = run_gpu_lbvh(&device, &queue, &aabbs);
        let items: Vec<(u32, Aabb)> = aabbs.iter().copied().enumerate().map(|(i, a)| (i as u32, a)).collect();
        let cpu = Bvh::build(items);
        assert_gpu_matches_cpu(&gpu, &cpu.nodes);
    }

    #[test]
    fn gpu_lbvh_matches_cpu_two_leaves() {
        let Some((device, queue)) = test_device::try_acquire() else { return; };
        let aabbs = vec![
            aabb_at(Vec3::ZERO, 0.5),
            aabb_at(Vec3::splat(10.0), 0.5),
        ];
        let gpu = run_gpu_lbvh(&device, &queue, &aabbs);
        let items: Vec<(u32, Aabb)> = aabbs.iter().copied().enumerate().map(|(i, a)| (i as u32, a)).collect();
        let cpu = Bvh::build(items);
        assert_gpu_matches_cpu(&gpu, &cpu.nodes);
    }

    #[test]
    fn gpu_lbvh_matches_cpu_8_balanced_leaves() {
        let Some((device, queue)) = test_device::try_acquire() else { return; };
        let aabbs: Vec<Aabb> = (0..8)
            .map(|i| aabb_at(Vec3::new(i as f32, 0.0, 0.0), 0.4))
            .collect();
        let gpu = run_gpu_lbvh(&device, &queue, &aabbs);
        let items: Vec<(u32, Aabb)> = aabbs.iter().copied().enumerate().map(|(i, a)| (i as u32, a)).collect();
        let cpu = Bvh::build(items);
        assert_gpu_matches_cpu(&gpu, &cpu.nodes);
    }

    #[test]
    fn gpu_lbvh_matches_cpu_5_leaves_asymmetric() {
        // Asymmetric tree — exercises the "right child isn't left + 1"
        // case where one subtree is internal, the other a leaf.
        let Some((device, queue)) = test_device::try_acquire() else { return; };
        let aabbs: Vec<Aabb> = (0..5)
            .map(|i| aabb_at(Vec3::new(i as f32, 0.0, 0.0), 0.4))
            .collect();
        let gpu = run_gpu_lbvh(&device, &queue, &aabbs);
        let items: Vec<(u32, Aabb)> = aabbs.iter().copied().enumerate().map(|(i, a)| (i as u32, a)).collect();
        let cpu = Bvh::build(items);
        assert_gpu_matches_cpu(&gpu, &cpu.nodes);
    }

    #[test]
    fn gpu_lbvh_matches_cpu_1024_leaves() {
        let Some((device, queue)) = test_device::try_acquire() else { return; };
        let aabbs: Vec<Aabb> = (0..1024u32)
            .map(|i| {
                let x = (i % 32) as f32;
                let y = (i / 32) as f32;
                aabb_at(Vec3::new(x, y, 0.0), 0.4)
            })
            .collect();
        let gpu = run_gpu_lbvh(&device, &queue, &aabbs);
        let items: Vec<(u32, Aabb)> = aabbs.iter().copied().enumerate().map(|(i, a)| (i as u32, a)).collect();
        let cpu = Bvh::build(items);
        assert_eq!(gpu.len(), cpu.nodes.len(), "1024-leaf GPU/CPU node count must match");
        assert_gpu_matches_cpu(&gpu, &cpu.nodes);
    }

    #[test]
    fn gpu_lbvh_matches_cpu_duplicate_morton_codes() {
        // 4 items at the exact same centre — all share one Morton code.
        // Index tie-break in delta() must keep CPU and GPU in agreement.
        let Some((device, queue)) = test_device::try_acquire() else { return; };
        let aabbs: Vec<Aabb> = (0..4)
            .map(|_| aabb_at(Vec3::ZERO, 0.5))
            .collect();
        let gpu = run_gpu_lbvh(&device, &queue, &aabbs);
        let items: Vec<(u32, Aabb)> = aabbs.iter().copied().enumerate().map(|(i, a)| (i as u32, a)).collect();
        let cpu = Bvh::build(items);
        assert_gpu_matches_cpu(&gpu, &cpu.nodes);
    }

    #[test]
    fn gpu_lbvh_matches_cpu_random_100_leaves() {
        let Some((device, queue)) = test_device::try_acquire() else { return; };
        // Deterministic pseudo-random AABBs across a 10×10×10 box.
        let mut state: u32 = 0xc0ffee01;
        let mut rand = || {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            (state >> 16) as f32 / 32768.0
        };
        let aabbs: Vec<Aabb> = (0..100)
            .map(|_| {
                let centre = Vec3::new(rand(), rand(), rand()) * 10.0;
                Aabb::from_centre(centre, Vec3::splat(0.2))
            })
            .collect();
        let gpu = run_gpu_lbvh(&device, &queue, &aabbs);
        let items: Vec<(u32, Aabb)> = aabbs.iter().copied().enumerate().map(|(i, a)| (i as u32, a)).collect();
        let cpu = Bvh::build(items);
        assert_gpu_matches_cpu(&gpu, &cpu.nodes);
    }
}
