//! Rasterising depth into the pages marking asked for (#866).
//!
//! Four passes, and the shape of them is the feature:
//!
//! 1. **Cull**, once per clipmap level, with the engine's existing
//!    meshlet cull. A level is a texel density and a density is a LOD,
//!    so the survivors differ per level and nothing about that is new —
//!    it is what the four cascades already do, seventeen times.
//! 2. **Compact** the hash table into a dense list of resident pages,
//!    bucketed by level.
//! 3. **Expand** into `(page, meshlet)` pairs: which meshlet actually
//!    touches which page. This is where a virtual shadow map earns its
//!    name — a meshlet is rasterised into the pages it covers rather
//!    than into a map sized for the worst case.
//! 4. **Draw**, once, indirect, over every pair. The atlas is one depth
//!    attachment and every page is a sub-rect of it, so 1681 pages are
//!    ONE render pass instead of 1681.
//!
//! # 🔴 The sun only, and the seam is not arbitrary
//!
//! A cull is per view. The sun's clipmap is **17** views. A hundred
//! local lights with six faces and an eight-level chain each are
//! **4848**, and the LOD selector is a two-pass reduction over the
//! meshlet DAG that cannot simply be inlined per page. Local pages are
//! marked and allocated today and counted as skipped here; rasterising
//! them needs the cull itself moved onto the GPU as one multi-view
//! dispatch, which is the next machine and not a bigger version of this
//! one.

use glam::{Mat4, Vec3};

use crate::meshlet::{
    CullParams, GpuGlobalMeshPool, MeshletCull, MeshletCullPipelines, MeshletScene, SceneCullParams,
};

use super::pool::{PagePool, PoolConfig};
use super::{ClipmapConfig, PageConfig};

const TABLE: &str = include_str!("../../../shaders/page_table.wgsl");
const COMPACT: &str = include_str!("../../../shaders/page_compact.wgsl");
const EXPAND: &str = include_str!("../../../shaders/page_expand.wgsl");
const DEPTH: &str = include_str!("../../../shaders/page_depth.wgsl");

/// What the pages are rasterised at. The same format the cascades use,
/// so the sampling path in #477 reaches for the same comparison sampler
/// rather than for a second one.
pub const PAGE_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// How far a caster may be from a page along the sun's own axis, in
/// metres, before it stops writing into it.
///
/// 🔴 A shadow's depth range is not its footprint. A page is a few
/// metres across at the finest level and the thing casting into it can
/// be a mountain a kilometre up, so this is deliberately generous and
/// deliberately separate from the clipmap's extent.
pub const SUN_SPAN: f32 = 2000.0;

/// Pages one level may list.
///
/// Sized to the whole pool: a frame is free to spend every page on one
/// level, and a bucket that silently clamped would drop shadows without
/// saying so.
fn bucket(pool: PoolConfig) -> u32 {
    pool.pages
}

