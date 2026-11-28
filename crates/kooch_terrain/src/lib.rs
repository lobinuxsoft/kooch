//! Planets on sparse voxel blocks (#313). Put a [`Terrain`] on an entity and [`PlanetPlugin`] draws
//! it: which blocks of which planets hold a slot of the shared pool is decided on the CPU, as
//! Zylann's octree streaming does; the GPU generates and meshes them into one [`BlockAtlas`].
//!
//! Each planet is an octree of fixed-size blocks in its own local space, so a planet that orbits
//! or spins regenerates nothing. Every planet competes for one pool by screen-space error: the
//! planet you stand on takes what the distant ones do not need, and memory stays fixed however
//! many planets exist.

mod atlas;
mod budget;
mod component;
mod draw;
mod eval;
mod generate;
// Only physics consumes the CPU mesh; the tests pin it either way.
#[cfg(any(feature = "physics", test))]
mod ground;
mod lod;
mod mesh;
mod plugin;
mod pool;
mod recent;
mod rerun;
mod roster;
mod seam;
mod sight;
mod stamp;
mod sync;
mod terrain;
#[cfg(test)]
mod test_device;
#[cfg(test)]
mod tests;

use glam::IVec3;

pub use atlas::BlockAtlas;
pub use component::Terrain;
pub use eval::Generator;
pub use generate::{
    BARE_SPHERE, Backlog, GeneratePass, Job, LAYERS, PLANET_INPUT_WGSL, validate_generator,
};
#[cfg(feature = "physics")]
pub use ground::{
    BlockState, BlockView, GroundColliders, GroundInvoker, GroundView, GroundWire, release_ground,
    sync_ground,
};
pub use lod::{Base, Limits, Planets, Shape, View, choose_lods};
pub use mesh::{CountJob, Cuts, MeshPass, Part, WriteJob};
pub use plugin::PlanetPlugin;
pub use pool::{BlockPool, Residency};
pub use recent::{BatchMean, RECENT};
pub use seam::{Leaves, Near};
pub use sight::Cone;
pub use stamp::{
    GpuStamp, OPERATION_ADD, OPERATION_INTERSECT, OPERATION_SUBTRACT, SHAPE_BOX, SHAPE_SPHERE,
    Stamp, StampBuffer,
};
pub use sync::{GroundHit, PlanetStats, PlanetTerrain, PlanetViewers, ground_hits, sync_planets};
pub use terrain::{CountedBlock, GpuTimes, MESH_BATCH, Pipeline};

use kooch_world::voxel::SUBGRID_DIM;

/// Samples per block axis; a level-`L` block covers `BLOCK_SAMPLES · voxel · 2^L` metres.
pub const BLOCK_SAMPLES: u32 = SUBGRID_DIM;

/// Samples per tile axis: the block's, plus one either side, so neighbours share the cells across
/// their border and the mesh closes there.
pub const TILE_SAMPLES: u32 = BLOCK_SAMPLES + 2;

/// Atlas bytes one block costs: a `TILE_SAMPLES³` tile of `R16Float`.
pub const BLOCK_BYTES: u64 = (TILE_SAMPLES as u64).pow(3) * 2;

/// Default pool size: 6144 blocks, ≈ 68.3 MiB of atlas.
pub const DEFAULT_CAPACITY: u32 = 6144;

/// One block of one planet: its octree level (0 = finest) and its cell at that level, in blocks
/// from the planet's centre.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BlockKey {
    pub planet: u32,
    pub level: u8,
    pub cell: IVec3,
}

/// Corner `i` in `0..8` of a unit cube, x fastest.
pub(crate) fn octant(i: i32) -> IVec3 {
    IVec3::new(i & 1, (i >> 1) & 1, (i >> 2) & 1)
}

impl BlockKey {
    /// The block one level coarser that holds this one.
    pub fn parent(self) -> Self {
        Self {
            planet: self.planet,
            level: self.level.saturating_add(1),
            cell: self.cell.div_euclid(IVec3::splat(2)),
        }
    }

    /// Child `i` in `0..8`, x fastest.
    pub fn child(self, i: i32) -> Self {
        Self {
            planet: self.planet,
            level: self.level.saturating_sub(1),
            cell: self.cell * 2 + octant(i),
        }
    }

    /// Every coarser block holding this one, nearest first, up to the planet's `root` level.
    pub fn ancestors(self, root: u8) -> impl Iterator<Item = BlockKey> {
        std::iter::successors(Some(self), move |key| {
            (key.level < root).then(|| key.parent())
        })
        .skip(1)
    }
}
