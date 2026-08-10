//! GPU residency for Inti: the per-frame constants UBO and the light
//! storage buffer, plus the bind group both shading paths bind.

use bytemuck::cast_slice;
use glam::Vec3;
use kooch_core::resource::Resources;
use wgpu::util::DeviceExt;

use crate::extract::{extract_lights, over_linear_budget};
use crate::frame::{AmbientLight, Exposure, FrameShadows, IntiFrame};
use crate::gpu_light::GpuLight;

/// Lights a fresh buffer holds before it has to grow. Sixteen covers an
/// authored room; growth is geometric from there.
const INITIAL_CAPACITY: u32 = 16;

/// Owns the two bindings `inti_pbr.wgsl` declares and keeps them sized
/// to the scene.
///
/// # One buffer, several views
///
/// The light set does not depend on where the camera is, so it is
/// shared across every view in a frame. `camera_position` does depend
/// on it and rides in the frame UBO anyway — which is safe only
/// because each view records **and submits** its own encoder:
/// `write(A) → submit(A) → write(B) → submit(B)` is ordered on the
/// queue, so B's camera cannot reach A's pass. The existing
/// `MaterialTwoPass::camera_buffer` is shared on exactly this
/// reasoning. If a future path ever records two views into one encoder,
/// this buffer needs a dynamic offset per view, the same way the
/// per-material screen UBO already does.
pub struct GpuLights {
    frame_buffer: wgpu::Buffer,
    light_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    layout: wgpu::BindGroupLayout,
    /// Comparison sampler for the shadow atlas.
    shadow_sampler: wgpu::Sampler,
    /// Plain sampler on the same texture, for the blocker search.
    shadow_point_sampler: wgpu::Sampler,
    /// 1×1 depth texture bound when there is no shadow atlas.
    ///
    /// A binding cannot be left empty, and a second pipeline for
    /// "no shadows" would be a whole code path exercised only in the
    /// case nobody looks at. Cleared to the far plane, so if the
    /// `shadows_enabled` flag ever failed to stop the sampling, the
    /// answer would be "fully lit" rather than "everything is dark".
    dummy_shadow: wgpu::TextureView,
    /// The atlas currently bound, or `None` while the dummy is.
    ///
    /// Kept rather than derived because the bind group is rebuilt from
    /// scratch whenever the light buffer grows, and rebuilding it
    /// against the dummy would drop the atlas the moment a scene
    /// crossed a capacity boundary — shadows disappearing when the
    /// seventeenth light is placed, with nothing in the log.
    shadow_atlas: Option<wgpu::TextureView>,
    capacity: u32,
    light_count: u32,
}

