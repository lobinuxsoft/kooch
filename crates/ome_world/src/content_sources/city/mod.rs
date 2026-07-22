//! Deterministic procedural "city" content source — fills every chunk
//! with a fixed budget of mixed volume primitives keyed off the chunk
//! coords, plus shared boundary primitives so adjacent chunks blend
//! smoothly across their shared face.
//!
//! # Determinism
//!
//! `(seed, chunk_id, prim_idx)` → one SplitMix64 stream per primitive.
//! No interior state, no global RNG, no allocator pressure — same
//! input always produces byte-identical output. AC6 of #360 (load-
//! order-independent TLAS topology) requires this.
//!
//! # Adjacency
//!
//! Each chunk emits N **interior** primitives (hashed by its own id)
//! plus **6 boundary primitives**, one per face, hashed by the
//! lower-coord chunk on that axis. Both chunks adjacent to a face
//! compute the same lower-coord chunk and therefore generate
//! byte-identical boundary primitives — their AABBs straddle the
//! shared plane and their smooth-blend supports cross the seam, so the
//! cross-chunk silhouette is continuous.

use ome_bvh::volume_primitive::{
    VolumePrimitive, TYPE_BOX, TYPE_CYLINDER, TYPE_SPHERE, primitive_aabb,
};
use ome_bvh::{Aabb, IS_RAYMARCH, LeafAabb, ROLE_RAYMARCH_ADD};

use crate::chunk::ChunkId;
use crate::content::{ChunkContent, ChunkContentSource};

/// Default interior primitives per chunk. Adds 6 boundary primitives
/// (one per face) for a total of 8 — the value the issue body pinned.
const DEFAULT_INTERIOR_PRIMITIVES: u32 = 2;

/// Domain-separation tags fed into the SplitMix mixer so interior and
/// boundary streams never collide.
const TAG_INTERIOR: u64 = 0xA1;
const TAG_BOUNDARY: u64 = 0xB2;

/// Default smoothness applied to every emitted primitive. Picked so
/// the AABB inflation crosses the chunk face on every test scale
/// (the smallest chunk side at level 0 is 64 m, so 2 m of smoothness
/// produces a visible cross-chunk blend). Tests that compare against
/// a scene-wide CPU fold at 1e-5 tolerance override this with a
/// smaller value via [`ProceduralCitySource::with_smoothness`] —
/// `smooth_union` is not associative for `k > 0`, so float drift
/// otherwise dominates the residual.
const DEFAULT_SMOOTHNESS_RADIUS: f32 = 2.0;

/// Procedural deterministic content source — the editor's default
/// while artist-authored scenes are not in scope.
pub struct ProceduralCitySource {
    seed: u64,
    interior_primitives_per_chunk: u32,
    smoothness_radius: f32,
}

impl ProceduralCitySource {
    /// Build with a custom seed. Two sources with the same seed produce
    /// byte-identical content for the same chunk id.
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            interior_primitives_per_chunk: DEFAULT_INTERIOR_PRIMITIVES,
            smoothness_radius: DEFAULT_SMOOTHNESS_RADIUS,
        }
    }

    /// Override the interior primitive count. Boundary primitives stay
    /// at 6 (one per face); the total becomes `n + 6`.
    pub fn with_interior_primitives(mut self, n: u32) -> Self {
        self.interior_primitives_per_chunk = n;
        self
    }

    /// Override the per-primitive smoothness radius. Tests comparing
    /// against a scene-wide CPU fold use `0.0` to keep `smooth_union`
    /// numerically order-independent (it collapses to `min` at
    /// `k → 0`); editor and game runtime stick with the default for
    /// the visible cross-chunk blend.
    pub fn with_smoothness(mut self, radius: f32) -> Self {
        self.smoothness_radius = radius.max(0.0);
        self
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn interior_primitives_per_chunk(&self) -> u32 {
        self.interior_primitives_per_chunk
    }

    pub fn smoothness_radius(&self) -> f32 {
        self.smoothness_radius
    }
}

