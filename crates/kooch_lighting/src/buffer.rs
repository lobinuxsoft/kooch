//! GPU residency for Inti: the per-frame constants UBO and the light
//! storage buffer, plus the bind group both shading paths bind.

use bytemuck::cast_slice;
use kooch_core::resource::Resources;
use wgpu::util::DeviceExt;

use crate::cluster::{ClusterCamera, ClusterSettings, GpuClusters};
use crate::extract::over_linear_budget;
use crate::frame::{AmbientLight, Exposure, FrameShadows, IntiFrame};
use crate::gpu_light::GpuLight;
use crate::light_frame::LightFrame;

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
    /// 1×1×6 cube array bound when no point light casts, for the same
    /// reason `dummy_shadow` exists: a binding cannot be left empty.
    dummy_cubes: wgpu::TextureView,
    /// The virtual shadow map's page table and atlas (#866), or the
    /// dummies while no sun is paging.
    ///
    /// 🔴 Held rather than derived, exactly like `shadow_atlas`: the
    /// bind group is rebuilt from scratch whenever the light buffer
    /// grows, and rebuilding against the dummies would drop the atlas
    /// the moment a scene crossed a capacity boundary.
    page_uniform: Option<wgpu::Buffer>,
    /// Where this view's slice of the raster uniform starts and how far
    /// it runs. The buffer holds one slice per camera, so the offset is
    /// part of the binding's identity.
    page_uniform_span: (u64, u64),
    page_keys: Option<wgpu::Buffer>,
    page_slots: Option<wgpu::Buffer>,
    page_atlas: Option<wgpu::TextureView>,
    /// Bound when nothing is paging. The uniform is zeroed, and its
    /// `sun.w` is the flag the shader reads — so "no pages" reads as
    /// "no sun casting through pages" rather than as a lookup into an
    /// empty table.
    dummy_page_buffer: wgpu::Buffer,
    dummy_page_atlas: wgpu::TextureView,
    /// The cube array currently bound, or `None` while the dummy is.
    shadow_cubes: Option<wgpu::TextureView>,
    /// The froxel grid (#780). Owned here rather than beside here: its
    /// two buffers are bindings in this group, its input is this
    /// struct's light buffer, and the two grow independently — one
    /// owner is what keeps the bind group naming what is actually bound.
    clusters: GpuClusters,
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
                        // One layer per cascade, and later per spot
                        // light (#777). The layer is an argument to the
                        // sample call, not a second binding.
                        view_dimension: wgpu::TextureViewDimension::D2Array,
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
                // The point lights' cube array (#778) — a fifth binding
                // in this same group, not a seventh group.
                //
                // 🔴 The issue for this work opened by saying it had to
                // decide "what a cube array plus two samplers
                // displaces", because all six bind GROUPS are spent.
                // Bindings are not groups, and the two samplers above
                // are reused untouched: a `wgpu::Sampler` is not bound
                // to a texture, so the comparison sampler that filters
                // the cascades filters a cube face just as well. The
                // whole cost is this one entry.
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        // Six layers per light, the light index chosen by
                        // the shader and the face by the direction.
                        view_dimension: wgpu::TextureViewDimension::CubeArray,
                        multisampled: false,
                    },
                    count: None,
                },
                // The froxel grid (#780): the per-cell records, then the
                // shared index list they point into. Two more bindings
                // in this group, on the same reasoning as the shadow
                // maps above — groups are what ran out, not bindings.
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Virtual shadow maps (#866): the page table's uniform,
                // its two arrays, and the physical atlas. Four more
                // bindings in this group on the same reasoning as the
                // cascades and the grid — groups are what ran out.
                //
                // 🔴 The atlas is `texture_depth_2d` and it is sampled
                // with `textureLoad`, not with the comparison sampler
                // above. A filter cannot cross a page border: the
                // neighbouring texels belong to another clipmap level or
                // another light, and hardware filtering has no way to be
                // told where the page ends.
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        // One layer per camera. See `page_place`.
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
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
        let dummy_cubes = create_dummy_cubes(device);
        let dummy_page_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("inti_dummy_pages"),
            // Large enough for the page uniform when it stands in for
            // it, and harmless as an empty table.
            size: 256,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let dummy_page_atlas = create_dummy_page_atlas(device);
        let clusters = GpuClusters::new(device);
        let bind_group = create_bind_group(
            device,
            &layout,
            &frame_buffer,
            &light_buffer,
            &dummy_shadow,
            &shadow_sampler,
            &shadow_point_sampler,
            &dummy_cubes,
            &clusters,
            PageBinding {
                uniform: &dummy_page_buffer,
                uniform_span: (0, 0),
                keys: &dummy_page_buffer,
                slots: &dummy_page_buffer,
                atlas: &dummy_page_atlas,
            },
        );
        Self {
            frame_buffer,
            light_buffer,
            bind_group,
            layout,
            shadow_sampler,
            shadow_point_sampler,
            dummy_shadow,
            dummy_cubes,
            page_uniform: None,
            page_uniform_span: (0, 0),
            page_keys: None,
            page_slots: None,
            page_atlas: None,
            dummy_page_buffer,
            dummy_page_atlas,
            shadow_atlas: None,
            shadow_cubes: None,
            clusters,
            capacity: INITIAL_CAPACITY,
            light_count: 0,
        }
    }

    /// Points the shadow bindings at the real maps.
    ///
    /// Call once the atlas exists; until then the dummy is bound and
    /// `shadows_enabled` is 0. Idempotent — the atlas is allocated once
    /// and this runs per frame, so re-binding the same view has to cost
    /// nothing.
    pub fn bind_shadow_maps(
        &mut self,
        device: &wgpu::Device,
        atlas: &wgpu::TextureView,
        cubes: &wgpu::TextureView,
    ) {
        let unchanged = self.shadow_atlas.as_ref().is_some_and(|v| v == atlas)
            && self.shadow_cubes.as_ref().is_some_and(|v| v == cubes);
        if unchanged {
            return;
        }
        self.shadow_atlas = Some(atlas.clone());
        self.shadow_cubes = Some(cubes.clone());
        self.rebuild_bind_group(device);
    }

    /// Binds the virtual shadow map's table and atlas (#866).
    ///
    /// Idempotent, and a no-op while nothing changed: the bind group is
    /// expensive to rebuild and this runs every frame.
    pub fn bind_shadow_pages(&mut self, device: &wgpu::Device, pages: PageBinding<'_>) {
        let unchanged = self.page_atlas.as_ref().is_some_and(|v| v == pages.atlas)
            && self
                .page_uniform
                .as_ref()
                .is_some_and(|b| b == pages.uniform)
            // 🔴 The offset is part of the identity. Two cameras share
            // one buffer and differ only here; comparing the handle
            // alone would leave the second one reading the first one's
            // slice, which is the whole bug this binding exists to fix.
            && self.page_uniform_span == pages.uniform_span;
        if unchanged {
            return;
        }
        self.page_uniform_span = pages.uniform_span;
        self.page_uniform = Some(pages.uniform.clone());
        self.page_keys = Some(pages.keys.clone());
        self.page_slots = Some(pages.slots.clone());
        self.page_atlas = Some(pages.atlas.clone());
        self.rebuild_bind_group(device);
    }

    /// Unbinds it, so a frame that stops paging stops sampling.
    ///
    /// 🔴 Not cosmetic: the atlas holds LAST frame's depths, and a
    /// shading pass that kept sampling it would show a shadow frozen in
    /// place — which is silent and gets blamed on everything else first.
    pub fn unbind_shadow_pages(&mut self, device: &wgpu::Device) {
        if self.page_atlas.is_none() {
            return;
        }
        self.page_uniform = None;
        self.page_uniform_span = (0, 0);
        self.page_keys = None;
        self.page_slots = None;
        self.page_atlas = None;
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
            self.shadow_cubes.as_ref().unwrap_or(&self.dummy_cubes),
            &self.clusters,
            PageBinding {
                uniform: self
                    .page_uniform
                    .as_ref()
                    .unwrap_or(&self.dummy_page_buffer),
                uniform_span: if self.page_uniform.is_some() {
                    self.page_uniform_span
                } else {
                    (0, 0)
                },
                keys: self.page_keys.as_ref().unwrap_or(&self.dummy_page_buffer),
                slots: self.page_slots.as_ref().unwrap_or(&self.dummy_page_buffer),
                atlas: self.page_atlas.as_ref().unwrap_or(&self.dummy_page_atlas),
            },
        );
    }

    /// Records the four passes that build the froxel grid (#780).
    ///
    /// Call on the frame's encoder, **before** the pass that shades:
    /// shading reads what these write. Nothing to record when
    /// [`Self::update`] was handed a camera with no matrices.
    pub fn record_clusters(&mut self, encoder: &mut wgpu::CommandEncoder) {
        self.clusters.record(encoder);
    }

    /// The uploaded lights, for a pass that walks them itself rather
    /// than through Inti's bind group.
    pub fn light_buffer(&self) -> &wgpu::Buffer {
        &self.light_buffer
    }

    /// The grid, for the editor's stats overlay.
    pub fn clusters(&self) -> &GpuClusters {
        &self.clusters
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
        camera: ClusterCamera,
        shadows: Option<FrameShadows>,
        light_frame: &mut LightFrame,
    ) {
        // Point lights learn their cube slot here rather than during the
        // walk: the ranking that produced the slots is in `shadows`, and
        // recomputing it would be a second sort that has to agree with
        // the first one forever. See `assign_point_slots`.
        if let Some(shadows) = shadows.as_ref() {
            crate::extract::assign_point_slots(
                light_frame.lights_mut(),
                &shadows.point_entities[..shadows.point_shadow_count as usize],
            );
        }
        let lights = &light_frame.lights().lights;
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
                    "past this count the light loop is the frame if the froxel grid is not \
                     building — check `ClusterSettings::enabled` and KOOCH_CLUSTERING.",
                );
            }
        }
        self.light_count = count;
        self.ensure_capacity(device, self.light_count);
        if !lights.is_empty() {
            queue.write_buffer(&self.light_buffer, 0, cast_slice(lights));
        }

        // The grid, ahead of the encoder for the same reason everything
        // else here is: it grows buffers, and a grown buffer is a
        // replaced one.
        let settings = resources
            .get::<ClusterSettings>()
            .copied()
            .unwrap_or_default();
        let clustered = camera
            .matrices
            .filter(|_| settings.enabled)
            .map(|(view, proj)| {
                let rebuilt = self.clusters.update(
                    device,
                    queue,
                    &settings,
                    view,
                    proj,
                    camera.viewport,
                    &self.light_buffer,
                    self.light_count,
                );
                if rebuilt {
                    self.rebuild_bind_group(device);
                }
                view
            });

        // Resolved here rather than by the editor because this is where
        // the buffer's order is decided, and the slot only means
        // anything against that order (#743).
        let debug_light = resources
            .get::<crate::DebugLight>()
            .and_then(|d| d.0)
            .and_then(|entity| light_frame.lights().slot_of(entity));

        let ambient = resources.get::<AmbientLight>().copied().unwrap_or_default();
        let exposure = resources.get::<Exposure>().copied().unwrap_or_default();
        let mut frame = IntiFrame::new(&ambient, &exposure, camera.position, self.light_count)
            .with_optional_shadows(shadows)
            .with_debug_light(debug_light)
            .with_specular_floor(
                resources
                    .get::<crate::SpecularFloor>()
                    .copied()
                    .unwrap_or_default()
                    .0,
            )
            .with_light_limit(
                resources
                    .get::<crate::LightLimit>()
                    .copied()
                    .unwrap_or_default()
                    .0,
            )
            .with_lights_hot(
                resources
                    .get::<crate::LightsHot>()
                    .copied()
                    .unwrap_or_default()
                    .0,
            )
            .with_directionals(light_frame.lights().directional_count);
        if let Some(view) = clustered {
            frame = frame.with_clusters(self.clusters.grid(), view, self.clusters.index_capacity());
        }
        queue.write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&frame));
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
/// The 1×1 cube array bound when nothing point-shaped casts.
///
/// Six layers, because a cube view of anything else is a validation
/// failure — the same trap `create_dummy_shadow` documents for `D2` vs
/// `D2Array`, one dimension further along. Cleared implicitly to zero,
/// which under reversed-Z is the far plane and therefore "fully lit".
fn create_dummy_cubes(device: &wgpu::Device) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("inti_dummy_cubes"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 6,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("inti_dummy_cubes_view"),
        dimension: Some(wgpu::TextureViewDimension::CubeArray),
        ..Default::default()
    })
}

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
    // 🔴 `D2Array` explicitly, not the default. A one-layer texture's
    // default view is `D2`, and a `D2` view against a `D2Array` layout
    // is a validation failure at bind-group creation — on every scene
    // with no shadow-casting sun, which is the case nobody renders while
    // developing shadows.
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("inti_dummy_shadow_view"),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}

