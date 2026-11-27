use std::collections::{HashMap, HashSet};

use glam::{DVec3, IVec3, Vec3};
use kooch_render::meshlet::{GlobalMeshPool, MeshBounds, PoolBuffers};

use super::{COUNT_WGSL, NETS_WGSL, PARTS, WRITE_WGSL};
use crate::test_device::try_acquire;
use crate::{
    BlockKey, BlockPool, CountedBlock, Leaves, Limits, Part, Pipeline, Planets, Residency, Shape,
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
) -> (Pipeline, Vec<CountedBlock>) {
    let mut pool = BlockPool::new(64);
    let mut residency = Residency::default();
    pool.apply(keys, &mut residency);
    let mut terrain = Pipeline::new(device, queue, 64).unwrap();
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

/// The sphere at level 1 with `octants` split into their eight level-0 blocks, and those parents.
fn split_sphere(octants: &[usize]) -> (Planets, Vec<BlockKey>, HashSet<BlockKey>) {
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
    let keys = rest.chain(children).collect();
    (planets, keys, parents)
}

/// Splits whose level changes meet around a face, an edge and a corner.
const SPLITS: [&[usize]; 4] = [&[0], &[7], &[0, 1, 2], &[0, 1, 2, 3, 4, 5, 6]];

/// 🔴 A level change closes too: octants of the sphere split into their eight finer blocks (#393).
/// One octant puts coarse blocks past the fine ones' − or + faces; three of the four around an
/// axis, or seven of eight, put one coarse block among fine ones around an edge or a corner, which
/// the sphere crosses at its poles.
#[test]
fn a_split_sphere_is_watertight() {
    let mut faults = Vec::new();
    for octants in SPLITS {
        let (planets, keys, parents) = split_sphere(octants);
        let found = mesh_faults(&planets, &keys, &parents);
        // A fine cell meeting one coarse cell twice makes a flat triangle: wasted, never a hole.
        let holes = found.into_iter().filter(|(at, _)| !at.starts_with("flat"));
        faults.extend(holes.map(|(at, n)| (format!("split {octants:?}, {at}"), n)));
    }
    assert!(faults.is_empty(), "faults: {faults:#?}");
}

/// Rays a torn change may let through past the settled count, of 100k: where four blocks' overlaps
/// meet, a grazing ray still slips by now and then (41 at most, 80°, 2026-11-25).
const LEAK: usize = 64;

/// Level changes as pairs of splits: from nothing split, and from one split to a wider one, so
/// fine blocks that existed before keep a seam meshed against a coarse neighbour now split.
const CHANGES: [(&[usize], &[usize]); 7] = [
    (&[], &[0]),
    (&[], &[7]),
    (&[], &[0, 1, 2]),
    (&[], &[0, 1, 2, 3, 4, 5, 6]),
    (&[0], &[0, 1]),
    (&[0], &[0, 1, 2, 3]),
    (&[0, 1, 2], &[0, 1, 2, 3, 4, 5, 6]),
];

/// 🔴 #1999: what shows while a level change settles has no hole. Blocks on both sides of the change
/// keep the seam they were meshed with before it, beside blocks meshed after; rays from outside
/// count where the sphere is seen through, against the same count with nothing torn.
#[test]
#[ignore = "about a minute: run with --ignored when the overlap or the seams change"]
fn a_torn_sphere_is_covered() {
    let mut holes = Vec::new();
    for (from, to) in CHANGES {
        let (planets, before, split) = split_sphere(from);
        let Some(old) = meshed(&planets, &before, &split) else {
            return;
        };
        let (_, after, split) = split_sphere(to);
        let new = meshed(&planets, &after, &split).expect("a device for the first");
        for (change, shown, stale) in [("split", &new, &old), ("merge", &old, &new)] {
            for degrees in [0.0f64, 80.0] {
                let tilt = degrees.to_radians();
                let count = |overlap: bool, stale| {
                    let torn = torn(shown, stale);
                    let kept = torn
                        .into_iter()
                        .filter(|(p, _)| overlap || *p != Part::Overlap);
                    see_through(kept.map(|(_, t)| t), tilt)
                };
                let settled = count(true, shown).len();
                let without = count(false, stale).len();
                let with = count(true, stale);
                eprintln!(
                    "{change} {from:?}→{to:?} at {degrees}°: {settled} settled, torn {without} without the overlap, {} with",
                    with.len()
                );
                if with.len() > settled + LEAK {
                    holes.push(format!(
                        "{change} {from:?}→{to:?} at {degrees}°: {} rays, first at {}",
                        with.len() - settled,
                        with[0]
                    ));
                }
            }
        }
    }
    assert!(holes.is_empty(), "holes: {holes:#?}");
}

/// The meshes of `shown`, each seam swapped for the one `stale` meshed for the same block.
fn torn<'a>(
    shown: &'a [(CountedBlock, Vec<[DVec3; 3]>)],
    stale: &'a [(CountedBlock, Vec<[DVec3; 3]>)],
) -> Vec<(Part, &'a Vec<[DVec3; 3]>)> {
    let seams: HashMap<BlockKey, &Vec<[DVec3; 3]>> = stale
        .iter()
        .filter(|(b, _)| b.part == Part::Seam)
        .map(|(b, t)| (b.key, t))
        .collect();
    shown
        .iter()
        .map(|(b, t)| match b.part {
            Part::Seam => (b.part, seams.get(&b.key).copied().unwrap_or(t)),
            _ => (b.part, t),
        })
        .collect()
}

