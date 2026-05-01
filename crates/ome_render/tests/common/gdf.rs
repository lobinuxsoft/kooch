//! GDF populate-pass integration test helpers — cascade readback,
//! CPU mirror of `eval_scene_bvh`, and scene fixtures (single-sphere
//! and 16-chunk procedural grid). Keeps `tests/gdf_populate.rs`
//! under the 400-LoC monolithic threshold while staying compatible
//! with the rest of the `tests/common/` harness.

#![allow(dead_code)] // Each test binary touches a different subset.

use bytemuck::Pod;
use glam::{Quat, Vec3};
use ome_bvh::{
    AccelCaps, ChunkInsert, IS_RAYMARCH, LeafAabb, OmeAccel, ROLE_RAYMARCH_ADD,
};
use ome_render::gdf::{CASCADE_0_VOXELS_PER_AXIS, CascadeDescriptor, GdfState};

use super::{SmokePrimitive, TYPE_SPHERE};

pub const ACC_UNION_IDENTITY: f32 = 1.0e6;

fn quat_rotate_inv(q: [f32; 4], v: Vec3) -> Vec3 {
    let inv = Quat::from_xyzw(-q[0], -q[1], -q[2], q[3]);
    inv.mul_vec3(v)
}

fn sdf_sphere_cpu(p: Vec3, radius: f32) -> f32 {
    p.length() - radius
}

fn smooth_union(d1: f32, d2: f32, k: f32) -> f32 {
    let h = (0.5 + 0.5 * (d2 - d1) / k).clamp(0.0, 1.0);
    d2 * (1.0 - h) + d1 * h - k * h * (1.0 - h)
}

fn smooth_intersection(d1: f32, d2: f32, k: f32) -> f32 {
    let h = (0.5 - 0.5 * (d2 - d1) / k).clamp(0.0, 1.0);
    d2 * (1.0 - h) + d1 * h + k * h * (1.0 - h)
}

fn smooth_subtraction(d1: f32, d2: f32, k: f32) -> f32 {
    let h = (0.5 - 0.5 * (d2 + d1) / k).clamp(0.0, 1.0);
    d1 * (1.0 - h) + (-d2) * h + k * h * (1.0 - h)
}

fn eval_primitive_at_cpu(p: Vec3, prim: &SmokePrimitive) -> f32 {
    let scale = Vec3::from_array(prim.scale).max(Vec3::splat(1e-5));
    let local = quat_rotate_inv(prim.rotation, p - Vec3::from_array(prim.position)) / scale;
    let s_min = scale.min_element();
    sdf_sphere_cpu(local, prim.params[0]) * s_min
}

/// Brute-force ground truth for `eval_scene_bvh`: fold every IS_RAYMARCH
/// primitive into the per-role accumulators with NO leaf-AABB gate.
///
/// **Post-#381 change:** the legacy `aabb_contains(p)` point-query
/// pruning is gone — `eval_scene_bvh` now uses distance-to-AABB
/// (`sdf_aabb(p, lo, hi) > acc_add`) which never silences a primitive
/// that could improve the running union. Equivalently: an SDF
/// primitive's contribution to the scene SDF is well-defined for ALL
/// points in R³, not just inside its leaf AABB. The GPU's pruning is
/// purely a performance optimisation that is provably equivalent to
/// brute-force when the AABBs envelope the primitive support
/// (which they do, with `max_smoothness_radius` inflation).
///
/// So the CPU mirror folds brute-force — that's the contract PR-4
/// will rely on when it samples the cascade with `textureSampleLevel`
/// at points far from any leaf AABB.
pub fn eval_scene_cpu(
    p: Vec3,
    prims: &[SmokePrimitive],
    leaves: &[LeafAabb],
    k_int_global: f32,
    k_sub_global: f32,
) -> f32 {
    let mut acc_add = ACC_UNION_IDENTITY;
    let mut acc_int = -ACC_UNION_IDENTITY;
    let mut acc_sub = ACC_UNION_IDENTITY;
    for (prim, leaf) in prims.iter().zip(leaves.iter()) {
        if (leaf.flags & IS_RAYMARCH) == 0 {
            continue;
        }
        let d = eval_primitive_at_cpu(p, prim);
        let k = prim.smoothness.max(1e-5);
        match leaf.flags & 0x3 {
            0 => acc_add = smooth_union(acc_add, d, k),
            1 => acc_int = smooth_intersection(acc_int, d, k),
            2 => acc_sub = smooth_union(acc_sub, d, k),
            _ => acc_add = smooth_union(acc_add, d, k),
        }
    }
    let k_int = k_int_global.max(1e-5);
    let k_sub = k_sub_global.max(1e-5);
    let r = smooth_intersection(acc_add, acc_int, k_int);
    smooth_subtraction(r, acc_sub, k_sub)
}

