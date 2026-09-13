//! Chunk identity and state: a fixed cubic region addressed by grid index and LOD. Heavy per-chunk
//! data is keyed by [`ChunkId`] in its owner's storage, keeping this envelope small.

use glam::{DVec3, IVec3, Vec3};
use kooch_core::Aabb;
use kooch_core::coord::{ActiveOrigin, UniverseCoord};

/// Side length of a level-0 chunk in meters. A chunk at level N has
/// side `BASE_CHUNK_SIZE_METERS << N` — each level doubles the side
/// (and quadruples the surface, octuples the volume covered).
pub const BASE_CHUNK_SIZE_METERS: f64 = 64.0;

/// Highest LOD before the size computation overflows: 64 × 4096 = 262 km a side, past any practical
/// ring.
pub const MAX_LOD_LEVEL: u8 = 12;

/// Equality needs both `coords` and `level`: one index at two levels names two overlapping chunks.
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

    /// AABB relative to [`ActiveOrigin`]; f32 is safe because far chunks unload before they leave
    /// its precision.
    pub fn bounds(&self, origin: &ActiveOrigin) -> Aabb {
        let world = self.world_origin();
        let delta = origin.coord().delta_to(&world);
        let min = delta.as_vec3();
        let s = self.size_meters() as f32;
        Aabb::new(min, min + Vec3::splat(s))
    }
}

/// `Unloaded → Loading → Loaded → Unloading → Unloaded`; the middle states exist before an async
/// loader does, so one lands without API churn.
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

/// Heavy data lives elsewhere keyed by [`ChunkId`], so the active-chunks map stays cheap to iterate
/// per frame.
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
