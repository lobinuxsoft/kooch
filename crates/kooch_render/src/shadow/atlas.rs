//! The shadow depth atlas: one texture, one cascade per quadrant.
//!
//! # Why an atlas and not four textures
//!
//! The shading pass samples whichever cascade covers the fragment, and
//! which one that is varies per pixel. Four separate textures would mean
//! four bindings and a dynamic index into them, which WGSL only offers
//! through binding arrays. One texture with four viewports is a single
//! binding and a scale-and-bias on the uv — and, on the bind-group
//! budget this engine has left, one binding is the difference between
//! fitting and not.
//!
//! # Why four culls
//!
//! Each cascade culls from its own light-space frustum, so each needs
//! its own survivor list. They could share one `MeshletCull` used four
//! times, and that would serialise the whole pass: cascade 0's indirect
//! draw reads the same `visible_meshlets` that cascade 1's cull writes,
//! so wgpu inserts a barrier between every pair. Four culls are buffers
//! — no textures — and let the cascades overlap on the GPU.

use crate::meshlet::MeshletCull;

use super::cascades::{CASCADE_COUNT, Cascade};

/// Depth format for the atlas.
///
/// `Depth32Float` rather than the 16-bit variant: the comparison happens
/// against a reconstructed world position, and 16 bits of depth over a
/// cascade that can span hundreds of metres quantises into visible
/// stair-stepping on the shadow of anything at a shallow angle.
pub const SHADOW_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Side of one cascade, in texels. The atlas is twice this on each axis.
///
/// 2048 per cascade means a 4096² atlas — 64 MiB at `Depth32Float`. That
/// is a real cost and it is the default because the alternative is
/// visible: at 1024 the near cascade is already soft enough that contact
/// shadows look detached.
pub const DEFAULT_CASCADE_SIZE: u32 = 2048;

/// Where a cascade lives in the atlas, in texels.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AtlasRegion {
    pub x: u32,
    pub y: u32,
    pub size: u32,
}

impl AtlasRegion {
    /// Scale and bias that maps a cascade's `[0,1]` shadow uv into the
    /// atlas's `[0,1]`. The shader multiplies then adds; packing it this
    /// way keeps the sample site to one `fma` rather than a branch on
    /// which quadrant.
    pub fn uv_scale_bias(&self, atlas_size: u32) -> [f32; 4] {
        let atlas = atlas_size.max(1) as f32;
        let scale = self.size as f32 / atlas;
        [scale, scale, self.x as f32 / atlas, self.y as f32 / atlas]
    }
}

/// The atlas texture, its per-cascade culls, and where each cascade sits.
pub struct ShadowAtlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// One per cascade. See the module docs for why these are not one.
    culls: Vec<MeshletCull>,
    cascade_size: u32,
    regions: [AtlasRegion; CASCADE_COUNT],
}

impl ShadowAtlas {
    /// Allocates the atlas and one cull per cascade.
    ///
    /// `initial_capacity` sizes the survivor lists; they grow later like
    /// every other buffer here, so a low guess costs a reallocation
    /// rather than a panic. The cull pipelines are shared and live on
    /// the render stage — nine compute pipelines per cascade is exactly
    /// what `MeshletCullPipelines` exists to avoid.
    pub fn new(
        device: &wgpu::Device,
        cascade_size: u32,
        initial_capacity: u32,
        max_triangles_per_meshlet: u32,
    ) -> Self {
        let cascade_size = cascade_size.max(1);
        let atlas_size = cascade_size * 2;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow_atlas"),
            size: wgpu::Extent3d {
                width: atlas_size,
                height: atlas_size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let culls = (0..CASCADE_COUNT)
            .map(|_| MeshletCull::new(device, initial_capacity.max(1), max_triangles_per_meshlet))
            .collect();

        Self {
            texture,
            view,
            culls,
            cascade_size,
            regions: quadrants(cascade_size),
        }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub fn cascade_size(&self) -> u32 {
        self.cascade_size
    }

    pub fn atlas_size(&self) -> u32 {
        self.cascade_size * 2
    }

    pub fn region(&self, cascade: usize) -> AtlasRegion {
        self.regions[cascade.min(CASCADE_COUNT - 1)]
    }

    pub fn cull(&self, cascade: usize) -> &MeshletCull {
        &self.culls[cascade.min(CASCADE_COUNT - 1)]
    }

    pub fn cull_mut(&mut self, cascade: usize) -> &mut MeshletCull {
        let index = cascade.min(CASCADE_COUNT - 1);
        &mut self.culls[index]
    }

    /// Grows every cascade's survivor lists to fit the scene.
    ///
    /// All four, unconditionally: a cascade that culls nothing this
    /// frame still dispatches one thread per instance-meshlet pair, and
    /// sizing only the ones that drew last frame is how the 257th
    /// instance panics in the cascade nobody was looking at.
    pub fn ensure_capacity(&mut self, device: &wgpu::Device, meshlets: u32, groups: u32) {
        for cull in &mut self.culls {
            cull.ensure_capacity(device, meshlets);
            cull.ensure_group_capacity(device, groups);
        }
    }

    /// Packs placed cascades into the records the shading model reads.
    ///
    /// The atlas does this rather than `cascades.rs` because the uv
    /// transform is the atlas's own layout — a cascade knows where it
    /// is in the world and nothing about which quadrant it landed in.
    pub fn gpu_cascades(
        &self,
        cascades: &[Cascade; CASCADE_COUNT],
    ) -> [kooch_lighting::GpuCascade; kooch_lighting::FRAME_CASCADE_COUNT] {
        let atlas = self.atlas_size();
        std::array::from_fn(|i| kooch_lighting::GpuCascade {
            view_proj: cascades[i].view_proj.to_cols_array_2d(),
            uv_scale_bias: self.regions[i].uv_scale_bias(atlas),
            far_depth: cascades[i].far_depth,
            texel_world_size: cascades[i].texel_world_size,
            depth_extent: cascades[i].depth_extent,
            _pad0: 0.0,
        })
    }

    /// Bytes the atlas occupies, for the VRAM tracker.
    pub fn byte_size(&self) -> u64 {
        let side = self.atlas_size() as u64;
        side * side * 4
    }
}

/// Quadrant layout: cascade 0 top-left, 1 top-right, 2 bottom-left,
/// 3 bottom-right.
///
/// Reading order, so a capture of the atlas is inspectable by eye — the
/// near cascade is the small tight one at the top left, and if it is not
/// there, the split scheme is upside down.
fn quadrants(cascade_size: u32) -> [AtlasRegion; CASCADE_COUNT] {
    let mut regions = [AtlasRegion {
        x: 0,
        y: 0,
        size: cascade_size,
    }; CASCADE_COUNT];
    for (i, region) in regions.iter_mut().enumerate() {
        region.x = (i as u32 % 2) * cascade_size;
        region.y = (i as u32 / 2) * cascade_size;
    }
    regions
}

#[cfg(test)]
mod tests;