/// Rays aimed at evenly spread points of the sphere, `tilt` radians off its normal; the points of
/// those that meet no triangle before passing a metre under them. A tilted ray sees through a crack
/// beside a sunk strip that a straight one would meet.
fn see_through<'a>(meshes: impl Iterator<Item = &'a Vec<[DVec3; 3]>>, tilt: f64) -> Vec<DVec3> {
    const ROWS: usize = 128;
    const COLUMNS: usize = 256;
    let cell = |p: DVec3| {
        let d = p.normalize();
        let row = (d.y.clamp(-1.0, 1.0).acos() / std::f64::consts::PI * ROWS as f64) as usize;
        let angle = d.z.atan2(d.x) + std::f64::consts::PI;
        let column = (angle / std::f64::consts::TAU * COLUMNS as f64) as usize;
        (row.min(ROWS - 1), column.min(COLUMNS - 1))
    };
    let triangles: Vec<[DVec3; 3]> = meshes.flatten().copied().collect();
    let mut buckets = vec![Vec::new(); ROWS * COLUMNS];
    for (i, tri) in triangles.iter().enumerate() {
        let cells = tri.map(cell);
        let rows = cells.iter().map(|c| c.0);
        // One cell of slack: an edge bows off the line between its ends' cells.
        let top = rows.clone().min().unwrap_or(0).saturating_sub(1);
        let bottom = (rows.max().unwrap_or(0) + 1).min(ROWS - 1);
        let columns: Vec<usize> = cells.iter().map(|c| c.1).collect();
        let (left, right) = (
            columns.iter().min().copied().unwrap_or(0),
            columns.iter().max().copied().unwrap_or(0),
        );
        // Across the seam of the angle, or at a pole: every column.
        let span: Vec<usize> = match right - left > COLUMNS / 2 || top <= 1 || bottom + 2 >= ROWS {
            true => (0..COLUMNS).collect(),
            false => (left.saturating_sub(1)..=(right + 1).min(COLUMNS - 1)).collect(),
        };
        for row in top..=bottom {
            for &column in &span {
                buckets[row * COLUMNS + column].push(i);
            }
        }
    }
    const RAYS: usize = 100_000;
    // From this far out along the ray, to this deep under the point it aims at.
    const OUT: f64 = 3.0;
    const UNDER: f64 = 1.0;
    let golden = std::f64::consts::PI * (3.0 - 5f64.sqrt());
    (0..RAYS)
        .filter_map(|i| {
            let y = 1.0 - 2.0 * (i as f64 + 0.5) / RAYS as f64;
            let r = (1.0 - y * y).sqrt();
            let a = i as f64 * golden;
            let d = DVec3::new(r * a.cos(), y, r * a.sin());
            // Tilted off the normal towards a tangent that turns from ray to ray.
            let side = if d.y.abs() < 0.9 { DVec3::Y } else { DVec3::X };
            let (t1, t2) = (
                d.cross(side).normalize(),
                d.cross(d.cross(side)).normalize(),
            );
            let turn = i as f64 * 2.399_963;
            let tangent = t1 * turn.cos() + t2 * turn.sin();
            let back = d * tilt.cos() + tangent * tilt.sin();
            let aim = d * RADIUS;
            let from = aim + back * OUT;
            let mut seen = HashSet::new();
            let crossed = (0..=16).any(|k| {
                let at = from - back * ((OUT + UNDER) * f64::from(k) / 16.0);
                let (row, column) = cell(at);
                seen.insert(row * COLUMNS + column)
                    && buckets[row * COLUMNS + column]
                        .iter()
                        .any(|&t| hits(from, -back, triangles[t], OUT + UNDER))
            });
            (!crossed).then_some(aim)
        })
        .collect()
}

