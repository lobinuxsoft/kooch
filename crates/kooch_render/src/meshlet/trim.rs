//! Static alpha as geometry (#452): a material whose coverage reads uv and textures only is baked
//! once over its uv square and the mesh is cut down to a **hull** around what it covers.
//!
//! The hull is deliberately coarse and always wider than the coverage: the material keeps cutting
//! and blending per pixel inside it, and what the cut saves is the fill of everything the alpha
//! never reached — the empty corners of a leaf card, the space around a sprite. Tracing the alpha
//! exactly would buy the same fill for a mesh nobody wants to pay for.
//!
//! Prior art: Humus' particle trimming and Unity's tight sprite mesh, both of which cover the
//! sprite in a handful of corners rather than following its edge.

pub mod bake;
mod cut;
mod region;

use std::collections::HashMap;

use kooch_core::Guid;

use crate::material::MaterialPipeline;
use crate::meshlet::asset::{DEFAULT_MAX_TRIANGLES, DEFAULT_MAX_VERTICES, MeshletMesh};
use crate::meshlet::builder::{LodConfig, build_meshlets_lod_chain};

/// Texels a side of the baked coverage. A row of R8 is then 256 bytes, the copy alignment.
pub const TRIM_SIDE: u32 = 256;
/// Corners a hull is allowed before it is grown and simplified again.
const HULL_CORNERS: usize = 16;
/// Triangles a cut mesh is allowed. Past it the vertices cost more than the fill they save.
const HULL_TRIANGLES: usize = 32;
/// Share of the mesh's uv a hull may keep and still be worth cutting. A sprite that fills its own
/// square has no empty corners to drop, and a cut would be all cost.
const HULL_SAVING: f32 = 0.9;
/// What counts as covered for a transparent material: anything its alpha is not zero at, because it
/// still blends there. A masked one is a cut, and its bake is already 0 or 1.
const TRANSPARENT_LEVEL: f64 = 1.0 / 255.0;
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

/// Why a pair keeps the mesh it was authored with. Logged, so a scene that expected a cut says what
/// it got instead.
#[derive(Clone, Copy, Debug)]
pub enum NoTrim {
    /// The material's own shader did not bake: it has no surface, or it never compiled.
    Bake,
    /// Nothing is covered at all, so there is no mesh to draw.
    Empty,
    /// The mesh's uv leaves its square: it tiles the coverage, which was baked once.
    Tiled,
    /// The hull needs more triangles than it would save fill.
    Budget,
    /// The coverage fills the mesh: there are no empty corners to drop.
    Cheap,
    /// The cut mesh does not meshletise.
    Meshlets,
}

/// A cut mesh and what it is worth: the share of the source's uv the hull still covers, which is the
/// share of the fill that is left to pay for.
pub struct Trimmed {
    pub mesh: MeshletMesh,
    pub kept: f32,
}

/// Bakes `slot`'s coverage and cuts `source` down to a hull around it.
pub fn build(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    materials: &MaterialPipeline,
    slot: u32,
    source: &MeshletMesh,
) -> Result<Trimmed, NoTrim> {
    let transparent = materials
        .slot_surface(slot)
        .is_some_and(|(_, surface)| surface.kind.blends());
    let threshold = match transparent {
        true => TRANSPARENT_LEVEL,
        false => 0.5,
    };
    let mask = bake::mask(device, queue, materials, slot, TRIM_SIDE).ok_or(NoTrim::Bake)?;
    let hull = region::hull(&mask, TRIM_SIDE, threshold, HULL_CORNERS).ok_or(NoTrim::Empty)?;
    let cut = cut::mesh(source, &hull, TRIM_SIDE, threshold)?;
    if cut.triangles > HULL_TRIANGLES {
        return Err(NoTrim::Budget);
    }
    if cut.kept > cut.whole * HULL_SAVING {
        return Err(NoTrim::Cheap);
    }
    let mesh = build_meshlets_lod_chain(
        &cut.mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig::default(),
    )
    .map_err(|_| NoTrim::Meshlets)?;
    Ok(Trimmed {
        mesh,
        kept: cut.kept / cut.whole.max(f32::EPSILON),
    })
}

#[cfg(test)]
mod tests;