/// What the raster did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RasterCounts {
    /// Sun pages listed, per level summed.
    pub pages: u32,
    /// Sun pages that did not fit their bucket. Non-zero means shadows
    /// are missing.
    pub dropped: u32,
    /// 🔴 Pages belonging to local lights, which are marked and
    /// allocated but not yet rasterised. Reported rather than ignored:
    /// a pool that looks full for a reason nobody stated is how a
    /// budget gets mis-read.
    pub local: u32,
    /// `(page, meshlet)` pairs the draw covered.
    pub pairs: u32,
    /// Pairs past the list's capacity.
    pub overflow: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RasterUniform {
    space: [u32; 4],
    pool: [u32; 4],
    chain: [u32; 4],
    world: [f32; 4],
    eye: [f32; 4],
    sun: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ExpandLevel {
    level: u32,
    _pad: [u32; 3],
}

/// The atlas, the buffers between the four passes, and the pipelines.
pub struct PageRasterizer {
    atlas: wgpu::Texture,
    atlas_view: wgpu::TextureView,
    uniform: wgpu::Buffer,
    page_list: wgpu::Buffer,
    counts: wgpu::Buffer,
    expand_args: wgpu::Buffer,
    draw_args: wgpu::Buffer,
    pairs: wgpu::Buffer,
    visible_counts: wgpu::Buffer,
    levels: wgpu::Buffer,
    level_stride: u64,

    compact_bgl: wgpu::BindGroupLayout,
    compact: wgpu::ComputePipeline,
    expand_args_pass: wgpu::ComputePipeline,
    draw_args_pass: wgpu::ComputePipeline,

    expand_bgl: wgpu::BindGroupLayout,
    storage_bgl: wgpu::BindGroupLayout,
    expand: wgpu::ComputePipeline,

    depth_bgl: wgpu::BindGroupLayout,
    depth: wgpu::RenderPipeline,

    culls: Vec<MeshletCull>,
    readback: RasterReadback,
    config: PageConfig,
    clipmap: ClipmapConfig,
    pool: PoolConfig,
}

impl PageRasterizer {
    pub fn new(
        device: &wgpu::Device,
        meshlet_bgl: &wgpu::BindGroupLayout,
        config: PageConfig,
        clipmap: ClipmapConfig,
        pool: PoolConfig,
        max_triangles_per_meshlet: u32,
    ) -> Self {
        let atlas = atlas_texture(device, config, pool);
        let atlas_view = atlas.create_view(&Default::default());
        let levels = clipmap.levels;

        let module = |label: &str, body: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(format!("{TABLE}\n{body}").into()),
            })
        };
        let compact_module = module("page_compact", COMPACT);
        let expand_module = module("page_expand", EXPAND);
        let depth_module = module("page_depth", DEPTH);

        let compact_bgl = compact_layout(device);
        let compact_layout_pipeline =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("page_compact_layout"),
                bind_group_layouts: &[Some(&compact_bgl)],
                immediate_size: 0,
            });
        let compute = |entry: &str, module: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let compact = compute("cs_compact", &compact_module, &compact_layout_pipeline);
        let expand_args_pass = compute("cs_expand_args", &compact_module, &compact_layout_pipeline);
        let draw_args_pass = compute("cs_draw_args", &compact_module, &compact_layout_pipeline);

        let expand_bgl = expand_layout(device);
        let storage_bgl = storage_layout(
            device,
            wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::VERTEX,
        );
        let expand_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("page_expand_layout"),
                // 🔴 Its OWN one-buffer layout for the descriptors,
                // not the meshlet pool's five. `max_storage_buffers_
                // _per_shader_stage` is 8 by default and the pool alone
                // would spend five of them on four buffers this pass
                // never reads.
                bind_group_layouts: &[
                    Some(&expand_bgl),
                    Some(&storage_bgl),
                    Some(&storage_bgl),
                    Some(&storage_bgl),
                ],
                immediate_size: 0,
            });
        let expand = compute("cs_expand", &expand_module, &expand_pipeline_layout);

        let depth_bgl = depth_layout(device);
        let depth_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("page_depth_layout"),
                bind_group_layouts: &[Some(&depth_bgl), Some(meshlet_bgl), Some(&storage_bgl)],
                immediate_size: 0,
            });
        let depth = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("page_depth"),
            layout: Some(&depth_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &depth_module,
                entry_point: Some("vs_page"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            // 🔴 A fragment stage where `shadow_depth` has none, and it
            // is not an oversight: it is the per-page scissor the
            // hardware cannot give per instance. See `page_depth.wgsl`.
            fragment: Some(wgpu::FragmentState {
                module: &depth_module,
                entry_point: Some("fs_page"),
                targets: &[],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: PAGE_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                // Reversed-Z, like every other depth test in the engine.
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        let level_stride = align.max(std::mem::size_of::<ExpandLevel>() as u64);
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;

        Self {
            atlas,
            atlas_view,
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_uniform"),
                size: std::mem::size_of::<RasterUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            page_list: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_list"),
                size: bucket(pool) as u64 * levels as u64 * 8,
                usage: storage | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            counts: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_counts"),
                size: count_slots(levels) as u64 * 4,
                usage: storage | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            expand_args: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_expand_args"),
                size: levels as u64 * 12,
                usage: storage | wgpu::BufferUsages::INDIRECT,
                mapped_at_creation: false,
            }),
            draw_args: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_draw_args"),
                size: 16,
                usage: storage | wgpu::BufferUsages::INDIRECT,
                mapped_at_creation: false,
            }),
            pairs: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_pairs"),
                size: PAIR_CAPACITY as u64 * 8,
                usage: storage,
                mapped_at_creation: false,
            }),
            visible_counts: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_visible_counts"),
                size: levels as u64 * 4,
                usage: storage,
                mapped_at_creation: false,
            }),
            levels: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_levels"),
                size: level_stride * levels as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            level_stride,
            compact_bgl,
            compact,
            expand_args_pass,
            draw_args_pass,
            expand_bgl,
            storage_bgl,
            expand,
            depth_bgl,
            depth,
            culls: (0..levels)
                .map(|_| MeshletCull::new(device, 1, max_triangles_per_meshlet))
                .collect(),
            readback: RasterReadback::new(device, count_slots(levels)),
            config,
            clipmap,
            pool,
        }
    }

    /// The depth atlas every resident page is rasterised into.
    pub fn atlas(&self) -> &wgpu::TextureView {
        &self.atlas_view
    }

    pub fn atlas_texture(&self) -> &wgpu::Texture {
        &self.atlas
    }

    /// What the atlas costs, which is the whole point of a pool.
    pub fn atlas_bytes(&self) -> u64 {
        self.pool.atlas_bytes(self.config)
    }

    /// The counters, for whoever reads them back.
    pub fn counts_buffer(&self) -> &wgpu::Buffer {
        &self.counts
    }

    /// Slots in [`Self::counts_buffer`].
    pub fn count_slots(&self) -> u32 {
        count_slots(self.clipmap.levels)
    }

    /// Reads the counters out of a mapped copy of [`Self::counts_buffer`].
    pub fn decode(&self, words: &[u32]) -> RasterCounts {
        let levels = self.clipmap.levels as usize;
        RasterCounts {
            pages: words[..levels]
                .iter()
                .map(|&n| n.min(bucket(self.pool)))
                .sum(),
            dropped: words[levels],
            local: words[levels + 1],
            pairs: words[levels + 2].min(PAIR_CAPACITY),
            overflow: words[levels + 3],
        }
    }
}