/// Signed distance from `p` to the axis-aligned box `[lo, hi]`.
/// Negative inside the box, positive outside, and equals the canonical
/// `sdf_aabb` the WGSL traversal uses for distance-to-AABB pruning.
/// Exposed so tests can classify voxels by their relationship to the
/// leaf AABB (inside / smoothness band / far) without re-deriving the
/// math at the call site.
pub fn sdf_aabb_cpu(p: Vec3, lo: Vec3, hi: Vec3) -> f32 {
    let centre = 0.5 * (lo + hi);
    let half_extent = 0.5 * (hi - lo);
    let q = (p - centre).abs() - half_extent;
    let outside = q.max(Vec3::ZERO).length();
    let inside = q.x.max(q.y.max(q.z)).min(0.0);
    outside + inside
}

/// Flat-buffer voxel layout: `voxels[(z * n + y) * n + x]`.
pub fn voxel_index(x: u32, y: u32, z: u32) -> usize {
    let n = CASCADE_0_VOXELS_PER_AXIS;
    ((z * n + y) * n + x) as usize
}

pub fn voxel_centre(descriptor: &CascadeDescriptor, x: u32, y: u32, z: u32) -> Vec3 {
    let origin = Vec3::from_array(descriptor.world_origin);
    origin
        + Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5) * descriptor.voxel_size
}

/// Read back the cascade-0 storage texture into a flat
/// `[voxels_per_axis³]` `f32` array.
pub fn readback_cascade(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    state: &GdfState,
) -> Vec<f32> {
    let n = CASCADE_0_VOXELS_PER_AXIS;
    let bytes_per_pixel = 4u32; // r32float
    // 64 voxels × 4 B = 256 B = exactly `COPY_BYTES_PER_ROW_ALIGNMENT`.
    // A future cascade-dim bump must keep this property — pin it.
    const _: () = assert!(
        (CASCADE_0_VOXELS_PER_AXIS * 4) % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT == 0,
        "cascade row stride must align to 256 B for buffer readback"
    );
    let bytes_per_row = n * bytes_per_pixel;
    let rows_per_image = n;
    let total_bytes = (bytes_per_row * rows_per_image * n) as u64;

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gdf_populate_test_staging"),
        size: total_bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("gdf_populate_test_readback_encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: state.cascade_texture(),
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(rows_per_image),
            },
        },
        wgpu::Extent3d { width: n, height: n, depth_or_array_layers: n },
    );
    queue.submit(Some(encoder.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        tx.send(r).ok();
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("device poll");
    rx.recv().expect("map_async sender").expect("map_async result");
    let data = slice.get_mapped_range();
    let out: Vec<f32> = bytemuck::cast_slice::<u8, f32>(&data).to_vec();
    drop(data);
    staging.unmap();
    out
}

/// Drive a single populate dispatch for `accel` + `camera_pos` and
/// return both the readback voxels and the descriptor that was
/// written. `camera_pos = ZERO` snaps the cascade origin so the
/// 16 m cube is centred on `(0,0,0)`.
pub fn dispatch_and_readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    accel: &OmeAccel,
    camera_pos: Vec3,
) -> (Vec<f32>, CascadeDescriptor) {
    let mut state = GdfState::new(device, &accel.buffers);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("gdf_populate_test_dispatch_encoder"),
    });
    state.dispatch_populate(&mut encoder, queue, camera_pos);
    queue.submit(Some(encoder.finish()));
    let voxels = readback_cascade(device, queue, &state);
    (voxels, state.last_descriptor())
}