impl Default for ProceduralCitySource {
    fn default() -> Self {
        // Stable default seed so editor screenshots stay reproducible
        // across invocations until an artist swaps in their own value.
        Self::new(0xC17C_5EED_5AFE_F00D)
    }
}

#[cfg(test)]
pub(super) const SMOOTHNESS_RADIUS: f32 = DEFAULT_SMOOTHNESS_RADIUS;

impl ChunkContentSource for ProceduralCitySource {
    fn populate(&self, chunk_id: ChunkId, world_aabb: Aabb) -> ChunkContent {
        let total = self.interior_primitives_per_chunk as usize + 6;
        let mut primitives = Vec::with_capacity(total);
        let mut leaf_aabbs = Vec::with_capacity(total);

        for prim_idx in 0..self.interior_primitives_per_chunk {
            let key = mix3(self.seed, chunk_hash(chunk_id), TAG_INTERIOR ^ prim_idx as u64);
            let prim = sample_interior(key, world_aabb, self.smoothness_radius);
            push_primitive(prim, self.smoothness_radius, &mut primitives, &mut leaf_aabbs);
        }

        for axis in 0..3u8 {
            for direction in [-1i32, 1i32] {
                let lower_id = lower_chunk_on_axis(chunk_id, axis, direction);
                let key = mix3(
                    self.seed,
                    chunk_hash(lower_id),
                    TAG_BOUNDARY ^ (axis as u64),
                );
                let prim = sample_boundary(
                    key,
                    chunk_id,
                    axis,
                    direction,
                    world_aabb,
                    self.smoothness_radius,
                );
                push_primitive(prim, self.smoothness_radius, &mut primitives, &mut leaf_aabbs);
            }
        }

        ChunkContent {
            primitives,
            leaf_aabbs,
            max_smoothness_radius: self.smoothness_radius,
        }
    }
}

fn push_primitive(
    prim: VolumePrimitive,
    smoothness: f32,
    prims: &mut Vec<VolumePrimitive>,
    leaves: &mut Vec<LeafAabb>,
) {
    let aabb = primitive_aabb(&prim, smoothness);
    leaves.push(LeafAabb {
        aabb_min: aabb.min.to_array(),
        flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
        aabb_max: aabb.max.to_array(),
        // Procedural content is not ECS-backed; keep `entity_id = 0`.
        // The renderer's leaf metadata path doesn't read this for
        // non-ECS leaves.
        entity_id: 0,
    });
    prims.push(prim);
}

fn sample_interior(key: u64, world_aabb: Aabb, smoothness: f32) -> VolumePrimitive {
    let (x, mut s) = unit_f32_then_advance(key);
    let (y, mut s2) = unit_f32_then_advance(s);
    let (z, _) = unit_f32_then_advance(s2);
    s = mix(s, 0xDEAD_BEEF);
    s2 = mix(s, 0x1234_5678);
    let (radius_norm, _) = unit_f32_then_advance(s);
    let position = lerp_point(world_aabb, x, y, z);
    let radius = 2.0 + 4.0 * radius_norm; // 2 .. 6 m
    primitive_for_tag(s2, position, radius, smoothness)
}

fn sample_boundary(
    key: u64,
    chunk_id: ChunkId,
    axis: u8,
    direction: i32,
    world_aabb: Aabb,
    smoothness: f32,
) -> VolumePrimitive {
    let (a, mut s) = unit_f32_then_advance(key);
    let (b, _) = unit_f32_then_advance(s);
    s = mix(s, 0xCAFE_BABE);
    let (r_norm, _) = unit_f32_then_advance(s);
    let plane_value = boundary_plane_world(chunk_id, axis, direction, world_aabb);
    let position = position_on_face(world_aabb, axis, plane_value, a, b);
    let radius = 1.5 + 2.5 * r_norm; // 1.5 .. 4 m — large enough to
    // cross the chunk face once inflated by `smoothness_radius`.
    primitive_for_tag(s, position, radius, smoothness)
}

