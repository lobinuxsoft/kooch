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
use kooch_core::Aabb;
use kooch_core::coord::{ActiveOrigin, UniverseCoord};

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
        Aabb::new(min, min + Vec3::splat(s))
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
mod tests;
