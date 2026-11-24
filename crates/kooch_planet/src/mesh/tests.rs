use std::collections::{HashMap, HashSet};

use glam::{DVec3, IVec3, Vec3};
use kooch_render::meshlet::{GlobalMeshPool, MeshBounds, PoolBuffers};

use super::{COUNT_WGSL, NETS_WGSL, WRITE_WGSL};
use crate::test_device::try_acquire;
use crate::{
    BlockKey, BlockPool, CountedBlock, Leaves, Limits, Planets, Residency, Shape, Terrain, WriteJob,
};

const RADIUS: f64 = 6.0;

#[test]
fn both_passes_validate() {
    for source in [COUNT_WGSL, WRITE_WGSL] {
        let module = naga::front::wgsl::parse_str(&format!("{NETS_WGSL}\n{source}"))
            .unwrap_or_else(|why| panic!("{why}"));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|why| panic!("{why:?}"));
    }
}

/// The eight level-0 blocks around the centre hold the whole sphere.
fn sphere_blocks() -> (Planets, Vec<BlockKey>) {
    let mut planets = Planets::default();
    let shape = Shape {
        radius: RADIUS,
        relief: 1.0,
        depth: 1.0,
        voxel: 0.5,
        lod_distance: 32.0,
    };
    planets.push(
        shape,
        &[DVec3::ZERO],
        Limits {
            floor: 8,
            ceiling: 8,
        },
    );
    let keys = (0..8)
        .map(|i| BlockKey {
            planet: 0,
            level: 0,
            cell: IVec3::new(i & 1, (i >> 1) & 1, i >> 2) - IVec3::ONE,
        })
        .collect();
    (planets, keys)
}

fn wait(device: &wgpu::Device) {
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
}

fn count_all(device: &wgpu::Device, queue: &wgpu::Queue) -> (Terrain, Vec<CountedBlock>) {
    let (planets, keys) = sphere_blocks();
    let mut pool = BlockPool::new(64);
    let mut residency = Residency::default();
    pool.apply(&keys, &mut residency);
    let mut terrain = Terrain::new(device, queue, 64).unwrap();
    let mut out = Vec::new();
    let mut all = Vec::new();
    for _ in 0..8 {
        let split = HashSet::new();
        let leaves = Leaves {
            pool: &pool,
            split: &split,
        };
        terrain.update(device, queue, leaves, &planets, &residency, &mut out);
        residency = Residency::default();
        all.append(&mut out);
        wait(device);
        if all.len() == keys.len() {
            break;
        }
    }
    (terrain, all)
}

fn storage(device: &wgpu::Device, words: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (words.max(1) * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn read(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer) -> Vec<u32> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: buffer.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, buffer.size());
    queue.submit([encoder.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    wait(device);
    let bytes = staging.slice(..).get_mapped_range().unwrap();
    bytemuck::cast_slice(&bytes).to_vec()
}

/// 🔴 The blocks of one level close into one surface facing out: every edge is used once each way.
/// A crack between blocks leaves an edge used once; a flipped triangle uses one twice the same way.
/// Read back from the meshlets the write pass put in the pool, as the renderer reads them.
#[test]
fn a_meshed_sphere_is_watertight() {
    let Some((device, queue)) = try_acquire() else {
        return;
    };
    let (mut terrain, blocks) = count_all(&device, &queue);
    assert_eq!(blocks.len(), 8, "every block comes back");
    let mut pool = GlobalMeshPool::new();
    let ids: Vec<u32> = blocks
        .iter()
        .map(|b| pool.reserve_gpu(b.sizes, MeshBounds::default()).mesh_id)
        .collect();
    let buffers = PoolBuffers {
        vertices: storage(&device, pool.vertices.len() * 8),
        meshlet_vertices: storage(&device, pool.meshlet_vertices.len()),
        meshlet_triangles: storage(&device, pool.meshlet_triangles.len().div_ceil(4)),
        meshlets: storage(&device, pool.meshlets.len() * 28),
    };
    let descriptor = |id: u32| pool.mesh_descriptors[id as usize];
    let jobs: Vec<WriteJob> = blocks
        .iter()
        .zip(&ids)
        .map(|(b, &id)| WriteJob::new(b.slot, b.voxel, b.sizes, &descriptor(id), b.near, b.cuts))
        .collect();
    terrain.write(&device, &queue, &buffers, &jobs);
    let vertices = read(&device, &queue, &buffers.vertices);
    let meshlet_vertices = read(&device, &queue, &buffers.meshlet_vertices);
    let bytes: Vec<u8> = read(&device, &queue, &buffers.meshlet_triangles)
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect();
    let meshlets = read(&device, &queue, &buffers.meshlets);

    let quantise = |p: DVec3| (p * 1000.0).round().as_ivec3();
    let mut edges: HashMap<(IVec3, IVec3), i32> = HashMap::new();
    let mut worst = 0.0f64;
    let mut triangles = 0;
    for (block, &id) in blocks.iter().zip(&ids) {
        let mesh = descriptor(id);
        let range = mesh.vertex_offset..mesh.vertex_offset + block.sizes.vertices;
        for m in mesh.first_meshlet..mesh.first_meshlet + mesh.meshlet_count {
            let d = &meshlets[m as usize * 28..m as usize * 28 + 28];
            let (first_mv, first_byte, count, tris) = (d[0], d[1], d[2], d[3]);
            assert!(
                count <= 64 && tris <= 124,
                "{count} vertices, {tris} triangles"
            );
            let corner = |t: u32, k: u32| -> DVec3 {
                let local = u32::from(bytes[(first_byte + t * 3 + k) as usize]);
                assert!(local < count, "a corner outside its meshlet");
                let v = meshlet_vertices[(first_mv + local) as usize];
                assert!(range.contains(&v), "a vertex outside its mesh");
                let p = &vertices[v as usize * 8..v as usize * 8 + 3];
                let p = Vec3::from_array([0, 1, 2].map(|k| f32::from_bits(p[k])));
                block.origin + p.as_dvec3()
            };
            for t in 0..tris {
                let [a, b, c] = [corner(t, 0), corner(t, 1), corner(t, 2)];
                for p in [a, b, c] {
                    worst = worst.max((p.length() - RADIUS).abs());
                }
                let facing = (b - a).cross(c - a).dot((a + b + c) / 3.0);
                assert!(facing > 0.0, "a triangle faces into the planet");
                triangles += 1;
                for (from, to) in [(a, b), (b, c), (c, a)] {
                    *edges.entry((quantise(from), quantise(to))).or_default() += 1;
                }
            }
        }
    }
    assert!(triangles > 100, "{triangles} triangles");
    assert!(worst < 0.1, "a vertex sits {worst} m off the sphere");
    for (&(from, to), &count) in &edges {
        assert_eq!(count, 1, "an edge is used {count} times the same way");
        assert_eq!(
            edges.get(&(to, from)),
            Some(&1),
            "an edge has no twin: a crack"
        );
    }
}
