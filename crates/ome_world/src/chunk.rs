//! Chunk types — identity, state, per-chunk data.
//!
//! A chunk is a fixed-size cubic region of the world identified by a
//! 3D grid index plus a level of detail. The grid is rectangular for
//! simplicity; a sphere-surface wrapper (cubed-sphere quadtree) sits on
//! top of these types as a separate addressing layer when planets land,
//! without rewriting any of the streaming machinery.
//!
//! Heavy per-chunk fields (sparse SDF baseline, RPN delta tree, BVH)
//! intentionally do NOT live in [`ChunkData`] — they belong to #136
//! (sparse storage), #307 (RPN tree, already merged), and #115 (BVH).
//! This module owns the minimal envelope the streaming subsystem
//! manages.

use glam::{DVec3, IVec3, Vec3};
use ome_core::coord::{ActiveOrigin, UniverseCoord};

/// Side length of a level-0 chunk in meters. A chunk at level N has
/// side `BASE_CHUNK_SIZE_METERS << N` — each level doubles the side
/// (and quadruples the surface, octuples the volume covered).
pub const BASE_CHUNK_SIZE_METERS: f64 = 64.0;

/// Highest LOD level the API supports without overflowing the size
/// computation. At level 12 a chunk side is 64 × 4096 = 262 km, which
/// covers an Earth-radius planet in ~23 chunks per axis — far past any
/// practical LOD ring.
pub const MAX_LOD_LEVEL: u8 = 12;

/// Identifier of a chunk in the world spatial grid.
///
/// Equality requires both `coords` AND `level`: the same grid index at
/// two different levels names two different chunks (covering
/// overlapping volumes at different resolutions).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChunkId {
    /// Grid index along each axis. The chunk's lower corner sits at
    /// `coords * size_meters()` in world coords.
    pub coords: IVec3,
    /// LOD level. `0` = highest detail; each step doubles the side
    /// length (halves the linear resolution).
    pub level: u8,
}

impl ChunkId {
    pub const fn new(coords: IVec3, level: u8) -> Self {
        Self { coords, level }
    }

    /// Side length of this chunk in meters.
    pub fn size_meters(&self) -> f64 {
        BASE_CHUNK_SIZE_METERS * (1u64 << self.level) as f64
    }

    /// Lower corner of this chunk in absolute universe coordinates.
    /// Useful when crossing sector boundaries; most consumers want
    /// [`Self::bounds`] instead.
    pub fn world_origin(&self) -> UniverseCoord {
        let s = self.size_meters();
        let world = DVec3::new(
            self.coords.x as f64 * s,
            self.coords.y as f64 * s,
            self.coords.z as f64 * s,
        );
        UniverseCoord::from_dvec3(world)
    }

    /// Axis-aligned bounding box of this chunk in the simulation frame
    /// (relative to [`ActiveOrigin`]). f32 is safe here because loaded
    /// chunks are by construction near the active origin — far chunks
    /// are unloaded before they leave f32 precision.
    pub fn bounds(&self, origin: &ActiveOrigin) -> Aabb {
        let world = self.world_origin();
        let delta = origin.coord().delta_to(&world);
        let min = delta.as_vec3();
        let s = self.size_meters() as f32;
        Aabb {
            min,
            max: min + Vec3::splat(s),
        }
    }
}

/// Axis-aligned bounding box in the simulation frame (camera-relative
/// coords; f32 is sufficient near the active origin).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    /// Squared distance from `point` to the closest boundary of the box.
    /// Returns `0.0` for points inside.
    pub fn distance_squared(&self, point: Vec3) -> f32 {
        let clamped = point.clamp(self.min, self.max);
        (point - clamped).length_squared()
    }
}

/// Lifecycle state of a chunk in memory.
///
/// State transitions: `Unloaded → Loading → Loaded → Unloading → Unloaded`.
/// Loading / Unloading are kept as explicit states even though no async
/// loader exists yet — once one lands (#54 follow-up issue) the
/// transitions stay valid without API churn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChunkState {
    /// Not present in memory.
    Unloaded,
    /// Load in progress. `progress` is `[0.0, 1.0]` for UI / debug HUDs.
    Loading { progress: f32 },
    /// Live in memory; eligible for queries / render.
    Loaded,
    /// Unload in progress (e.g. flushing baked edits to disk before
    /// page-out).
    Unloading,
}

/// Per-chunk envelope managed by the streaming subsystem.
///
/// Heavy data (sparse SDF baseline, RPN delta tree, BVH) is owned by
/// other crates / issues and keyed by [`ChunkId`] in their own storage.
/// This struct stays small so the active-chunks `HashMap` is cheap to
/// iterate per frame.
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkData {
    pub id: ChunkId,
    pub state: ChunkState,
    /// Frame index when this chunk was last touched by the activation
    /// system. Used as an LRU-like signal by eviction policies.
    pub last_seen_frame: u64,
}

