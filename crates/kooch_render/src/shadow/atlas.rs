//! The shadow depth array: one texture, one cascade per layer.

use crate::meshlet::MeshletCull;

use super::cascades::{CASCADE_COUNT, Cascade};

/// Depth format for the atlas.
pub const SHADOW_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Side of one layer, in texels.
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
    pub fn new(
        device: &wgpu::Device,
        cascade_size: u32,
        initial_capacity: u32,
        max_triangles_per_meshlet: u32,
    ) -> Self {
        let cascade_size = cascade_size.max(1);
        // Cascades first, then one layer per spot light that can cast
        // (#777). Cascades keep layers 0..CASCADE_COUNT so a capture of
        // the array still reads near-to-far from the top.
        let layers = (CASCADE_COUNT + kooch_lighting::MAX_SPOT_SHADOWS) as u32;
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

        // One cull per layer, cascades and spots alike: they could
        // share, and that would serialise the pass, because each one's
        // indirect draw reads the survivor list the next one writes.
        let culls = (0..layers)
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

    /// Which layer spot-shadow `slot` renders into: behind the cascades.
    pub fn spot_layer(slot: usize) -> u32 {
        (CASCADE_COUNT + slot) as u32
    }

    /// The depth target for one spot light's shadow.
    pub fn spot_layer_view(&self, slot: usize) -> &wgpu::TextureView {
        &self.layer_views[(CASCADE_COUNT + slot).min(self.layer_views.len() - 1)]
    }

    /// The cull for one spot light's shadow. Lives behind the cascades'
    /// in the same list, on the same index scheme as the layers.
    pub fn spot_cull(&self, slot: usize) -> &MeshletCull {
        &self.culls[(CASCADE_COUNT + slot).min(self.culls.len() - 1)]
    }

    pub fn cull(&self, cascade: usize) -> &MeshletCull {
        &self.culls[cascade.min(CASCADE_COUNT - 1)]
    }

    /// Grows every cascade's survivor lists to fit the scene.
    pub fn ensure_capacity(&mut self, device: &wgpu::Device, meshlets: u32, groups: u32) {
        for cull in &mut self.culls {
            cull.ensure_capacity(device, meshlets);
            cull.ensure_group_capacity(device, groups);
        }
    }

    /// Packs placed cascades into the records the shading model reads.
    pub fn gpu_cascades(
        &self,
        cascades: &[Cascade; CASCADE_COUNT],
    ) -> [kooch_lighting::GpuCascade; kooch_lighting::FRAME_CASCADE_COUNT] {
        gpu_cascade_layers(cascades)
    }
}

/// The packing, free of the atlas so it is testable without a device.
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