/// Pairs one frame may draw.
///
/// 8 MiB, and a ceiling rather than a guess: `RasterCounts::overflow`
/// says when it was reached, which is the difference between a bound
/// and a silent truncation.
pub const PAIR_CAPACITY: u32 = 1 << 20;

fn count_slots(levels: u32) -> u32 {
    // Per level, then: bucket overflow, local pages skipped, pairs, pair
    // overflow.
    levels + 4
}

fn atlas_texture(device: &wgpu::Device, config: PageConfig, pool: PoolConfig) -> wgpu::Texture {
    let side = pool.per_row() * config.page;
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow_page_atlas"),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: PAGE_DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

impl PageRasterizer {
    /// The uniform every raster pass reads. Written once a frame,
    /// before any of them.
    fn write_uniform(
        &self,
        queue: &wgpu::Queue,
        eye: Vec3,
        sun: Vec3,
        lights: u32,
        meshlets_per_mesh: u32,
    ) {
        let d = sun.normalize_or(Vec3::NEG_Y);
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&RasterUniform {
                space: [
                    super::mark::stride(self.config, self.clipmap),
                    self.config.face_pages(),
                    self.config.side(0),
                    // The sun's slot is one past the last light, the way
                    // marking assigns it.
                    lights.max(1),
                ],
                pool: [
                    self.pool.entries(),
                    self.pool.pages,
                    self.pool.per_row(),
                    self.config.page,
                ],
                chain: [
                    self.clipmap.levels,
                    PAIR_CAPACITY,
                    bucket(self.pool),
                    meshlets_per_mesh.max(1),
                ],
                world: [
                    self.clipmap.base,
                    SUN_SPAN,
                    (self.pool.per_row() * self.config.page) as f32,
                    0.0,
                ],
                eye: [eye.x, eye.y, eye.z, 0.0],
                sun: [d.x, d.y, d.z, 1.0],
            }),
        );
    }

    /// The table becomes a dense list, bucketed by level, and the
    /// expansion's dispatch sizes are computed from it.
    ///
    /// Public because it is the half that can be tested without a
    /// scene: hand it a table and the buckets say whether a page
    /// decodes back to the level it was encoded from.
    pub fn record_compaction(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        page_pool: &PagePool,
        eye: Vec3,
        sun: Vec3,
        lights: u32,
        meshlets_per_mesh: u32,
    ) {
        self.write_uniform(queue, eye, sun, lights, meshlets_per_mesh);
        encoder.clear_buffer(&self.counts, 0, None);
        encoder.clear_buffer(&self.expand_args, 0, None);
        encoder.clear_buffer(&self.draw_args, 0, None);
        let bind_group = self.compact_bind_group(device, page_pool);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("shadow pages: compact"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.compact);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(self.pool.entries().div_ceil(64), 1, 1);
        pass.set_pipeline(&self.expand_args_pass);
        pass.dispatch_workgroups(self.clipmap.levels.div_ceil(64), 1, 1);
    }

    fn compact_bind_group(&self, device: &wgpu::Device, page_pool: &PagePool) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("page_compact_bg"),
            layout: &self.compact_bgl,
            entries: &[
                entry(0, &self.uniform),
                entry(1, page_pool.keys()),
                entry(2, page_pool.slots()),
                entry(3, &self.page_list),
                entry(4, &self.counts),
                entry(5, &self.expand_args),
                entry(6, &self.visible_counts),
                entry(7, &self.draw_args),
            ],
        })
    }

    /// The compacted pages, for whoever reads them back.
    /// COPY_SRC so a test can read it.
    pub fn page_list_buffer(&self) -> &wgpu::Buffer {
        &self.page_list
    }

    /// Grows every level's cull to the scene.
    pub fn ensure_capacity(&mut self, device: &wgpu::Device, meshlets: u32, groups: u32) {
        for cull in &mut self.culls {
            cull.ensure_capacity(device, meshlets.max(1));
            cull.ensure_group_capacity(device, groups.max(1));
        }
    }

    /// The clipmap level's orthographic clip-from-world.
    ///
    /// 🔴 Built to agree with `sun_basis` and `sun_page_rect` in the
    /// shader, term for term. This matrix decides which meshlets survive
    /// and those two decide where they land, so a disagreement is
    /// geometry culled for one page and drawn into another.
    fn level_clip(&self, level: u32, eye: Vec3, sun: Vec3) -> Mat4 {
        let f = sun.normalize_or(Vec3::NEG_Y);
        let up = if f.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
        let s = f.cross(up).normalize();
        let u = s.cross(f);
        let rotation = Mat4::from_cols(
            glam::Vec4::new(s.x, u.x, f.x, 0.0),
            glam::Vec4::new(s.y, u.y, f.y, 0.0),
            glam::Vec4::new(s.z, u.z, f.z, 0.0),
            glam::Vec4::W,
        );
        let half = self.clipmap.extent(level) * 0.5;
        // Reversed-Z orthographic: 1 at the near plane, 0 at the far,
        // matching `page_depth.wgsl`'s `1 - (z + span) / (2 * span)`.
        let projection = Mat4::from_cols(
            glam::Vec4::new(1.0 / half, 0.0, 0.0, 0.0),
            glam::Vec4::new(0.0, 1.0 / half, 0.0, 0.0),
            glam::Vec4::new(0.0, 0.0, -1.0 / (2.0 * SUN_SPAN), 0.0),
            glam::Vec4::new(0.0, 0.0, 0.5, 1.0),
        );
        projection * rotation * Mat4::from_translation(-eye)
    }

    /// Culls, compacts, expands and draws. Call **after** the marking
    /// pass: it reads the table marking filled.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        cull_pipelines: &MeshletCullPipelines,
        mesh_pool: &GpuGlobalMeshPool,
        scene: &MeshletScene,
        meshlet_bg: &wgpu::BindGroup,
        instances: &wgpu::Buffer,
        page_pool: &PagePool,
        scene_params: &SceneCullParams,
        eye: Vec3,
        sun: Vec3,
        lights: u32,
        lod_target: f32,
    ) {
        let levels = self.clipmap.levels;
        self.write_uniform(queue, eye, sun, lights, scene_params.meshlets_per_mesh);

        // 1. One cull per level. A level is a texel density and a
        //    density is a LOD.
        for level in 0..levels {
            queue.write_buffer(
                &self.levels,
                level as u64 * self.level_stride,
                bytemuck::bytes_of(&ExpandLevel {
                    level,
                    _pad: [0; 3],
                }),
            );
            let clip = self.level_clip(level, eye, sun);
            let params = CullParams::new(
                clip,
                eye - sun.normalize_or(Vec3::NEG_Y) * SUN_SPAN,
                scene_params.meshlets_per_mesh,
            )
            .with_orthographic_lod(
                self.clipmap.extent(level),
                self.config.virtual_size as f32,
                lod_target.max(0.01),
            );
            self.culls[level as usize].dispatch_scene_pool_atomic(
                cull_pipelines,
                device,
                queue,
                encoder,
                mesh_pool,
                scene,
                &params,
                scene_params,
            );
            // The expansion's dispatch size is pages times survivors,
            // and the survivor count only exists on the GPU.
            encoder.copy_buffer_to_buffer(
                self.culls[level as usize].visible_count_buffer(),
                0,
                &self.visible_counts,
                level as u64 * 4,
                4,
            );
        }

        encoder.clear_buffer(&self.counts, 0, None);
        encoder.clear_buffer(&self.expand_args, 0, None);
        encoder.clear_buffer(&self.draw_args, 0, None);

        let compact_bg = self.compact_bind_group(device, page_pool);
        let instances_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("page_raster_instances_bg"),
            layout: &self.storage_bgl,
            entries: &[entry(0, instances)],
        });
        let descriptors_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("page_raster_descriptors_bg"),
            layout: &self.storage_bgl,
            entries: &[entry(0, &mesh_pool.meshlets)],
        });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("shadow pages: compact and expand"),
                timestamp_writes: None,
            });
            // 2. The hash table becomes a dense list, bucketed by level.
            pass.set_pipeline(&self.compact);
            pass.set_bind_group(0, &compact_bg, &[]);
            pass.dispatch_workgroups(self.pool.entries().div_ceil(64), 1, 1);
            pass.set_pipeline(&self.expand_args_pass);
            pass.dispatch_workgroups(levels.div_ceil(64), 1, 1);

            // 3. Pairs. One indirect dispatch per level, sized by the
            //    pass above rather than by a CPU guess.
            pass.set_pipeline(&self.expand);
            pass.set_bind_group(1, &descriptors_bg, &[]);
            pass.set_bind_group(3, &instances_bg, &[]);
            for level in 0..levels {
                let visible_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("page_expand_visible_bg"),
                    layout: &self.storage_bgl,
                    entries: &[entry(
                        0,
                        self.culls[level as usize].visible_meshlets_buffer(),
                    )],
                });
                let expand_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("page_expand_bg"),
                    layout: &self.expand_bgl,
                    entries: &[
                        entry(0, &self.uniform),
                        entry(1, &self.page_list),
                        entry(2, &self.counts),
                        entry(3, &self.pairs),
                        entry(4, &self.visible_counts),
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &self.levels,
                                offset: 0,
                                size: std::num::NonZeroU64::new(
                                    std::mem::size_of::<ExpandLevel>() as u64
                                ),
                            }),
                        },
                    ],
                });
                pass.set_bind_group(0, &expand_bg, &[level * self.level_stride as u32]);
                pass.set_bind_group(2, &visible_bg, &[]);
                pass.dispatch_workgroups_indirect(&self.expand_args, level as u64 * 12);
            }

            // 4. One draw for the whole clipmap, so its instance count
            //    is the whole pair list.
            pass.set_pipeline(&self.draw_args_pass);
            pass.set_bind_group(0, &compact_bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }

        let depth_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("page_depth_bg"),
            layout: &self.depth_bgl,
            entries: &[
                entry(0, &self.uniform),
                entry(1, &self.page_list),
                entry(2, &self.pairs),
            ],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow pages: depth"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.atlas_view,
                    depth_ops: Some(wgpu::Operations {
                        // Reversed-Z: 0 is far, so a page nothing drew
                        // into reads as "nothing between here and the
                        // light" rather than as fully shadowed.
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.depth);
            pass.set_bind_group(0, &depth_bg, &[]);
            pass.set_bind_group(1, meshlet_bg, &[]);
            pass.set_bind_group(2, &instances_bg, &[]);
            pass.draw_indirect(&self.draw_args, 0);
        }

        self.readback.record(encoder, &self.counts);
    }

    /// Maps this frame's counters and picks up whatever earlier frames
    /// returned. Call **after** the encoder has been submitted.
    pub fn poll(&mut self) -> Option<RasterCounts> {
        let words = self.readback.poll()?;
        Some(self.decode(&words))
    }
}

fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

fn buffer_entry(
    binding: u32,
    read_only: bool,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry(
    binding: u32,
    dynamic: bool,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_layout(device: &wgpu::Device, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("page_raster_storage_bgl"),
        entries: &[buffer_entry(0, true, visibility)],
    })
}

fn compact_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let c = wgpu::ShaderStages::COMPUTE;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("page_compact_bgl"),
        entries: &[
            uniform_entry(0, false, c),
            buffer_entry(1, true, c),
            buffer_entry(2, true, c),
            buffer_entry(3, false, c),
            buffer_entry(4, false, c),
            buffer_entry(5, false, c),
            buffer_entry(6, true, c),
            buffer_entry(7, false, c),
        ],
    })
}

fn expand_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let c = wgpu::ShaderStages::COMPUTE;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("page_expand_bgl"),
        entries: &[
            uniform_entry(0, false, c),
            buffer_entry(1, true, c),
            buffer_entry(2, false, c),
            buffer_entry(3, false, c),
            buffer_entry(4, true, c),
            uniform_entry(5, true, c),
        ],
    })
}

fn depth_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let v = wgpu::ShaderStages::VERTEX;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("page_depth_bgl"),
        entries: &[
            uniform_entry(0, false, v),
            buffer_entry(1, true, v),
            buffer_entry(2, true, v),
        ],
    })
}

