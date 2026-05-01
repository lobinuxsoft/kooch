//! CPU-only tests for [`super::ProceduralCitySource`]. Pin the two
//! invariants the renderer + streaming layer compose against:
//! determinism (AC6 of #360 — TLAS topology byte-identical under
//! reordered loads) and adjacency (cross-chunk smooth-blend continuity
//! at chunk faces).

use super::*;
use glam::{IVec3, Vec3};
use ome_core::coord::ActiveOrigin;

fn chunk(x: i32, y: i32, z: i32) -> ChunkId {
    ChunkId::new(IVec3::new(x, y, z), 0)
}

fn world_aabb_for(id: ChunkId) -> Aabb {
    id.bounds(&ActiveOrigin::ZERO)
}

#[test]
fn populate_is_deterministic_for_same_seed_and_id() {
    let src = ProceduralCitySource::new(0xDEAD_BEEF);
    let id = chunk(3, -2, 5);
    let aabb = world_aabb_for(id);
    let a = src.populate(id, aabb);
    let b = src.populate(id, aabb);
    assert_eq!(a.primitives, b.primitives);
    assert_eq!(a.leaf_aabbs.len(), b.leaf_aabbs.len());
    for (la, lb) in a.leaf_aabbs.iter().zip(b.leaf_aabbs.iter()) {
        assert_eq!(la.aabb_min, lb.aabb_min);
        assert_eq!(la.aabb_max, lb.aabb_max);
        assert_eq!(la.flags, lb.flags);
        assert_eq!(la.entity_id, lb.entity_id);
    }
}

#[test]
fn different_seeds_produce_different_content() {
    let s1 = ProceduralCitySource::new(1);
    let s2 = ProceduralCitySource::new(2);
    let id = chunk(0, 0, 0);
    let aabb = world_aabb_for(id);
    assert_ne!(s1.populate(id, aabb).primitives, s2.populate(id, aabb).primitives);
}

#[test]
fn total_primitive_count_is_interior_plus_six() {
    let src = ProceduralCitySource::new(7);
    let id = chunk(0, 0, 0);
    let content = src.populate(id, world_aabb_for(id));
    assert_eq!(
        content.primitives.len(),
        src.interior_primitives_per_chunk() as usize + 6,
    );
    assert_eq!(content.leaf_aabbs.len(), content.primitives.len());
}

#[test]
fn adjacent_chunks_share_boundary_primitive() {
    // Two chunks sharing the +x face of A == -x face of B.
    let src = ProceduralCitySource::new(42);
    let a = chunk(0, 0, 0);
    let b = chunk(1, 0, 0);
    let content_a = src.populate(a, world_aabb_for(a));
    let content_b = src.populate(b, world_aabb_for(b));

    // Order in `populate`: axis 0 first, direction -1 then +1.
    // For chunk A: index `interior + 1` is the +x face.
    // For chunk B: index `interior + 0` is the -x face.
    // Both reduce to lower-chunk = A on the X axis.
    let interior = src.interior_primitives_per_chunk() as usize;
    let prim_a_plus_x = content_a.primitives[interior + 1];
    let prim_b_minus_x = content_b.primitives[interior];
    assert_eq!(
        prim_a_plus_x, prim_b_minus_x,
        "boundary primitives between adjacent chunks must match byte-for-byte",
    );
}

#[test]
fn boundary_primitive_aabb_straddles_chunk_face() {
    // The +x boundary primitive of chunk (0,0,0) must have an AABB
    // that crosses x = 64 (the face value at level 0).
    let src = ProceduralCitySource::new(13);
    let id = chunk(0, 0, 0);
    let aabb = world_aabb_for(id);
    let content = src.populate(id, aabb);
    let interior = src.interior_primitives_per_chunk() as usize;
    let leaf = content.leaf_aabbs[interior + 1];
    assert!(
        leaf.aabb_min[0] < 64.0 && leaf.aabb_max[0] > 64.0,
        "boundary primitive AABB must straddle the chunk face plane (got [{}, {}])",
        leaf.aabb_min[0],
        leaf.aabb_max[0],
    );
}

#[test]
fn interior_primitives_stay_inside_world_aabb() {
    let src = ProceduralCitySource::new(99);
    let id = chunk(2, 0, -1);
    let aabb = world_aabb_for(id);
    let content = src.populate(id, aabb);
    let interior = src.interior_primitives_per_chunk() as usize;
    for prim in &content.primitives[..interior] {
        let p = Vec3::from_array(prim.position);
        assert!(p.x >= aabb.min.x && p.x <= aabb.max.x);
        assert!(p.y >= aabb.min.y && p.y <= aabb.max.y);
        assert!(p.z >= aabb.min.z && p.z <= aabb.max.z);
    }
}

#[test]
fn max_smoothness_radius_matches_constant() {
    let src = ProceduralCitySource::new(5);
    let id = chunk(0, 0, 0);
    let content = src.populate(id, world_aabb_for(id));
    assert_eq!(content.max_smoothness_radius, SMOOTHNESS_RADIUS);
}

#[test]
fn primitive_types_mix_sphere_box_cylinder() {
    // Sample many chunks and verify all three types appear at least
    // once — guards against a hashing regression that collapses the
    // modulo to a single tag.
    let src = ProceduralCitySource::new(1234);
    let mut seen = [false; 3];
    for x in -3..3 {
        for z in -3..3 {
            let id = chunk(x, 0, z);
            let content = src.populate(id, world_aabb_for(id));
            for prim in &content.primitives {
                match prim.type_tag {
                    TYPE_SPHERE => seen[0] = true,
                    TYPE_BOX => seen[1] = true,
                    TYPE_CYLINDER => seen[2] = true,
                    _ => {}
                }
            }
        }
    }
    assert!(seen.iter().all(|&x| x), "all three primitive types must appear");
}
