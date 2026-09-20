//! Static alpha as geometry (#452): a masked material whose cut reads uv and textures only is baked
//! once over its uv square, contoured, and the mesh is cut against the contour. What is left is
//! plain opaque geometry — no per-pixel discard, no masked bin, meshlet LODs and a solid shadow.
//!
//! Prior art: Humus' particle trimming, Unity's tight sprite mesh, and Epic's advice to model
//! Nanite foliage rather than mask it.

mod bake;
mod cut;
mod region;

use std::collections::HashMap;

use kooch_core::Guid;

use crate::material::MaterialPipeline;
use crate::meshlet::asset::{DEFAULT_MAX_TRIANGLES, DEFAULT_MAX_VERTICES, MeshletMesh};
use crate::meshlet::builder::{LodConfig, build_meshlets_lod_chain};

/// Texels a side of the baked cut. A row of R8 is then 256 bytes, the copy alignment, and the
/// contour lands within half a texel of the edge the raster would have cut.
pub const TRIM_SIDE: u32 = 256;
/// Frames a pair has to ask for the same trim before it bakes. A dragged slider republishes the
/// material every frame; without this it would bake, and read back, on each one.
const SETTLE_FRAMES: u32 = 8;

/// What a (mesh, material) pair became, and the material stamp it was cut against.
struct Trim {
    stamp: u64,
    /// The trimmed mesh, or `None` for a pair that cannot be cut: it keeps the masked raster.
    mesh: Option<Guid>,
}

/// A (mesh, material) pair that wants trimming, as [`AlphaTrim::next`] takes them.
#[derive(Clone, Copy)]
pub struct TrimPair {
    pub mesh: Guid,
    pub material: Guid,
    pub slot: u32,
    /// [`MaterialPipeline::slot_stamp`]: a cut is only valid for the values it was baked from.
    pub stamp: u64,
}

/// Which mesh each masked pair draws instead, cached for the session.
#[derive(Default)]
pub struct AlphaTrim {
    trims: HashMap<(Guid, Guid), Trim>,
    /// The pair asking, its stamp, and how many frames the stamp has held.
    settling: Option<((Guid, Guid), u64, u32)>,
}

impl AlphaTrim {
    /// The mesh that replaces `mesh` for `material`, once one has been cut.
    pub fn mesh_for(&self, mesh: Guid, material: Guid) -> Option<Guid> {
        self.trims.get(&(mesh, material))?.mesh
    }

    /// The pair to cut this frame: the first whose trim is missing or stale, once its material has
    /// held still for [`SETTLE_FRAMES`]. One per frame — each costs a readback.
    pub fn next(&mut self, pairs: &[TrimPair]) -> Option<(Guid, Guid, u32, u64)> {
        let pair = pairs.iter().find(|pair| self.stale(pair))?;
        let key = (pair.mesh, pair.material);
        let held =
            matches!(self.settling, Some((at, stamp, _)) if at == key && stamp == pair.stamp);
        let frames = match (held, self.settling) {
            (true, Some((_, _, frames))) => frames + 1,
            _ => 0,
        };
        self.settling = Some((key, pair.stamp, frames));
        (frames >= SETTLE_FRAMES).then_some((pair.mesh, pair.material, pair.slot, pair.stamp))
    }

    /// Records what the pair cut into, so it is neither asked for nor retried.
    pub fn remember(&mut self, mesh: Guid, material: Guid, stamp: u64, trimmed: Option<Guid>) {
        self.trims.insert(
            (mesh, material),
            Trim {
                stamp,
                mesh: trimmed,
            },
        );
        self.settling = None;
    }

    /// Whether the pair has no trim, or one cut from other values.
    fn stale(&self, pair: &TrimPair) -> bool {
        self.trims
            .get(&(pair.mesh, pair.material))
            .is_none_or(|trim| trim.stamp != pair.stamp)
    }
}

/// Why a pair stays with its per-pixel cut. Logged, so a scene that expected geometry says what it
/// got instead.
#[derive(Clone, Copy, Debug)]
pub enum NoTrim {
    /// The material's own shader did not bake: it has no surface, or it never compiled.
    Bake,
    /// The cut keeps nothing at all, so there is no mesh to draw.
    Empty,
    /// The mesh's uv leaves its square: it tiles the coverage, which was baked once.
    Tiled,
    /// The cut mesh does not meshletise.
    Meshlets,
}

/// Bakes `slot`'s cut and cuts `source` against it.
pub fn build(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    materials: &MaterialPipeline,
    slot: u32,
    source: &MeshletMesh,
) -> Result<MeshletMesh, NoTrim> {
    let mask = bake::mask(device, queue, materials, slot, TRIM_SIDE).ok_or(NoTrim::Bake)?;
    let coverage = region::coverage(&mask, TRIM_SIDE).ok_or(NoTrim::Empty)?;
    let geometry = cut::mesh(source, &coverage, &mask, TRIM_SIDE)?;
    build_meshlets_lod_chain(
        &geometry,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig::default(),
    )
    .map_err(|_| NoTrim::Meshlets)
}

#[cfg(test)]
mod tests;
