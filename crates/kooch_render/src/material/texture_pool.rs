//! `MaterialTexturePool` — GPU-resident texture store keyed by asset [`Guid`], plus the
//! per-material bind group the two-pass material shader consumes.

use std::collections::HashMap;

use kooch_core::Guid;

use super::TextureDefault;
use super::shader::MAX_PARAM_TEXTURES;
use super::values::TextureRef;
use crate::texture::{GpuTexture, Image, ImageFormat, Mipmapper};

/// The hardware minimum, which means the feature is off.
pub const NO_ANISOTROPY: u16 = 1;

/// GPU texture registry + per-material bind group factory.
pub struct MaterialTexturePool {
    textures: HashMap<Guid, GpuTexture>,
    fallback_white: GpuTexture,
    fallback_black: GpuTexture,
    fallback_normal: GpuTexture,
    sampler: wgpu::Sampler,
    anisotropy: u16,
    bgl: wgpu::BindGroupLayout,
    /// Owned here because it caches a render pipeline per format, and
    /// this is the one place textures are uploaded from.
    mipmapper: Mipmapper,
}

impl MaterialTexturePool {
    /// Builds the pool with the three 1×1 fallbacks, a filtering sampler, and the per-material bind
    /// group layout (4 textures + 1 sampler). White and black read the same in sRGB and linear.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let solid = |rgba| {
            GpuTexture::upload(
                device,
                queue,
                &Image::solid_color(rgba, ImageFormat::Rgba8Unorm),
            )
        };
        let fallback_white = solid([255, 255, 255, 255]);
        let fallback_black = solid([0, 0, 0, 255]);
        let fallback_normal = solid([128, 128, 255, 255]);

        let sampler = create_sampler(device, NO_ANISOTROPY);

        let bgl = Self::bind_group_layout(device);

        Self {
            textures: HashMap::new(),
            fallback_white,
            fallback_black,
            fallback_normal,
            mipmapper: Mipmapper::new(device),
            sampler,
            anisotropy: NO_ANISOTROPY,
            bgl,
        }
    }

    /// Per-material bind group layout: albedo(0), normal(1), metal_roughness(2) textures, sampler(3)
    /// and a shader's fourth texture(4).
    pub fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        let texture_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material_texture_bgl"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                texture_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture_entry(4),
            ],
        })
    }

    /// Replaces the sampler with one taking `samples` along the long axis of a footprint, and
    /// reports whether anything changed.
    pub fn set_anisotropy(&mut self, device: &wgpu::Device, samples: u16) -> bool {
        let samples = samples.max(NO_ANISOTROPY);
        if samples == self.anisotropy {
            return false;
        }
        self.anisotropy = samples;
        self.sampler = create_sampler(device, samples);
        true
    }

    /// What the sampler currently takes.
    pub fn anisotropy(&self) -> u16 {
        self.anisotropy
    }

    /// Uploads `image` under `guid`, replacing any prior texture for that
    /// GUID (hot-reload friendly). Idempotent per content.
    pub fn register(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        guid: Guid,
        image: &Image,
    ) {
        let texture = GpuTexture::upload_with(device, queue, image, &mut self.mipmapper);
        self.textures.insert(guid, texture);
    }

    /// Drops the texture for `guid`, so the next sync uploads it again.
    pub fn evict(&mut self, guid: Guid) -> bool {
        self.textures.remove(&guid).is_some()
    }

    /// True if a texture is already uploaded for `guid`.
    pub fn contains(&self, guid: Guid) -> bool {
        self.textures.contains_key(&guid)
    }

    /// Number of registered (non-fallback) textures.
    pub fn len(&self) -> usize {
        self.textures.len()
    }

    /// Whether any non-fallback texture is registered.
    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }

    /// The texture `slot` names, or its fallback when unassigned or not uploaded yet.
    fn view_or_fallback(&self, slot: TextureRef) -> &wgpu::TextureView {
        let fallback = match slot.fallback {
            TextureDefault::White => &self.fallback_white,
            TextureDefault::Black => &self.fallback_black,
            TextureDefault::Normal => &self.fallback_normal,
        };
        slot.guid
            .and_then(|g| self.textures.get(&g))
            .map(|t| &t.view)
            .unwrap_or(&fallback.view)
    }

    /// Builds the per-material bind group. Every slot binds something, so the shader samples all of
    /// them unconditionally.
    pub fn material_bind_group(
        &self,
        device: &wgpu::Device,
        slots: &[TextureRef; MAX_PARAM_TEXTURES as usize],
    ) -> wgpu::BindGroup {
        let texture = |binding: u32, slot: TextureRef| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(self.view_or_fallback(slot)),
        };
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material_texture_bg"),
            layout: &self.bgl,
            entries: &[
                texture(0, slots[0]),
                texture(1, slots[1]),
                texture(2, slots[2]),
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                texture(4, slots[3]),
            ],
        })
    }

    /// The per-material bind group layout, for pipeline construction.
    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.bgl
    }
}

#[cfg(test)]
mod tests;

/// The material sampler, at a given anisotropy.
fn create_sampler(device: &wgpu::Device, anisotropy: u16) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("material_texture_sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        anisotropy_clamp: anisotropy.max(NO_ANISOTROPY),
        ..Default::default()
    })
}