#[allow(clippy::too_many_arguments)]
/// What the shading model needs to read a virtual shadow page.
///
/// A struct rather than four more parameters, because they are only ever
/// present or absent together: a table without an atlas indexes nothing
/// and an atlas without a table cannot be addressed.
#[derive(Clone, Copy)]
pub struct PageBinding<'a> {
    pub uniform: &'a wgpu::Buffer,
    /// Where this camera's slice of `uniform` starts, and how long it
    /// is. `(0, 0)` binds the whole buffer, which is what the dummy
    /// wants.
    ///
    /// 🔴 One buffer with a slice per camera, not one write per frame.
    /// `Queue::write_buffer` is not ordered against the encoder, so two
    /// writes to the same range in one frame hand BOTH passes the second
    /// value — the engine has already shipped that bug once (#853). A
    /// camera writing its own range cannot be overwritten by the other.
    pub uniform_span: (u64, u64),
    pub keys: &'a wgpu::Buffer,
    pub slots: &'a wgpu::Buffer,
    pub atlas: &'a wgpu::TextureView,
}

#[allow(clippy::too_many_arguments)]
fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    frame: &wgpu::Buffer,
    lights: &wgpu::Buffer,
    shadow_atlas: &wgpu::TextureView,
    shadow_sampler: &wgpu::Sampler,
    shadow_point_sampler: &wgpu::Sampler,
    shadow_cubes: &wgpu::TextureView,
    clusters: &GpuClusters,
    pages: PageBinding<'_>,
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
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(shadow_cubes),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: clusters.cells().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: clusters.indices().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: pages.uniform,
                    offset: pages.uniform_span.0,
                    size: std::num::NonZeroU64::new(pages.uniform_span.1),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: pages.keys.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 10,
                resource: pages.slots.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 11,
                resource: wgpu::BindingResource::TextureView(pages.atlas),
            },
        ],
    })
}

/// A 1x1 depth texture bound when no page is being sampled, for the same
/// reason `create_dummy_shadow` exists.
fn create_dummy_page_atlas(device: &wgpu::Device) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("inti_dummy_page_atlas"),
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
    // Declared as an array of one, because the layout says array and a
    // 2D view against a `D2Array` entry is a validation error rather
    // than a silently different picture.
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}