fn primitive_for_tag(
    stream: u64,
    position: [f32; 3],
    radius: f32,
    smoothness: f32,
) -> VolumePrimitive {
    let tag = stream % 3;
    let (type_tag, params) = match tag {
        0 => (TYPE_SPHERE, [radius, 0.0, 0.0, 0.0]),
        1 => (
            TYPE_BOX,
            [radius * 0.7, radius * 0.9, radius * 0.7, radius * 0.1],
        ),
        _ => (TYPE_CYLINDER, [radius * 0.8, radius * 0.5, 0.0, 0.0]),
    };
    VolumePrimitive {
        position,
        type_tag,
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0, 1.0, 1.0],
        smoothness,
        params,
    }
}

fn lower_chunk_on_axis(chunk_id: ChunkId, axis: u8, direction: i32) -> ChunkId {
    let mut coords = chunk_id.coords;
    if direction < 0 {
        match axis {
            0 => coords.x -= 1,
            1 => coords.y -= 1,
            _ => coords.z -= 1,
        }
    }
    ChunkId::new(coords, chunk_id.level)
}

fn boundary_plane_world(
    chunk_id: ChunkId,
    axis: u8,
    direction: i32,
    world_aabb: Aabb,
) -> f32 {
    // `chunk_id.bounds` already feeds us the simulation-frame extent;
    // pick the relevant face value directly so the position lands
    // exactly on the chunk seam.
    let _ = chunk_id;
    let (lo, hi) = match axis {
        0 => (world_aabb.min.x, world_aabb.max.x),
        1 => (world_aabb.min.y, world_aabb.max.y),
        _ => (world_aabb.min.z, world_aabb.max.z),
    };
    if direction < 0 { lo } else { hi }
}

fn position_on_face(
    world_aabb: Aabb,
    axis: u8,
    plane_value: f32,
    a: f32,
    b: f32,
) -> [f32; 3] {
    let lerp = |lo: f32, hi: f32, t: f32| lo + (hi - lo) * t;
    match axis {
        0 => [
            plane_value,
            lerp(world_aabb.min.y, world_aabb.max.y, a),
            lerp(world_aabb.min.z, world_aabb.max.z, b),
        ],
        1 => [
            lerp(world_aabb.min.x, world_aabb.max.x, a),
            plane_value,
            lerp(world_aabb.min.z, world_aabb.max.z, b),
        ],
        _ => [
            lerp(world_aabb.min.x, world_aabb.max.x, a),
            lerp(world_aabb.min.y, world_aabb.max.y, b),
            plane_value,
        ],
    }
}

fn lerp_point(aabb: Aabb, x: f32, y: f32, z: f32) -> [f32; 3] {
    [
        aabb.min.x + (aabb.max.x - aabb.min.x) * x,
        aabb.min.y + (aabb.max.y - aabb.min.y) * y,
        aabb.min.z + (aabb.max.z - aabb.min.z) * z,
    ]
}

/// SplitMix64 mixer — deterministic, stateless, byte-identical across
/// platforms. The streaming layer relies on this property for AC6 of
/// #360 (TLAS topology determinism under reordered loads).
fn mix(state: u64, salt: u64) -> u64 {
    let mut z = state.wrapping_add(salt).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn mix3(a: u64, b: u64, c: u64) -> u64 {
    mix(mix(a, b), c)
}

fn unit_f32_then_advance(state: u64) -> (f32, u64) {
    let next = mix(state, 0x517C_C1B7_2722_0A95);
    let bits24 = (next >> 40) as u32; // top 24 bits = high entropy
    let f = (bits24 as f32) / ((1u32 << 24) as f32);
    (f, next)
}

fn chunk_hash(id: ChunkId) -> u64 {
    let xz = ((id.coords.x as i64 as u64) << 32) | (id.coords.z as i64 as u64 & 0xFFFF_FFFF);
    let yl = ((id.coords.y as i64 as u64) << 32) | (id.level as u64);
    mix(xz, yl)
}

#[cfg(test)]
mod tests;
