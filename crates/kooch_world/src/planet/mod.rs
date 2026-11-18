//! Planet terrain residency (#313): which voxel blocks of which planets hold a slot of the shared
//! pool this frame. Decided on the CPU, as Zylann's octree streaming does; the GPU fills what it
//! picks (#313 step 3).
//!
//! Each planet is an octree of fixed-size blocks in its own local space, so a planet that orbits
//! or spins regenerates nothing. Every planet competes for one pool by screen-space error: the
//! planet you stand on takes what the distant ones do not need, and memory stays fixed however
//! many planets exist.

mod pool;
mod select;
#[cfg(test)]
mod tests;

use glam::IVec3;

pub use pool::{BlockPool, Residency};
pub use select::{Limits, Planets, Shape, View, select};

use crate::voxel::{SUBGRID_DIM, SUBGRID_TILE_DIM};

/// Samples per block axis; a level-`L` block covers `BLOCK_SAMPLES · voxel · 2^L` metres.
pub const BLOCK_SAMPLES: u32 = SUBGRID_DIM;

/// Atlas bytes one block costs: a `SUBGRID_TILE_DIM³` tile of `R16Float`.
pub const BLOCK_BYTES: u64 = (SUBGRID_TILE_DIM as u64).pow(3) * 2;

/// Default pool size: 6144 blocks, ≈ 57.6 MiB of atlas.
pub const DEFAULT_CAPACITY: u32 = 6144;

/// One block of one planet: its octree level (0 = finest) and its cell at that level, in blocks
/// from the planet's centre.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BlockKey {
    pub planet: u32,
    pub level: u8,
    pub cell: IVec3,
}
