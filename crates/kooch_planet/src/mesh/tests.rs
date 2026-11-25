use std::collections::{HashMap, HashSet};

use glam::{DVec3, IVec3, Vec3};
use kooch_render::meshlet::{GlobalMeshPool, MeshBounds, PoolBuffers};

use super::{COUNT_WGSL, NETS_WGSL, PARTS, WRITE_WGSL};
use crate::test_device::try_acquire;
use crate::{
    BlockKey, BlockPool, CountedBlock, Leaves, Limits, Part, Planets, Residency, Shape, Terrain,
    WriteJob,
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

/// The eight blocks of `level` around the centre, which hold the whole sphere at that voxel.
fn sphere(voxel: f64, level: u8) -> (Planets, Vec<BlockKey>) {
    let mut planets = Planets::default();
    let shape = Shape {
        radius: RADIUS,
        relief: 1.0,
        depth: 1.0,
        voxel,
        lod_distance: 32.0,
    };
    planets.push(
        shape,
        &[DVec3::ZERO],
        Limits {
            floor: 16,
            ceiling: 16,
        },
    );
    let keys = (0..8)
        .map(|i| BlockKey {
            planet: 0,
            level,
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

fn count_all(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    planets: &Planets,
    keys: &[BlockKey],
    split: &HashSet<BlockKey>,
) -> (Terrain, Vec<CountedBlock>) {
    let mut pool = BlockPool::new(64);
    let mut residency = Residency::default();
    pool.apply(keys, &mut residency);
    let mut terrain = Terrain::new(device, queue, 64).unwrap();
    let mut out = Vec::new();
    let mut all = Vec::new();
    for _ in 0..64 {
        let leaves = Leaves { pool: &pool, split };
        terrain.update(device, queue, leaves, planets, &residency, &mut out);
        residency = Residency::default();
        all.append(&mut out);
        wait(device);
        if all.len() == keys.len() * PARTS {
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

/// 🔴 The blocks of one level close into one surface facing out.
#[test]
fn a_meshed_sphere_is_watertight() {
    let (planets, keys) = sphere(0.5, 0);
    let faults = mesh_faults(&planets, &keys, &HashSet::new());
    assert!(faults.is_empty(), "faults: {faults:#?}");
}

/// 🔴 A level change closes too: octants of the sphere split into their eight finer blocks (#393).
/// One octant puts coarse blocks past the fine ones' − or + faces; three of the four around an
/// axis, or seven of eight, put one coarse block among fine ones around an edge or a corner, which
/// the sphere crosses at its poles.
#[test]
fn a_split_sphere_is_watertight() {
    let mut faults = Vec::new();
    for octants in [&[0][..], &[7], &[0, 1, 2], &[0, 1, 2, 3, 4, 5, 6]] {
        let (planets, coarse) = sphere(0.25, 1);
        let parents: HashSet<BlockKey> = octants.iter().map(|&o| coarse[o]).collect();
        let children = parents.iter().flat_map(|parent| {
            (0..8).map(|i| BlockKey {
                level: 0,
                cell: parent.cell * 2 + IVec3::new(i & 1, (i >> 1) & 1, i >> 2),
                ..*parent
            })
        });
        let rest = coarse.iter().copied().filter(|k| !parents.contains(k));
        let keys: Vec<BlockKey> = rest.chain(children).collect();
        let found = mesh_faults(&planets, &keys, &parents);
        // A fine cell meeting one coarse cell twice makes a flat triangle: wasted, never a hole.
        let holes = found.into_iter().filter(|(at, _)| !at.starts_with("flat"));
        faults.extend(holes.map(|(at, n)| (format!("split {octants:?}, {at}"), n)));
    }
    assert!(faults.is_empty(), "faults: {faults:#?}");
}

/// Where the mesh fails to close, counted by kind and place. Every edge is used once each way. A crack between blocks leaves an edge used once; a flipped
/// triangle uses one twice the same way. Read back from the meshlets the write pass put in the
/// pool, as the renderer reads them.
fn mesh_faults(
    planets: &Planets,
    keys: &[BlockKey],
    split: &HashSet<BlockKey>,
) -> Vec<(String, u32)> {
    let Some((device, queue)) = try_acquire() else {
        return Vec::new();
    };
    let (mut terrain, mut blocks) = count_all(&device, &queue, planets, keys, split);
    assert_eq!(
        blocks.len(),
        keys.len() * PARTS,
        "every mesh of every block comes back"
    );
    // The overlap is a sunk copy under the seam: open by design, never part of the closed surface.
    blocks.retain(|b| b.part != Part::Overlap);
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
        .map(|(b, &id)| {
            WriteJob::new(
                b.slot,
                b.voxel,
                b.sizes,
                &descriptor(id),
                b.near,
                b.cuts,
                b.part,
            )
        })
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
    // Which blocks a point lies on and where in each: `-`/`+` on a low/high face, `.` inside. What
    // tells a face seam from an edge or corner one.
    let place = |p: DVec3| {
        blocks
            .iter()
            .filter(|b| b.part == Part::Interior)
            .filter_map(|b| {
                let side = f64::from(b.voxel) * 16.0;
                let local = (p - b.origin) / side;
                let near = local.cmpge(DVec3::splat(-0.02)) & local.cmple(DVec3::splat(1.02));
                near.all().then(|| {
                    let face = local.to_array().map(|x| match x {
                        x if x < 0.02 => '-',
                        x if x > 0.98 => '+',
                        _ => '.',
                    });
                    format!("L{} {}", b.key.level, face.iter().collect::<String>())
                })
            })
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let mut faults: HashMap<String, u32> = HashMap::new();
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
                let centre = (a + b + c) / 3.0;
                let facing = (b - a).cross(c - a).dot(centre);
                triangles += 1;
                if !(facing > 0.0) {
                    let kind = if facing == 0.0 { "flat" } else { "inward" };
                    let key = format!("{kind} by L{}: {}", block.key.level, place(centre));
                    *faults.entry(key).or_default() += 1;
                    // A flat one repeats a corner and would twin its own edges.
                    if facing == 0.0 {
                        continue;
                    }
                }
                for (from, to) in [(a, b), (b, c), (c, a)] {
                    *edges.entry((quantise(from), quantise(to))).or_default() += 1;
                }
            }
        }
    }
    assert!(triangles > 100, "{triangles} triangles");
    assert!(worst < 0.1, "a vertex sits {worst} m off the sphere");
    for (&(from, to), &count) in &edges {
        let mid = (from + to).as_dvec3() / 2000.0;
        let kind = match (count, edges.get(&(to, from)).copied().unwrap_or(0)) {
            (1, 1) => continue,
            (1, _) if !edges.contains_key(&(to, from)) => "open",
            _ => "doubled",
        };
        *faults.entry(format!("{kind}: {}", place(mid))).or_default() += 1;
    }
    let mut faults: Vec<_> = faults.into_iter().collect();
    faults.sort();
    eprintln!("{triangles} triangles");
    faults
}