/// Whether the ray from `from` along unit `along` meets `tri` within `reach`, either side facing.
fn hits(from: DVec3, along: DVec3, tri: [DVec3; 3], reach: f64) -> bool {
    let [a, b, c] = tri;
    let (ab, ac) = (b - a, c - a);
    let p = along.cross(ac);
    let det = ab.dot(p);
    if !(det.abs() > 1e-15) {
        return false;
    }
    let to = from - a;
    let u = to.dot(p) / det;
    let q = to.cross(ab);
    let v = along.dot(q) / det;
    let t = ac.dot(q) / det;
    const SLACK: f64 = 1e-9;
    u >= -SLACK && v >= -SLACK && u + v <= 1.0 + SLACK && t > 0.0 && t < reach
}

/// Every block's mesh of `keys`, each part apart, as triangles in the planet's space. Read back
/// from the meshlets the write pass put in the pool, as the renderer reads them.
fn meshed(
    planets: &Planets,
    keys: &[BlockKey],
    split: &HashSet<BlockKey>,
) -> Option<Vec<(CountedBlock, Vec<[DVec3; 3]>)>> {
    let (device, queue) = try_acquire()?;
    let (mut terrain, blocks) = count_all(&device, &queue, planets, keys, split);
    assert_eq!(
        blocks.len(),
        keys.len() * PARTS,
        "every mesh of every block comes back"
    );
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
    let meshes = blocks.into_iter().zip(&ids).map(|(block, &id)| {
        let mesh = descriptor(id);
        let range = mesh.vertex_offset..mesh.vertex_offset + block.sizes.vertices;
        let mut triangles = Vec::new();
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
            triangles.extend((0..tris).map(|t| [corner(t, 0), corner(t, 1), corner(t, 2)]));
        }
        (block, triangles)
    });
    Some(meshes.collect())
}

/// Where the mesh fails to close, counted by kind and place. Every edge is used once each way. A
/// crack between blocks leaves an edge used once; a flipped triangle uses one twice the same way.
fn mesh_faults(
    planets: &Planets,
    keys: &[BlockKey],
    split: &HashSet<BlockKey>,
) -> Vec<(String, u32)> {
    let Some(mut blocks) = meshed(planets, keys, split) else {
        return Vec::new();
    };
    // The overlap is a sunk copy under the seam: open by design, never part of the closed surface.
    blocks.retain(|(b, _)| b.part != Part::Overlap);
    let quantise = |p: DVec3| (p * 1000.0).round().as_ivec3();
    // Which blocks a point lies on and where in each: `-`/`+` on a low/high face, `.` inside. What
    // tells a face seam from an edge or corner one.
    let place = |p: DVec3| {
        blocks
            .iter()
            .map(|(b, _)| b)
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
    for (block, mesh) in &blocks {
        for &[a, b, c] in mesh {
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