impl GpuLights {
    /// The layout both shading pipelines declare. An associated
    /// function because a pipeline layout has to be built before any
    /// `GpuLights` exists — the same shape `MaterialPool` uses.
    pub fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("inti_lights_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    // Visible to both stages because the R64 path shades
                    // in a fragment shader and the R32 fallback in a
                    // compute one, off the same layout.
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(
                            std::mem::size_of::<IntiFrame>() as u64,
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // The shadow atlas lives in Inti's group rather than one
                // of its own: the bind-group budget is spent, and a
                // shadow map without its light is not a thing any shader
                // wants. See `TARGET_MAX_BIND_GROUPS`.
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // A comparison sampler, so the depth test happens on the
                // texture unit and each tap comes back bilinearly
                // filtered. Sampling depth and comparing by hand would
                // be sixteen manual comparisons per fragment and no
                // filtering.
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                // A second, non-comparison sampler on the same texture,
                // for PCSS's blocker search — which needs the depth
                // itself, where a comparison sampler only ever answers
                // "nearer or not".
                //
                // 🔴 This was written off as impossible: "there is no
                // bind group left". The exhausted budget is on *groups*,
                // and this is a fourth binding inside a group that
                // already exists. Bevy does exactly this
                // (`directional_shadow_textures_linear_sampler`), which
                // is what made the claim worth re-checking.
                //
                // Non-filtering because the sample type is `Depth`, and
                // wgpu will not pair that with a filtering sampler. The
                // search averages eight taps, so bilinear on each of
                // them buys very little.
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
        })
    }

    pub fn new(device: &wgpu::Device) -> Self {
        let layout = Self::bind_group_layout(device);
        let frame_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("inti_frame_ubo"),
            contents: bytemuck::bytes_of(&IntiFrame::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let light_buffer = create_light_buffer(device, INITIAL_CAPACITY);
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("inti_shadow_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            // Reversed-Z: an occluder is nearer the light and therefore
            // GREATER. The comparison has to match, or every shadow
            // inverts and the lit side goes dark.
            compare: Some(wgpu::CompareFunction::Greater),
            // Clamped: a tap that falls off a cascade should read as the
            // edge rather than wrapping into the neighbouring quadrant,
            // which would be a shadow from the wrong distance.
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let shadow_point_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("inti_shadow_point_sampler"),
            // Nearest on both, because the layout declares this
            // non-filtering and wgpu checks that the sampler agrees.
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let dummy_shadow = create_dummy_shadow(device);
        let bind_group = create_bind_group(
            device,
            &layout,
            &frame_buffer,
            &light_buffer,
            &dummy_shadow,
            &shadow_sampler,
            &shadow_point_sampler,
        );
        Self {
            frame_buffer,
            light_buffer,
            bind_group,
            layout,
            shadow_sampler,
            shadow_point_sampler,
            dummy_shadow,
            shadow_atlas: None,
            capacity: INITIAL_CAPACITY,
            light_count: 0,
        }
    }

    /// Points the shadow binding at a real atlas.
    ///
    /// Call once the atlas exists; until then the dummy is bound and
    /// `shadows_enabled` is 0. Idempotent — the atlas is allocated once
    /// and this runs per frame, so re-binding the same view has to cost
    /// nothing.
    pub fn bind_shadow_atlas(&mut self, device: &wgpu::Device, atlas: &wgpu::TextureView) {
        if self.shadow_atlas.as_ref().is_some_and(|v| v == atlas) {
            return;
        }
        self.shadow_atlas = Some(atlas.clone());
        self.rebuild_bind_group(device);
    }

    /// Rebuilds the bind group against whatever shadow view is current.
    ///
    /// One place, so growth and atlas binding cannot disagree about
    /// which texture is bound.
    fn rebuild_bind_group(&mut self, device: &wgpu::Device) {
        self.bind_group = create_bind_group(
            device,
            &self.layout,
            &self.frame_buffer,
            &self.light_buffer,
            self.shadow_atlas.as_ref().unwrap_or(&self.dummy_shadow),
            &self.shadow_sampler,
            &self.shadow_point_sampler,
        );
    }

    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    /// Lights uploaded by the last [`Self::update`]. Reported by the
    /// editor stats overlay so "I placed a light and nothing changed"
    /// has an answer that is not a guess.
    pub fn light_count(&self) -> u32 {
        self.light_count
    }

    /// Walks the world, uploads the lights, and writes the per-frame
    /// constants.
    ///
    /// Call **before** creating the frame's encoder: growing the
    /// storage buffer replaces it, and a replaced buffer must not be
    /// one a recorded pass already references.
    ///
    /// `shadows` is `None` when nothing casts — no sun, or the atlas
    /// has not been built. The dummy atlas stays bound and the shader
    /// skips the sampling entirely.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &Resources,
        camera_position: Vec3,
        shadows: Option<FrameShadows>,
    ) {
        let extracted = extract_lights(resources);
        let lights = &extracted.lights;
        let count = lights.len() as u32;
        // Logged on change, never per frame. "I placed a light and
        // nothing happened" and "the light never reached the GPU" look
        // identical from the chair, and this is the one line that
        // separates them — at the cost of nothing in a steady scene.
        if count != self.light_count {
            tracing::debug!(
                target: "kooch_lighting::buffer",
                from = self.light_count,
                to = count,
                "light count changed",
            );
            // On the transition only. The advice is worth one line when
            // a scene crosses the threshold and worth nothing at sixty
            // lines a second afterwards.
            if let Some(budget) = over_linear_budget(lights.len())
                && over_linear_budget(self.light_count as usize).is_none()
            {
                tracing::warn!(
                    target: "kooch_lighting::buffer",
                    lights = lights.len(),
                    budget,
                    "Inti shades every light for every pixel; past this count that loop is \
                     the frame. Clustering is the fix and is not implemented yet.",
                );
            }
        }
        self.light_count = count;
        self.ensure_capacity(device, self.light_count);
        if !lights.is_empty() {
            queue.write_buffer(&self.light_buffer, 0, cast_slice(lights));
        }

        // Resolved here rather than by the editor because this is where
        // the buffer's order is decided, and the slot only means
        // anything against that order (#743).
        let debug_light = resources
            .get::<crate::DebugLight>()
            .and_then(|d| d.0)
            .and_then(|entity| extracted.slot_of(entity));

        let ambient = resources.get::<AmbientLight>().copied().unwrap_or_default();
        let exposure = resources.get::<Exposure>().copied().unwrap_or_default();
        queue.write_buffer(
            &self.frame_buffer,
            0,
            bytemuck::bytes_of(
                &IntiFrame::new(&ambient, &exposure, camera_position, self.light_count)
                    .with_optional_shadows(shadows)
                    .with_debug_light(debug_light),
            ),
        );
    }

    /// Grows geometrically to fit `needed`. Never shrinks: a scene that
    /// oscillates around a capacity boundary would otherwise reallocate
    /// every frame.
    fn ensure_capacity(&mut self, device: &wgpu::Device, needed: u32) {
        if needed <= self.capacity {
            return;
        }
        let capacity = needed.next_power_of_two().max(INITIAL_CAPACITY);
        self.light_buffer = create_light_buffer(device, capacity);
        self.capacity = capacity;
        self.rebuild_bind_group(device);
        tracing::debug!(
            target: "kooch_lighting::buffer",
            capacity,
            "grew the Inti light buffer",
        );
    }
}

/// Capacity is floored at one element even for an unlit scene: wgpu
/// rejects a zero-sized storage binding, and a second pipeline for
/// "no lights" would be a whole code path that only runs in the case
/// nobody looks at.
fn create_light_buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("inti_lights_storage"),
        size: (capacity.max(1) as u64) * std::mem::size_of::<GpuLight>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// A 1×1 depth texture for the frames with no atlas.
fn create_dummy_shadow(device: &wgpu::Device) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("inti_dummy_shadow"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    frame: &wgpu::Buffer,
    lights: &wgpu::Buffer,
    shadow_atlas: &wgpu::TextureView,
    shadow_sampler: &wgpu::Sampler,
    shadow_point_sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("inti_lights_bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: frame.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: lights.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(shadow_atlas),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(shadow_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(shadow_point_sampler),
            },
        ],
    })
}
