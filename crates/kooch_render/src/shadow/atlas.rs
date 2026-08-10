//! The shadow depth array: one texture, one cascade per layer.
//!
//! # Why an array and not an atlas
//!
//! This used to be a single 2×2 atlas with a cascade in each quadrant,
//! on the stated belief that several shadow maps would mean several
//! bindings "and a dynamic index into them, which WGSL only offers
//! through binding arrays".
//!
//! 🔴 That conflated two different things. A `texture_depth_2d_array` is
//! **one** texture and **one** binding, and the layer is an ordinary
//! argument to `textureSampleCompareLevel` — binding arrays are a
//! separate feature and are not involved. Bevy has always bound its
//! shadow maps this way (`directional_shadow_textures:
//! texture_depth_2d_array`).
//!
//! The cost of the mistake was that the atlas is full at four cascades:
//! a spot light (#777) had nowhere to go, when in Bevy it is one more
//! layer. Layers also grow without the texture getting quadratically
//! larger, which is what a 4096² atlas does the moment it needs a fifth
//! occupant.
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

/// Side of one layer, in texels.
///
/// 2048 per cascade over four layers is 64 MiB at `Depth32Float` — the
/// same as the 4096² atlas it replaced, because the pixel count is
/// identical. It is the default because the alternative is visible: at
/// 1024 the near cascade is already soft enough that contact shadows
/// look detached.
pub const DEFAULT_CASCADE_SIZE: u32 = 2048;

/// The atlas texture, its per-cascade culls, and where each cascade sits.
pub struct ShadowAtlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// One per cascade. See the module docs for why these are not one.
    culls: Vec<MeshletCull>,
    cascade_size: u32,
    /// One view per layer, because a render attachment is a single
    /// layer: the array view above is for sampling and cannot be a
    /// depth target.
    layer_views: Vec<wgpu::TextureView>,
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
        let layers = CASCADE_COUNT as u32;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow_atlas"),
            size: wgpu::Extent3d {
                width: cascade_size,
                height: cascade_size,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("shadow_atlas_array_view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let layer_views = (0..layers)
            .map(|layer| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("shadow_atlas_layer_view"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();

        let culls = (0..CASCADE_COUNT)
            .map(|_| MeshletCull::new(device, initial_capacity.max(1), max_triangles_per_meshlet))
            .collect();

        Self {
            texture,
            view,
            culls,
            cascade_size,
            layer_views,
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

    /// The depth target for one cascade's raster pass.
    pub fn layer_view(&self, cascade: usize) -> &wgpu::TextureView {
        &self.layer_views[cascade.min(CASCADE_COUNT - 1)]
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
        gpu_cascade_layers(cascades)
    }
}

/// The packing, free of the atlas so it is testable without a device.
///
/// Cascade `i` renders into layer `i`. It is the identity today and it
/// is still written down, because the moment spot lights take layers
/// behind the cascades (#777) this is the function that has to keep them
/// apart.
pub fn gpu_cascade_layers(
    cascades: &[Cascade; CASCADE_COUNT],
) -> [kooch_lighting::GpuCascade; kooch_lighting::FRAME_CASCADE_COUNT] {
    {
        std::array::from_fn(|i| kooch_lighting::GpuCascade {
            view_proj: cascades[i].view_proj.to_cols_array_2d(),
            layer: i as u32,
            _pad_layer: [0; 3],
            far_depth: cascades[i].far_depth,
            texel_world_size: cascades[i].texel_world_size,
            depth_extent: cascades[i].depth_extent,
            _pad0: 0.0,
        })
    }
}

impl ShadowAtlas {
    /// Bytes the array occupies, for the VRAM tracker.
    pub fn byte_size(&self) -> u64 {
        let side = self.cascade_size as u64;
        side * side * 4 * self.layer_views.len() as u64
    }
}

#[cfg(test)]
mod tests;
