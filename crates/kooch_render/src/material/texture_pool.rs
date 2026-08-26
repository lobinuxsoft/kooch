//! `MaterialTexturePool` — GPU-resident texture store keyed by asset
//! [`Guid`], plus the per-material bind group the two-pass material
//! shader consumes.
//!
//! # Why not a bindless array
//!
//! The meshlet material path follows Bevy's two-pass model: one fragment
//! pass resolves each pixel's `material_id` into a depth target, then one
//! pass **per registered material** shades the pixels that survive a
//! hardware depth test. Each material pass binds *its own* three
//! textures through a standard bind group — no `binding_array`, no
//! non-uniform indexing (both fragile across drivers). This pool owns the
//! uploaded [`GpuTexture`]s and hands out that per-material bind group.
//!
//! # Branch-free fallbacks
//!
//! A material may reference none of the three maps. Rather than branch in
//! the shader, every slot always binds *something*: a 1×1 fallback whose
//! sampled value is the identity for that channel's math —
//! - albedo → white (`base_color * 1 = base_color`)
//! - metal/roughness → white (`scalar * 1 = scalar`)
//! - normal → flat `[128,128,255]` (decodes to `(0,0,1)`, the geometric
//!   normal, i.e. no perturbation)
//!
//! so the shader samples unconditionally and the absent-texture case
//! costs one texture fetch, never a divergent branch.

use std::collections::HashMap;

use kooch_core::Guid;

use crate::texture::{GpuTexture, Image, ImageFormat, Mipmapper};

/// Which PBR channel a texture feeds. Selects the matching fallback and
/// documents the expected color space at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureSlot {
    /// Base color / albedo — sRGB.
    Albedo,
    /// Tangent-space normal map — linear.
    Normal,
    /// Packed metal (B) + roughness (G), glTF convention — linear.
    MetalRoughness,
}

/// The hardware minimum, which means the feature is off.
///
/// 🔴 This is the setting that actually improves a floor, and it is not
/// the mip bias. A surface at a grazing angle covers a footprint that is
/// long and thin, and an isotropic filter has one number for it: it
/// takes the LONG axis, picks the level that would not alias there, and
/// blurs the short axis by the same amount. That is why a tiled floor
/// goes soft towards the horizon while a wall facing the camera stays
/// sharp — the level is right for one axis and wrong for the other.
/// Anisotropic filtering takes several samples along the long axis
/// instead of one coarse one.
pub const NO_ANISOTROPY: u16 = 1;

/// GPU texture registry + per-material bind group factory.
///
/// CPU-side coordination structure (populated at asset-load / sync time,
/// queried when building material passes), so a `HashMap<Guid, _>` is the
/// right tool — this never runs in a GPU hot loop.
pub struct MaterialTexturePool {
    textures: HashMap<Guid, GpuTexture>,
    fallback_albedo: GpuTexture,
    fallback_normal: GpuTexture,
    fallback_metal_roughness: GpuTexture,
    sampler: wgpu::Sampler,
    anisotropy: u16,
    bgl: wgpu::BindGroupLayout,
    /// Owned here because it caches a render pipeline per format, and
    /// this is the one place textures are uploaded from.
    mipmapper: Mipmapper,
}

impl MaterialTexturePool {
    /// Builds the pool with the three 1×1 fallbacks, a filtering sampler,
    /// and the per-material bind group layout (3 textures + 1 sampler).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let fallback_albedo = GpuTexture::upload(
            device,
            queue,
            &Image::solid_color([255, 255, 255, 255], ImageFormat::Rgba8UnormSrgb),
        );
        let fallback_normal = GpuTexture::upload(
            device,
            queue,
            &Image::solid_color([128, 128, 255, 255], ImageFormat::Rgba8Unorm),
        );
        let fallback_metal_roughness = GpuTexture::upload(
            device,
            queue,
            &Image::solid_color([255, 255, 255, 255], ImageFormat::Rgba8Unorm),
        );

        let sampler = create_sampler(device, NO_ANISOTROPY);

        let bgl = Self::bind_group_layout(device);

        Self {
            textures: HashMap::new(),
            fallback_albedo,
            fallback_normal,
            fallback_metal_roughness,
            mipmapper: Mipmapper::new(device),
            sampler,
            anisotropy: NO_ANISOTROPY,
            bgl,
        }
    }

    /// Per-material bind group layout: albedo(0), normal(1),
    /// metal_roughness(2) textures + sampler(3).
    ///
    /// Visible to compute as well as fragment (#824). The compute
    /// shading pass samples the same three maps through
    /// `textureSampleGrad`, which takes its gradients explicitly and is
    /// therefore legal outside a fragment stage — the derivatives the
    /// surface reconstruction computes analytically are what makes that
    /// true here.
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
            ],
        })
    }

    /// Replaces the sampler with one taking `samples` along the long
    /// axis of a footprint, and reports whether anything changed.
    ///
    /// ⚠️ Every per-material bind group is rebuilt from this sampler
    /// each frame, so there is nothing to invalidate — which is what
    /// makes this a live setting rather than a restart.
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
    ///
    /// What a re-import is, from the pool's side. The bytes on disk did
    /// not change — the answer about them did, and the answer lives in
    /// the texture's descriptor: a chain is levels allocated at creation
    /// and there is no way to add one to a texture that already exists.
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

    /// Resolves a texture view for `guid`, falling back to the slot's
    /// identity texture when the GUID is `None` or unregistered.
    fn view_or_fallback(&self, guid: Option<Guid>, slot: TextureSlot) -> &wgpu::TextureView {
        let fallback = match slot {
            TextureSlot::Albedo => &self.fallback_albedo,
            TextureSlot::Normal => &self.fallback_normal,
            TextureSlot::MetalRoughness => &self.fallback_metal_roughness,
        };
        guid.and_then(|g| self.textures.get(&g))
            .map(|t| &t.view)
            .unwrap_or(&fallback.view)
    }

    /// Builds the per-material bind group. Any `None`/unregistered channel
    /// binds its branch-free fallback, so the shader samples all three
    /// unconditionally.
    pub fn material_bind_group(
        &self,
        device: &wgpu::Device,
        albedo: Option<Guid>,
        normal: Option<Guid>,
        metal_roughness: Option<Guid>,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material_texture_bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        self.view_or_fallback(albedo, TextureSlot::Albedo),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        self.view_or_fallback(normal, TextureSlot::Normal),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        self.view_or_fallback(metal_roughness, TextureSlot::MetalRoughness),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
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
///
/// ⚠️ `anisotropy_clamp` above 1 requires every filter mode to be
/// linear — wgpu rejects the sampler otherwise. All three already are,
/// and this function is where that stays true.
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