impl ChunkData {
    pub fn new(id: ChunkId) -> Self {
        Self {
            id,
            state: ChunkState::Unloaded,
            last_seen_frame: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-3;

    #[test]
    fn chunk_size_doubles_per_level() {
        let l0 = ChunkId::new(IVec3::ZERO, 0);
        let l1 = ChunkId::new(IVec3::ZERO, 1);
        let l3 = ChunkId::new(IVec3::ZERO, 3);
        assert!((l0.size_meters() - BASE_CHUNK_SIZE_METERS).abs() < 1e-6);
        assert!((l1.size_meters() - BASE_CHUNK_SIZE_METERS * 2.0).abs() < 1e-6);
        assert!((l3.size_meters() - BASE_CHUNK_SIZE_METERS * 8.0).abs() < 1e-6);
    }

    #[test]
    fn id_equality_requires_both_fields() {
        let a = ChunkId::new(IVec3::new(1, 2, 3), 0);
        let b = ChunkId::new(IVec3::new(1, 2, 3), 0);
        let c = ChunkId::new(IVec3::new(1, 2, 3), 1);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn id_hashable_for_hashmap() {
        use std::collections::HashMap;
        let mut m: HashMap<ChunkId, u32> = HashMap::new();
        m.insert(ChunkId::new(IVec3::new(0, 0, 0), 0), 42);
        assert_eq!(m.get(&ChunkId::new(IVec3::new(0, 0, 0), 0)), Some(&42));
        // Same coords, different level → not the same chunk.
        assert_eq!(m.get(&ChunkId::new(IVec3::new(0, 0, 0), 1)), None);
    }

    #[test]
    fn world_origin_uses_chunk_size() {
        let id = ChunkId::new(IVec3::new(2, -1, 0), 0);
        let world = id.world_origin().to_dvec3();
        assert!((world.x - 2.0 * BASE_CHUNK_SIZE_METERS).abs() < 1e-3);
        assert!((world.y - -1.0 * BASE_CHUNK_SIZE_METERS).abs() < 1e-3);
        assert!(world.z.abs() < 1e-3);
    }

    #[test]
    fn bounds_at_zero_origin_match_world_origin() {
        let origin = ActiveOrigin::ZERO;
        let id = ChunkId::new(IVec3::ZERO, 0);
        let b = id.bounds(&origin);
        assert!((b.min - Vec3::ZERO).length() < EPS);
        assert!((b.max - Vec3::splat(BASE_CHUNK_SIZE_METERS as f32)).length() < EPS);
    }

    #[test]
    fn bounds_shift_with_active_origin() {
        // Chunk at world (0,0,0); active origin shifted +100 m on x.
        // The chunk should appear at -100 m in the simulation frame.
        let origin = ActiveOrigin::new(UniverseCoord::from_dvec3(DVec3::new(100.0, 0.0, 0.0)));
        let id = ChunkId::new(IVec3::ZERO, 0);
        let b = id.bounds(&origin);
        assert!((b.min.x - (-100.0)).abs() < EPS);
        assert!((b.max.x - (-100.0 + BASE_CHUNK_SIZE_METERS as f32)).abs() < EPS);
    }

    #[test]
    fn chunk_state_default_is_unloaded() {
        let data = ChunkData::new(ChunkId::new(IVec3::ZERO, 0));
        assert_eq!(data.state, ChunkState::Unloaded);
        assert_eq!(data.last_seen_frame, 0);
    }

    #[test]
    fn aabb_center_and_extents() {
        let b = Aabb {
            min: Vec3::new(0.0, 0.0, 0.0),
            max: Vec3::new(2.0, 4.0, 6.0),
        };
        assert_eq!(b.center(), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(b.extents(), Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn aabb_distance_squared_inside_is_zero() {
        let b = Aabb {
            min: Vec3::ZERO,
            max: Vec3::splat(10.0),
        };
        assert!(b.distance_squared(Vec3::splat(5.0)) < 1e-6);
    }

    #[test]
    fn aabb_distance_squared_outside_is_corner_distance() {
        let b = Aabb {
            min: Vec3::ZERO,
            max: Vec3::splat(10.0),
        };
        // Point at (-3, -4, 0) → closest is (0,0,0), distance² = 25.
        let d2 = b.distance_squared(Vec3::new(-3.0, -4.0, 0.0));
        assert!((d2 - 25.0).abs() < 1e-3);
    }
}