/// Single-chunk scene with one sphere at the origin (radius 1.5 m,
/// per-leaf k 0.25). Returns the accel plus the CPU-mirror inputs.
pub fn build_single_sphere_accel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (OmeAccel, Vec<SmokePrimitive>, Vec<LeafAabb>, f32, f32) {
    const RADIUS: f32 = 1.5;
    const K_LEAF: f32 = 0.25;

    let prims = vec![SmokePrimitive {
        position: [0.0, 0.0, 0.0],
        type_tag: TYPE_SPHERE,
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0, 1.0, 1.0],
        smoothness: K_LEAF,
        params: [RADIUS, 0.0, 0.0, 0.0],
    }];
    let leaves = vec![LeafAabb {
        aabb_min: [-RADIUS - K_LEAF, -RADIUS - K_LEAF, -RADIUS - K_LEAF],
        flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
        aabb_max: [RADIUS + K_LEAF, RADIUS + K_LEAF, RADIUS + K_LEAF],
        entity_id: 0,
    }];
    let primitives_bytes = bytemuck::cast_slice::<_, u8>(&prims).to_vec();

    let mut accel = OmeAccel::new(
        device,
        AccelCaps::TEST,
        std::mem::size_of::<SmokePrimitive>() as u32,
    )
    .expect("OmeAccel::new");
    accel
        .insert_chunk(
            queue,
            ChunkInsert {
                key: 1,
                leaf_aabbs: &leaves,
                primitives_bytes: &primitives_bytes,
                max_smoothness_radius: K_LEAF,
            },
        )
        .expect("insert_chunk");

    let k_int_global = 0.25_f32;
    let k_sub_global = 0.25_f32;
    accel.update_gpu_standalone(device, queue, k_int_global, k_sub_global);
    (accel, prims, leaves, k_int_global, k_sub_global)
}

/// Two non-overlapping single-sphere chunks: spheres at
/// `(±separation, 0, 0)` of `radius`, each its own chunk so the TLAS
/// has two distinct leaves. Returns the accel + per-primitive +
/// per-leaf data the CPU mirror of `eval_scene_bvh` consumes.
///
/// Closes #383: with a single-leaf TLAS the `if sdf_aabb(p, leaf_far) >
/// acc_add { continue; }` BVH pruning rule never fires, so the AC test
/// in `gdf_populate_matches_eval_scene_bvh_per_voxel` does not exercise
/// it. This fixture forces the second leaf to either descend (when its
/// AABB is closer than the running `acc_add`) or be pruned (when the
/// first leaf already wins) — both branches must match the brute-force
/// CPU fold within the Nyquist voxel tolerance.
pub fn build_two_sphere_accel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    separation: f32,
    radius: f32,
) -> (OmeAccel, Vec<SmokePrimitive>, Vec<LeafAabb>, f32, f32) {
    const K_LEAF: f32 = 0.25;
    let mut prims = Vec::with_capacity(2);
    let mut leaves = Vec::with_capacity(2);
    let mut accel = OmeAccel::new(
        device,
        AccelCaps::TEST,
        std::mem::size_of::<SmokePrimitive>() as u32,
    )
    .expect("OmeAccel::new");

    for (i, x) in [-separation, separation].iter().enumerate() {
        let prim = SmokePrimitive {
            position: [*x, 0.0, 0.0],
            type_tag: TYPE_SPHERE,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            smoothness: K_LEAF,
            params: [radius, 0.0, 0.0, 0.0],
        };
        let leaf = LeafAabb {
            aabb_min: [*x - radius - K_LEAF, -radius - K_LEAF, -radius - K_LEAF],
            flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
            aabb_max: [*x + radius + K_LEAF, radius + K_LEAF, radius + K_LEAF],
            entity_id: i as u32,
        };
        let bytes = bytemuck::cast_slice::<_, u8>(&[prim]).to_vec();
        accel
            .insert_chunk(
                queue,
                ChunkInsert {
                    key: 200 + i as u64,
                    leaf_aabbs: &[leaf],
                    primitives_bytes: &bytes,
                    max_smoothness_radius: K_LEAF,
                },
            )
            .expect("insert_chunk");
        prims.push(prim);
        leaves.push(leaf);
    }

    let k_int_global = K_LEAF;
    let k_sub_global = K_LEAF;
    accel.update_gpu_standalone(device, queue, k_int_global, k_sub_global);
    (accel, prims, leaves, k_int_global, k_sub_global)
}