/// The three-slot ring the raster's counters come home in.
///
/// The same state machine `ClusterReadback` and the marking pass use.
/// 🔴 `map_async` before the encoder is submitted is a validation error,
/// which is why the copy and the map are two calls and not one.
pub struct RasterReadback {
    slots: Vec<(wgpu::Buffer, std::sync::Arc<std::sync::Mutex<SlotState>>)>,
    next: usize,
    pending: Option<usize>,
    slot_words: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    Writable,
    InFlight,
    Ready,
}

impl RasterReadback {
    pub fn new(device: &wgpu::Device, words: u32) -> Self {
        let size = words as u64 * 4;
        Self {
            slots: (0..3)
                .map(|i| {
                    (
                        device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some(&format!("page_raster_readback_{i}")),
                            size,
                            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        }),
                        std::sync::Arc::new(std::sync::Mutex::new(SlotState::Writable)),
                    )
                })
                .collect(),
            next: 0,
            pending: None,
            slot_words: words as usize,
        }
    }

    /// Copies the counters into a free slot. A frame with none simply
    /// skips: the cached count is one frame older, which is the same
    /// kind of stale it already was.
    pub fn record(&mut self, encoder: &mut wgpu::CommandEncoder, counters: &wgpu::Buffer) {
        let Some(index) = self.acquire() else {
            return;
        };
        encoder.copy_buffer_to_buffer(
            counters,
            0,
            &self.slots[index].0,
            0,
            self.slot_words as u64 * 4,
        );
        self.pending = Some(index);
    }

    /// Maps what was recorded and returns whatever earlier frames
    /// finished. Call once a frame, **after** the submit.
    pub fn poll(&mut self) -> Option<Vec<u32>> {
        if let Some(index) = self.pending.take() {
            let (buffer, state) = &self.slots[index];
            *state.lock().unwrap() = SlotState::InFlight;
            let flag = state.clone();
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    if result.is_ok() {
                        *flag.lock().unwrap() = SlotState::Ready;
                    }
                });
        }
        for (buffer, state) in &self.slots {
            if *state.lock().unwrap() != SlotState::Ready {
                continue;
            }
            let words = {
                let view = buffer.slice(..).get_mapped_range();
                bytemuck::cast_slice::<u8, u32>(&view).to_vec()
            };
            buffer.unmap();
            *state.lock().unwrap() = SlotState::Writable;
            return Some(words);
        }
        None
    }

    fn acquire(&mut self) -> Option<usize> {
        for _ in 0..self.slots.len() {
            let index = self.next;
            self.next = (self.next + 1) % self.slots.len();
            if *self.slots[index].1.lock().unwrap() == SlotState::Writable {
                return Some(index);
            }
        }
        None
    }
}
