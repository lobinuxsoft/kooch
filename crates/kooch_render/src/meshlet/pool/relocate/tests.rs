use std::sync::OnceLock;

use super::super::{GlobalMeshPool, GpuMeshSizes, MeshBounds};
use super::MOVE_WGSL;

// One device per test binary (#334).
static SHARED_DEVICE: OnceLock<Option<(wgpu::Device, wgpu::Queue)>> = OnceLock::new();

fn try_acquire_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    SHARED_DEVICE
        .get_or_init(|| {
            let adapter = kooch_core::gpu::test_adapter()?;
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("pool_move_test_device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::default(),
            }))
            .ok()
        })
        .clone()
}

fn read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    at: u32,
    words: u32,
) -> Vec<u32> {
    let size = u64::from(words) * 4;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, u64::from(at) * 4, &staging, 0, size);
    queue.submit([encoder.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    let bytes = staging
        .slice(..)
        .get_mapped_range()
        .expect("mapped")
        .to_vec();
    bytemuck::cast_slice(&bytes).to_vec()
}

#[test]
fn the_move_pass_validates() {
    let module = naga::front::wgsl::parse_str(MOVE_WGSL).unwrap_or_else(|why| panic!("{why}"));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|why| panic!("{why:?}"));
}

/// 🔴 A GPU-written mesh holds pool indices: moved without rebasing them, it draws another mesh's
/// vertices, which reads as exploded geometry and nothing else.
#[test]
fn a_moved_mesh_is_rebased() {
    let Some((device, queue)) = try_acquire_device() else {
        return;
    };
    let sizes = GpuMeshSizes {
        vertices: 8,
        meshlet_vertices: 8,
        triangle_bytes: 12,
        meshlets: 2,
    };
    let mut pool = GlobalMeshPool::new();
    let first = pool.reserve_gpu(sizes, MeshBounds::default()).mesh_id;
    let kept = pool.reserve_gpu(sizes, MeshBounds::default()).mesh_id;
    let mut gpu = pool.upload(&device, &queue);
    // What the producer writes: indices into the ranges the mesh was granted.
    let at = pool.mesh_descriptors[kept as usize];
    let mv: Vec<u32> = (0..8).map(|k| at.vertex_offset + k).collect();
    let offset = u64::from(at.meshlet_vertex_offset) * 4;
    queue.write_buffer(&gpu.meshlet_vertices, offset, bytemuck::cast_slice(&mv));
    let mut words = [0u32; 56];
    for m in 0..2 {
        words[m * 28] = at.meshlet_vertex_offset + 4 * m as u32;
        words[m * 28 + 1] = at.meshlet_triangle_offset;
        words[m * 28 + 7] = u32::MAX;
    }
    let offset = u64::from(at.first_meshlet) * 112;
    queue.write_buffer(&gpu.meshlets, offset, bytemuck::cast_slice(&words));

    pool.release(first);
    let moved = pool.retain(&[kept]);
    gpu.sync(&device, &queue, &mut pool);
    let now = pool.mesh_descriptors[moved[0].expect("kept") as usize];
    assert_ne!(
        now.vertex_offset, at.vertex_offset,
        "the fixture did not move"
    );

    let mv = read(
        &device,
        &queue,
        &gpu.meshlet_vertices,
        now.meshlet_vertex_offset,
        8,
    );
    let want: Vec<u32> = (0..8).map(|k| now.vertex_offset + k).collect();
    assert_eq!(mv, want, "meshlet vertices still index the old range");
    let m = read(&device, &queue, &gpu.meshlets, now.first_meshlet * 28, 56);
    assert_eq!(
        [m[0], m[28]],
        [now.meshlet_vertex_offset, now.meshlet_vertex_offset + 4]
    );
    assert_eq!(m[1], now.meshlet_triangle_offset);
    assert_eq!(m[7], u32::MAX, "a root meshlet gained a parent");
}
