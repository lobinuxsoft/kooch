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

use ome_core::Guid;

use crate::texture::{GpuTexture, Image, ImageFormat};

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
    bgl: wgpu::BindGroupLayout,
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

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material_texture_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            ..Default::default()
        });

        let bgl = Self::bind_group_layout(device);

        Self {
            textures: HashMap::new(),
            fallback_albedo,
            fallback_normal,
            fallback_metal_roughness,
            sampler,
            bgl,
        }
    }

    /// Per-material bind group layout: albedo(0), normal(1),
    /// metal_roughness(2) textures + sampler(3), all fragment-visible.
    pub fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        let texture_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
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
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        })
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
        let texture = GpuTexture::upload(device, queue, image);
        self.textures.insert(guid, texture);
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
mod tests {
    use super::*;

    /// Acquires a headless device with default features/limits. Returns
    /// `None` when no adapter is available (CI without a GPU) so the test
    /// skips rather than fails, matching the crate's other GPU tests.
    fn try_acquire_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: None,
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("material_texture_pool_test_device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::default(),
        }))
        .ok()
    }

    fn checker(format: ImageFormat) -> Image {
        Image::solid_color([200, 100, 50, 255], format)
    }

    #[test]
    fn register_and_lookup_roundtrip() {
        let Some((device, queue)) = try_acquire_device() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let mut pool = MaterialTexturePool::new(&device, &queue);
        assert!(pool.is_empty());

        let g = Guid::new_v4();
        assert!(!pool.contains(g));
        pool.register(&device, &queue, g, &checker(ImageFormat::Rgba8UnormSrgb));
        assert!(pool.contains(g));
        assert_eq!(pool.len(), 1);

        // Re-register same GUID replaces, does not grow.
        pool.register(&device, &queue, g, &checker(ImageFormat::Rgba8UnormSrgb));
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn material_bind_group_builds_with_and_without_textures() {
        let Some((device, queue)) = try_acquire_device() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let mut pool = MaterialTexturePool::new(&device, &queue);
        let albedo = Guid::new_v4();
        pool.register(
            &device,
            &queue,
            albedo,
            &checker(ImageFormat::Rgba8UnormSrgb),
        );

        // All-fallback (no maps) must build against the same layout.
        let _bg_none = pool.material_bind_group(&device, None, None, None);
        // Mixed: real albedo, fallback normal + metal_roughness.
        let _bg_mixed = pool.material_bind_group(&device, Some(albedo), None, None);
        // Unregistered GUID silently falls back.
        let _bg_missing = pool.material_bind_group(&device, Some(Guid::new_v4()), None, None);
    }
}