/// 16-chunk procedural grid: 4×4 grid of unit-ish spheres in XY,
/// dense enough that adjacent inflated AABBs overlap several
/// voxels — exercises the multi-chunk traversal path the
/// `no_zero_voxels` test depends on.
pub fn build_16_chunk_accel(device: &wgpu::Device, queue: &wgpu::Queue) -> OmeAccel {
    const RADIUS: f32 = 0.6;
    const K_LEAF: f32 = 0.1;
    const SPACING: f32 = 1.5;

    let mut accel = OmeAccel::new(
        device,
        AccelCaps::TEST,
        std::mem::size_of::<SmokePrimitive>() as u32,
    )
    .expect("OmeAccel::new");

    for (chunk_idx, (gx, gy)) in (0..4).flat_map(|x| (0..4).map(move |y| (x, y))).enumerate() {
        let cx = (gx as f32 - 1.5) * SPACING;
        let cy = (gy as f32 - 1.5) * SPACING;
        let prim = SmokePrimitive {
            position: [cx, cy, 0.0],
            type_tag: TYPE_SPHERE,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            smoothness: K_LEAF,
            params: [RADIUS, 0.0, 0.0, 0.0],
        };
        let leaf = LeafAabb {
            aabb_min: [
                cx - RADIUS - K_LEAF,
                cy - RADIUS - K_LEAF,
                -RADIUS - K_LEAF,
            ],
            flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
            aabb_max: [
                cx + RADIUS + K_LEAF,
                cy + RADIUS + K_LEAF,
                RADIUS + K_LEAF,
            ],
            entity_id: chunk_idx as u32,
        };
        let bytes = bytemuck::cast_slice::<_, u8>(&[prim]).to_vec();
        accel
            .insert_chunk(
                queue,
                ChunkInsert {
                    key: 100 + chunk_idx as u64,
                    leaf_aabbs: &[leaf],
                    primitives_bytes: &bytes,
                    max_smoothness_radius: K_LEAF,
                },
            )
            .expect("insert_chunk");
    }

    accel.update_gpu_standalone(device, queue, 0.1, 0.1);
    accel
}

/// Spin a fresh `OmeAccel` with no chunks. The TLAS uniforms still
/// get ticked so `num_chunks = 0` lands on the GPU.
pub fn build_empty_accel(device: &wgpu::Device, queue: &wgpu::Queue) -> OmeAccel {
    let mut accel = OmeAccel::new(
        device,
        AccelCaps::TEST,
        std::mem::size_of::<SmokePrimitive>() as u32,
    )
    .expect("OmeAccel::new");
    accel.update_gpu_standalone(device, queue, 0.0, 0.0);
    accel
}

// `Pod` is needed for the trait-bound on `bytemuck::cast_slice` callers
// in this module; the import otherwise looks unused under
// `#[allow(dead_code)]` rules but is required by the casts above.
#[allow(unused_imports)]
use Pod as _PodImportAnchor;
