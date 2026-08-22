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

use kooch_lighting::PAGE_TABLE as TABLE;
const COMPACT: &str = include_str!("../../../shaders/page_compact.wgsl");
const EXPAND: &str = include_str!("../../../shaders/page_expand.wgsl");
const DEPTH: &str = include_str!("../../../shaders/page_depth.wgsl");

/// What the pages are rasterised at. The same format the cascades use,
/// so the sampling path in #477 reaches for the same comparison sampler
/// rather than for a second one.
pub const PAGE_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Which winding the page transform calls a front face.
///
/// 🔴 **`Cw`, and the cascades' `Ccw` is not a discrepancy to tidy up.**
/// Two flips sit between a world triangle and this page's clip space,
/// and only one of them is part of the 2D map the rasteriser winds by:
///
/// - `sun_basis` returns `(s, u, f)` with `u = cross(s, f)`, whose
///   determinant is **-1**. It is left-handed, unlike the cascades'
///   `look_to_rh`.
/// - `page_clip` negates Y, because a page's rect is in texel rows —
///   which run down — and clip space runs up. The reader agrees with
///   that flip, so removing it would mirror every shadow instead.
///
/// Measured on the GPU through those very functions: a triangle facing
/// the light comes out with a signed area of **-0.25**. Declaring `Ccw`
/// therefore made `cull_mode: Back` discard every surface that casts and
/// keep the far shell of every closed mesh — blobby shadows, full of
/// holes, changing shape with the clipmap level.
///
/// A constant rather than a literal in the descriptor because it is half
/// of a pair: `a_light_facing_triangle_is_the_front_face` asserts the
/// shader and this agree.
pub const PAGE_FRONT_FACE: wgpu::FrontFace = wgpu::FrontFace::Cw;

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
    pool.slice()
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
    /// Local-light pages that DID reach a bucket of `page_list`.
    ///
    /// 🔴 Listed is not drawn. The compaction now buckets them by their
    /// chain level, which is what makes the shape of the local half
    /// visible for the first time — how many pages sit at each LOD, and
    /// therefore how much geometry each one will ask for. Nothing
    /// expands them yet: their buckets have no survivor list, so their
    /// dispatch is sized zero.
    pub listed: u32,
    /// `(page, meshlet)` pairs the draw covered.
    pub pairs: u32,
    /// Pairs past the list's capacity.
    pub overflow: u32,
    /// Resident pages belonging to ANOTHER camera.
    ///
    /// 🔴 Not a failure — the table is shared and every view compacts
    /// its own — but the number without which "the pool is full and my
    /// view got forty pages" cannot be read.
    pub others: u32,
    /// Which camera this is.
    pub view: u32,
    /// Meshlet/page tests the expansion ran, summed over the levels.
    ///
    /// 🔴 The expansion is a product — this level's pages times this
    /// level's survivors — so its cost is not the pairs it emits but
    /// the combinations it walks to find them. A ratio of tests to
    /// pairs is how much of the pass is spent proving a miss, and it is
    /// the number that decides the shape of the local-light raster.
    pub tests: u64,
    /// The level that ran the most tests, and how many.
    pub worst: (u32, u64),
    /// What the OTHER shape of the expansion would have cost: cells a
    /// scatter would visit, summed over the levels.
    ///
    /// 🔴 Measured, not run. See `count_scatter` in `page_expand.wgsl`
    /// for why the two shapes win at opposite ends of the chain and why
    /// shipping one of them for every level cost two thirds of the
    /// frame rate the last time it was guessed at.
    pub scatter: u64,
    /// Tests a per-level hybrid would run: the cheaper of the two
    /// shapes at every level, summed.
    ///
    /// The gap between this and [`Self::tests`] is the entire prize on
    /// offer, and it is the only number that says whether the hybrid is
    /// worth building.
    pub hybrid: u64,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RasterUniform {
    space: [u32; 4],
    views: [u32; 4],
    pool: [u32; 4],
    chain: [u32; 4],
    world: [f32; 4],
    eye: [f32; 4],
    sun: [f32; 4],
    local: [u32; 4],
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
    /// The whole array, for whatever samples it.
    atlas_view: wgpu::TextureView,
    /// One 2D view per layer, for the render pass that fills it.
    layers: Vec<wgpu::TextureView>,
    /// One slice per camera. See [`PageRasterizer::uniform_span`].
    uniform: wgpu::Buffer,
    uniform_stride: u64,
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

    /// This frame's index, for the age debug view. See `views.w`.
    frame: u32,
    /// Triangles a meshlet may hold — the builder's cap, and the fixed
    /// vertex count the indirect draw issues.
    triangles: u32,
    culls: Vec<MeshletCull>,
    /// The bind groups that never change once built.
    ///
    /// 🔴 Built ONCE, not per level per view per frame. They used to be
    /// created inside the loop over the seventeen clipmap levels, which
    /// is 34 allocations per camera per frame before counting the culls
    /// — the "you are allocating an enormous amount" the profile and the
    /// naked eye both caught. Everything that varies per view now
    /// travels as a dynamic offset instead.
    bound: Option<Bound>,
    readback: RasterReadback,
    config: PageConfig,
    clipmap: ClipmapConfig,
    pool: PoolConfig,
}

/// Every bind group the four passes need, keyed by what invalidates it.
struct Bound {
    compact: wgpu::BindGroup,
    expand: wgpu::BindGroup,
    depth: wgpu::BindGroup,
    /// One per clipmap level: each level's cull owns its own visible
    /// list, so this is the one thing a single dispatch could not
    /// replace without the culls sharing an output buffer.
    visible: Vec<wgpu::BindGroup>,
    instances: wgpu::BindGroup,
    descriptors: wgpu::BindGroup,
    /// What the groups above were built against. A pool resize, a scene
    /// that grew or a reallocated instance buffer all land here.
    keys: BoundKeys,
}

/// Handles compare by identity in wgpu, which is what `Lights` already
/// leans on to decide whether its own bind group has to be rebuilt.
#[derive(PartialEq)]
struct BoundKeys {
    keys: wgpu::Buffer,
    slots: wgpu::Buffer,
    instances: wgpu::Buffer,
    descriptors: wgpu::Buffer,
    visible: Vec<wgpu::Buffer>,
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
        let atlas_view = atlas.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let layers = (0..atlas.depth_or_array_layers())
            .map(|layer| {
                atlas.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("shadow_page_atlas_layer"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let levels = clipmap.levels;
        let buckets = levels + config.levels();

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
                front_face: PAGE_FRONT_FACE,
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
        // 🔴 Rounded UP to a multiple, not `max`. A dynamic offset has
        // to be a multiple of `min_uniform_buffer_offset_alignment`, and
        // `max` only guarantees it when the struct is smaller than the
        // alignment. This one is 112 bytes: on a device that aligns to
        // 64 the `max` would give a 112-byte stride and every camera
        // past the first would bind at an illegal offset.
        let uniform_stride = (std::mem::size_of::<RasterUniform>() as u64)
            .div_ceil(align)
            .max(1)
            * align;
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;

        Self {
            atlas,
            atlas_view,
            layers,
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_uniform"),
                size: uniform_stride * atlas_layers(pool) as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            uniform_stride,
            page_list: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_list"),
                size: bucket(pool) as u64 * buckets as u64 * 8,
                // 🔴 COPY_SRC because `page_list_buffer` is public and the
                // only reason to expose a GPU buffer is to read it back.
                usage: storage | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            counts: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_counts"),
                size: count_slots(buckets) as u64 * 4,
                usage: storage | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            expand_args: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_expand_args"),
                size: buckets as u64 * 12,
                usage: storage | wgpu::BufferUsages::INDIRECT,
                mapped_at_creation: false,
            }),
            draw_args: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_draw_args"),
                size: 16,
                usage: storage | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC,
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
                size: buckets as u64 * 4,
                // 🔴 COPY_SRC: the survivor counts ride home in the same
                // readback as the page counts, so the expansion's cost
                // can be read as the product it is. See `count_slots`.
                usage: storage | wgpu::BufferUsages::COPY_SRC,
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
            frame: 0,
            triangles: max_triangles_per_meshlet.max(1),
            culls: (0..levels)
                .map(|_| MeshletCull::new(device, 1, max_triangles_per_meshlet))
                .collect(),
            bound: None,
            readback: RasterReadback::new(device, count_slots(buckets)),
            config,
            clipmap,
            pool,
        }
    }

    /// The depth atlas every resident page is rasterised into: the whole
    /// array, one layer per camera.
    pub fn atlas(&self) -> &wgpu::TextureView {
        &self.atlas_view
    }

    /// Where a camera's slice of the uniform starts and how far it runs.
    /// Where this camera's slice of the uniform starts.
    ///
    /// # 🔴 One slice per camera, and no longer one per frame
    ///
    /// This was briefly double-buffered by frame parity, because the
    /// marking ran AFTER the fused pass: the table and atlas the shading
    /// sampled were then a frame old, while `Queue::write_buffer` — which
    /// wgpu applies at the top of the submit, ahead of every command in
    /// it — handed that same pass this frame's eye and sun. The reader
    /// re-based the clipmap a frame ahead of the pages it was searching.
    ///
    /// The marking now runs BEFORE the fused pass, so the table, the
    /// atlas and the uniform are all this frame's and the hazard is
    /// gone with the ordering that caused it. The write jumping to the
    /// front of the submit is now exactly what is wanted.
    pub fn uniform_span(&self, view: u32) -> (u64, u64) {
        let layers = atlas_layers(self.pool);
        (
            self.uniform_stride * view.min(layers - 1) as u64,
            std::mem::size_of::<RasterUniform>() as u64,
        )
    }

    pub fn atlas_texture(&self) -> &wgpu::Texture {
        &self.atlas
    }

    /// What the atlas costs, which is the whole point of a pool.
    pub fn atlas_bytes(&self) -> u64 {
        self.pool.atlas_bytes(self.config)
    }

    /// The uniform every page pass reads, including the shading model
    /// that samples what this draws.
    pub fn uniform_buffer(&self) -> &wgpu::Buffer {
        &self.uniform
    }

    /// The counters, for whoever reads them back.
    pub fn counts_buffer(&self) -> &wgpu::Buffer {
        &self.counts
    }

    /// Triangles a meshlet may hold, which is the vertex count the
    /// indirect draw issues divided by three.
    /// Stamps the frame the age debug view measures against.
    pub fn set_frame(&mut self, frame: u32) {
        self.frame = frame;
    }

    pub fn triangles_per_meshlet(&self) -> u32 {
        self.triangles
    }

    /// The draw arguments the compaction wrote, for whoever reads them
    /// back. `COPY_SRC` so a test can.
    pub fn draw_args_buffer(&self) -> &wgpu::Buffer {
        &self.draw_args
    }

    /// Buckets in `page_list`: the sun's clipmap levels, then a local
    /// light's chain levels.
    ///
    /// 🔴 A bucket is a LOD and NOT a light. Every lamp shares the local
    /// run, because a page carries the light it belongs to in its own
    /// key — nothing downstream needs the list split by lamp. A bucket
    /// per light per level is the 4848-view shape this avoids.
    pub fn buckets(&self) -> u32 {
        self.clipmap.levels + self.config.levels()
    }

    /// Slots in [`Self::counts_buffer`].
    pub fn count_slots(&self) -> u32 {
        count_slots(self.buckets())
    }

    /// Reads the counters out of a mapped copy of [`Self::counts_buffer`].
    pub fn decode(&self, words: &[u32], view: u32) -> RasterCounts {
        let sun_levels = self.clipmap.levels as usize;
        let levels = self.buckets() as usize;
        let cap = bucket(self.pool);
        let mut tests = 0u64;
        let mut worst = (0u32, 0u64);
        let mut scatter = 0u64;
        let mut hybrid = 0u64;
        for level in 0..levels {
            let pages = words[level].min(cap) as u64;
            let meshlets = words.get(levels + 5 + level).copied().unwrap_or(0) as u64;
            let cells = words.get(levels * 2 + 5 + level).copied().unwrap_or(0) as u64;
            let work = pages * meshlets;
            tests += work;
            scatter += cells;
            // The choice a hybrid would make at this level, which is
            // the only place the choice can be made: the two shapes
            // cross over somewhere in the middle of the chain and
            // neither end knows where.
            hybrid += work.min(cells);
            if work > worst.1 {
                worst = (level as u32, work);
            }
        }
        RasterCounts {
            tests,
            worst,
            scatter,
            hybrid,
            pages: words[..sun_levels].iter().map(|&n| n.min(cap)).sum(),
            listed: words[sun_levels..levels].iter().map(|&n| n.min(cap)).sum(),
            dropped: words[levels],
            local: words[levels + 1],
            pairs: words[levels + 2].min(PAIR_CAPACITY),
            overflow: words[levels + 3],
            others: words[levels + 4],
            view,
        }
    }
}

/// Layers the atlas really has, which is the view count the pool was
/// built for.
fn atlas_layers(pool: PoolConfig) -> u32 {
    pool.slices()
}

/// Pairs one frame may draw.
///
/// 8 MiB, and a ceiling rather than a guess: `RasterCounts::overflow`
/// says when it was reached, which is the difference between a bound
/// and a silent truncation.
pub const PAIR_CAPACITY: u32 = 1 << 20;

fn count_slots(buckets: u32) -> u32 {
    // Per level, then: bucket overflow, local pages skipped, pairs, pair
    // overflow, pages owned by another view — and THEN the survivors
    // each level's cull produced, copied in from `visible_counts`.
    //
    // 🔴 The second run is what makes the expansion's cost readable. Its
    // work is pages TIMES meshlets per level, and both halves were
    // already on the GPU in different buffers, so the only thing missing
    // was bringing them home together. Nothing is counted at dispatch
    // time: an atomic per thread would cost more than the number is
    // worth, and the product is exact anyway.
    //
    // The THIRD run is the cost of the shape this pass does not use —
    // the cells a scatter would visit — so the two can be compared per
    // level instead of guessed at. That one IS counted at dispatch
    // time, because unlike the product it is not a number two buffers
    // already hold.
    buckets * 3 + 5
}

/// The atlas: one square layer per camera.
///
/// 🔴 An ARRAY and not one big surface, and that is the whole shape of
/// this change. A layer is an attachment a camera owns: it clears it
/// with a plain `LoadOp::Clear` and cannot reach the other camera's,
/// which is what lets a shared pool be emptied and refilled by one view
/// while the other is still sampling last frame's pages. The
/// alternatives — a scissor, a stencil, a clearing draw — all partition
/// one surface and all of them are a rule somebody has to keep.
fn atlas_texture(device: &wgpu::Device, config: PageConfig, pool: PoolConfig) -> wgpu::Texture {
    let side = pool.per_row() * config.page;
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow_page_atlas"),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: pool.slices(),
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
    fn write_uniform(&self, queue: &wgpu::Queue, view: u32, eye: Vec3, sun: Vec3, lights: u32) {
        let d = sun.normalize_or(Vec3::NEG_Y);
        // The sun's slot is one past the last light, the way marking
        // assigns it, so a view addresses one more slot than there are
        // lights.
        let sun_slot = lights.max(1);
        let stride = super::mark::stride(self.config, self.clipmap);
        queue.write_buffer(
            &self.uniform,
            self.uniform_span(view).0,
            bytemuck::bytes_of(&RasterUniform {
                space: [
                    stride,
                    self.config.face_pages(),
                    self.config.side(0),
                    sun_slot,
                ],
                views: [
                    view.min(atlas_layers(self.pool) - 1),
                    stride * (sun_slot + 1),
                    self.pool.slice(),
                    // 🔴 Only the age debug view reads this. A page's age
                    // is a difference against the current frame, and the
                    // shading pass has no other way to know what frame
                    // it is in.
                    self.frame,
                ],
                pool: [
                    self.pool.entries(),
                    self.pool.total(),
                    self.pool.per_row(),
                    self.config.page,
                ],
                chain: [
                    self.clipmap.levels,
                    PAIR_CAPACITY,
                    bucket(self.pool),
                    // 🔴 Triangles a MESHLET may hold, which is the
                    // fixed vertex count the indirect draw issues —
                    // `max_triangles_per_meshlet * 3`, the same figure
                    // `MeshletCull::new` documents for the cascades'
                    // draw. It used to be `meshlets_per_mesh`, which is
                    // a different quantity entirely: the meshlet count
                    // of the registered mesh. At the engine's defaults
                    // that issued about a third of the vertices a
                    // meshlet needs, so every meshlet was drawn up to
                    // its fortieth triangle and cut. The shadows were
                    // fragments that followed the meshlet structure and
                    // rearranged themselves whenever the LOD changed.
                    self.triangles,
                ],
                world: [
                    self.clipmap.base,
                    SUN_SPAN,
                    // The side of ONE LAYER, which is what a page's clip
                    // position is placed inside.
                    (self.pool.per_row() * self.config.page) as f32,
                    0.0,
                ],
                eye: [eye.x, eye.y, eye.z, 0.0],
                sun: [d.x, d.y, d.z, 1.0],
                local: [self.config.levels(), self.buckets(), 0, 0],
            }),
        );
    }

    /// The table becomes a dense list, bucketed by level, and the
    /// expansion's dispatch sizes are computed from it.
    ///
    /// Public because it is the half that can be tested without a
    /// scene: hand it a table and the buckets say whether a page
    /// decodes back to the level it was encoded from.
    #[allow(clippy::too_many_arguments)]
    #[profiling::function]
    pub fn record_compaction(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        page_pool: &PagePool,
        view: u32,
        eye: Vec3,
        sun: Vec3,
        lights: u32,
    ) {
        self.write_uniform(queue, view, eye, sun, lights);
        encoder.clear_buffer(&self.counts, 0, None);
        encoder.clear_buffer(&self.expand_args, 0, None);
        encoder.clear_buffer(&self.draw_args, 0, None);
        let bind_group = self.compact_bind_group(device, page_pool);
        let offset = [self.uniform_span(view).0 as u32];
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("shadow pages: compact"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.compact);
        pass.set_bind_group(0, &bind_group, &offset);
        pass.dispatch_workgroups(self.pool.entries().div_ceil(64), 1, 1);
        pass.set_pipeline(&self.expand_args_pass);
        pass.dispatch_workgroups(self.clipmap.levels.div_ceil(64), 1, 1);
        // Without a pair list this only fixes the vertex count, which is
        // exactly the half worth asserting on without a scene.
        pass.set_pipeline(&self.draw_args_pass);
        pass.dispatch_workgroups(1, 1, 1);
    }

    fn compact_bind_group(&self, device: &wgpu::Device, page_pool: &PagePool) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("page_compact_bg"),
            layout: &self.compact_bgl,
            entries: &[
                self.uniform_entry(0),
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

    /// The uniform, bound as ONE camera's slice. The slice that gets
    /// read is picked by the dynamic offset at `set_bind_group` time,
    /// so the same group serves every camera.
    fn uniform_entry(&self, binding: u32) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &self.uniform,
                offset: 0,
                size: std::num::NonZeroU64::new(std::mem::size_of::<RasterUniform>() as u64),
            }),
        }
    }

    /// Builds every bind group the passes need, and only when one of the
    /// buffers behind them has actually been replaced.
    ///
    /// 🔴 The keys are compared, not assumed. A pool resize swaps the
    /// table, a scene that outgrows its cull swaps the visible lists and
    /// a reallocated instance buffer swaps that — and a cached group
    /// pointing at a freed buffer is a validation error per frame, which
    /// is the failure mode this project has already paid for once.
    fn ensure_bound(
        &mut self,
        device: &wgpu::Device,
        page_pool: &PagePool,
        instances: &wgpu::Buffer,
        descriptors: &wgpu::Buffer,
    ) {
        let keys = BoundKeys {
            keys: page_pool.keys().clone(),
            slots: page_pool.slots().clone(),
            instances: instances.clone(),
            descriptors: descriptors.clone(),
            visible: self
                .culls
                .iter()
                .map(|c| c.visible_meshlets_buffer().clone())
                .collect(),
        };
        if self.bound.as_ref().is_some_and(|b| b.keys == keys) {
            return;
        }
        let storage = |label: &str, buffer: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.storage_bgl,
                entries: &[entry(0, buffer)],
            })
        };
        self.bound = Some(Bound {
            compact: self.compact_bind_group(device, page_pool),
            expand: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("page_expand_bg"),
                layout: &self.expand_bgl,
                entries: &[
                    self.uniform_entry(0),
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
            }),
            depth: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("page_depth_bg"),
                layout: &self.depth_bgl,
                entries: &[
                    self.uniform_entry(0),
                    entry(1, &self.page_list),
                    entry(2, &self.pairs),
                ],
            }),
            visible: self
                .culls
                .iter()
                .map(|c| storage("page_expand_visible_bg", c.visible_meshlets_buffer()))
                .collect(),
            instances: storage("page_raster_instances_bg", instances),
            descriptors: storage("page_raster_descriptors_bg", descriptors),
            keys,
        });
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
    #[profiling::function]
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
        view: u32,
        eye: Vec3,
        sun: Vec3,
        lights: u32,
        lod_target: f32,
    ) {
        let levels = self.clipmap.levels;
        let buckets = self.buckets();
        let view = view.min(atlas_layers(self.pool) - 1);
        self.write_uniform(queue, view, eye, sun, lights);
        let uniform_offset = self.uniform_span(view).0 as u32;

        // 1. One cull per level. A level is a texel density and a
        //    density is a LOD.
        //
        // 🔴 Seventeen full cull dispatches per view per frame, each
        // writing a uniform and recording its own passes. This is the
        // only part of the track that is CPU work rather than GPU work,
        // and it was invisible to the profiler until this scope existed.
        // 🔴 The local buckets have no cull, so nothing ever writes
        // their survivor count — and an unwritten storage buffer is not
        // zero, it is whatever the allocator handed over. `cs_expand_args`
        // multiplies that by a page count and dispatches the result.
        encoder.clear_buffer(&self.visible_counts, 0, None);
        {
            profiling::scope!("cull: clipmap levels");
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
        }

        // Per view, all of it: the page list, the pair list and the
        // dispatch arguments describe THIS camera's clipmap and nothing
        // else. The table, the pool and the atlas are the shared things,
        // and none of them is cleared here.
        encoder.clear_buffer(&self.counts, 0, None);
        encoder.clear_buffer(&self.expand_args, 0, None);
        encoder.clear_buffer(&self.draw_args, 0, None);

        self.ensure_bound(device, page_pool, instances, &mesh_pool.meshlets);
        let bound = self.bound.as_ref().expect("just built");

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("shadow pages: compact and expand"),
                timestamp_writes: None,
            });
            // 2. The hash table becomes a dense list, bucketed by level
            //    — this camera's pages only, the rest counted and left.
            pass.set_pipeline(&self.compact);
            pass.set_bind_group(0, &bound.compact, &[uniform_offset]);
            pass.dispatch_workgroups(self.pool.entries().div_ceil(64), 1, 1);
            pass.set_pipeline(&self.expand_args_pass);
            pass.dispatch_workgroups(buckets.div_ceil(64), 1, 1);

            // 3. Pairs. One indirect dispatch per level, sized by the
            //    pass above rather than by a CPU guess. The only thing
            //    that changes between levels is two dynamic offsets and
            //    the visible list — no bind group is built here.
            pass.set_pipeline(&self.expand);
            pass.set_bind_group(1, &bound.descriptors, &[]);
            pass.set_bind_group(3, &bound.instances, &[]);
            // 🔴 The SUN's buckets only. A local light's pages are
            // listed and bucketed by their chain level, but no cull
            // produces a survivor list for those buckets yet, so there
            // is no `visible` bind group to hand them and their
            // `expand_args` are zero either way. The loop bound says
            // which half is missing; a bound of `buckets` would hand
            // them the wrong level's survivors instead.
            for level in 0..levels {
                pass.set_bind_group(
                    0,
                    &bound.expand,
                    &[uniform_offset, level * self.level_stride as u32],
                );
                pass.set_bind_group(2, &bound.visible[level as usize], &[]);
                pass.dispatch_workgroups_indirect(&self.expand_args, level as u64 * 12);
            }

            // 4. One draw for the whole clipmap, so its instance count
            //    is the whole pair list.
            pass.set_pipeline(&self.draw_args_pass);
            pass.set_bind_group(0, &bound.compact, &[uniform_offset]);
            pass.dispatch_workgroups(1, 1, 1);
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow pages: depth"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    // 🔴 THIS camera's layer. The clear below is the
                    // whole reason the atlas is an array: a camera
                    // wipes its own pages and cannot reach the ones the
                    // other camera is still sampling.
                    view: &self.layers[view as usize],
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
            pass.set_bind_group(0, &bound.depth, &[uniform_offset]);
            pass.set_bind_group(1, meshlet_bg, &[]);
            pass.set_bind_group(2, &bound.instances, &[]);
            pass.draw_indirect(&self.draw_args, 0);
        }

        // The survivor counts, brought home alongside the page counts so
        // the expansion's cost can be read as the product it is. See
        // `count_slots`.
        encoder.copy_buffer_to_buffer(
            &self.visible_counts,
            0,
            &self.counts,
            (self.buckets() as u64 + 5) * 4,
            self.buckets() as u64 * 4,
        );
        self.readback.record(encoder, &self.counts, view);
    }

    /// Maps this frame's counters and picks up whatever earlier frames
    /// returned. Call **after** the encoder has been submitted.
    pub fn poll(&mut self) -> Option<RasterCounts> {
        let (words, view) = self.readback.poll()?;
        Some(self.decode(&words, view))
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
            // Dynamic, so one bind group serves every camera: its slice
            // of the uniform travels as an offset instead of as a
            // second allocation.
            uniform_entry(0, true, c),
            buffer_entry(1, true, c),
            // 🔴 Writable now: the compaction records each page's place
            // in `page_list` back into its table entry, which is the
            // only pass that knows both. See `PAGE_CELL`.
            buffer_entry(2, false, c),
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
            uniform_entry(0, true, c),
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
            uniform_entry(0, true, v),
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
    /// Which camera each slot's copy was taken for, captured when the
    /// copy is RECORDED. The ring is frames deep and the cameras take
    /// turns, so asking the rasterizer at map time labels the number
    /// with whichever one ran last.
    views: Vec<u32>,
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
            views: vec![0; 3],
            next: 0,
            pending: None,
            slot_words: words as usize,
        }
    }

    /// Copies the counters into a free slot. A frame with none simply
    /// skips: the cached count is one frame older, which is the same
    /// kind of stale it already was.
    pub fn record(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        counters: &wgpu::Buffer,
        view: u32,
    ) {
        let Some(index) = self.acquire() else {
            return;
        };
        self.views[index] = view;
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
    pub fn poll(&mut self) -> Option<(Vec<u32>, u32)> {
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
        for (index, (buffer, state)) in self.slots.iter().enumerate() {
            if *state.lock().unwrap() != SlotState::Ready {
                continue;
            }
            let words = {
                let mapped = buffer.slice(..).get_mapped_range();
                bytemuck::cast_slice::<u8, u32>(&mapped).to_vec()
            };
            buffer.unmap();
            *state.lock().unwrap() = SlotState::Writable;
            return Some((words, self.views[index]));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// What the shader says a struct measures, per WGSL's own layout
    /// rules.
    fn shader_size(body: &str, name: &str) -> u32 {
        let source = format!("{TABLE}\n{body}");
        let module = naga::front::wgsl::parse_str(&source).expect("the shader parses");
        let mut layouter = naga::proc::Layouter::default();
        layouter
            .update(module.to_ctx())
            .expect("the shader has a layout");
        for (handle, ty) in module.types.iter() {
            if ty.name.as_deref() == Some(name) {
                return layouter[handle].size;
            }
        }
        panic!("`{name}` is not declared in this shader");
    }

    /// Where every field of a shader struct starts, in declaration
    /// order.
    ///
    /// 🔴 The half a size check cannot see. Two structs of the same
    /// size with two fields swapped measure identical and mean
    /// different things — and that is not hypothetical either: a field
    /// added after `paint` in the shader and before it in Rust broke
    /// the page DEBUG VIEW, a feature the change had not touched.
    pub fn shader_offsets(body: &str, name: &str) -> Vec<(String, u32)> {
        let source = format!("{TABLE}\n{body}");
        let module = naga::front::wgsl::parse_str(&source).expect("the shader parses");
        for (_, ty) in module.types.iter() {
            if ty.name.as_deref() != Some(name) {
                continue;
            }
            let naga::TypeInner::Struct { members, .. } = &ty.inner else {
                panic!("`{name}` is not a struct");
            };
            return members
                .iter()
                .map(|m| (m.name.clone().unwrap_or_default(), m.offset))
                .collect();
        }
        panic!("`{name}` is not declared in this shader");
    }

    /// 🔴 The bug class this exists for cost a frame that rendered
    /// nothing but validation errors, once per frame forever.
    ///
    /// `ExpandLevel` held a `vec3<u32>` for padding. A `vec3<u32>`
    /// **aligns to 16**, so the field started at offset 16 and the
    /// struct measured 32 bytes against the Rust mirror's 16. It
    /// compiles. It validates. It fails at BIND time — *"bound with
    /// size 16 where the shader expects 32"* — which is the one place
    /// no test was looking.
    ///
    /// A comment saying "mirrors X field for field" is not a check.
    /// This is.
    #[test]
    fn the_uniform_mirrors_match_the_shader() {
        assert_eq!(
            shader_size(COMPACT, "PageRaster") as usize,
            std::mem::size_of::<RasterUniform>(),
            "PageRaster",
        );
        assert_eq!(
            shader_size(EXPAND, "ExpandLevel") as usize,
            std::mem::size_of::<ExpandLevel>(),
            "ExpandLevel",
        );
    }

    /// The other half: same size, wrong order.
    #[test]
    fn the_uniform_fields_line_up() {
        let mine = [
            ("space", std::mem::offset_of!(RasterUniform, space)),
            ("views", std::mem::offset_of!(RasterUniform, views)),
            ("pool", std::mem::offset_of!(RasterUniform, pool)),
            ("chain", std::mem::offset_of!(RasterUniform, chain)),
            ("world", std::mem::offset_of!(RasterUniform, world)),
            ("eye", std::mem::offset_of!(RasterUniform, eye)),
            ("sun", std::mem::offset_of!(RasterUniform, sun)),
            ("local", std::mem::offset_of!(RasterUniform, local)),
        ];
        let theirs = shader_offsets(COMPACT, "PageRaster");
        assert_eq!(theirs.len(), mine.len(), "field count");
        for ((name, offset), (their_name, their_offset)) in mine.iter().zip(&theirs) {
            assert_eq!(name, their_name, "field order");
            assert_eq!(*offset as u32, *their_offset, "`{name}` starts elsewhere");
        }
    }

    /// The three runs of counters do not overlap, and the shader that
    /// writes the third one agrees with the Rust that reads it.
    ///
    /// Per BUCKET rather than per clipmap level: the sun's levels and a
    /// local light's chain levels both get one, so a run sized to the
    /// clipmap alone puts the survivors inside the overflow flags.
    ///
    /// 🔴 A counter buffer is one flat array of `u32` shared by four
    /// shaders and one `copy_buffer_to_buffer`, addressed by arithmetic
    /// written out twice. The first run is per level, the second is
    /// filled by a copy from `visible_counts`, the third is written by
    /// `count_scatter`. Getting the base of the third wrong does not
    /// fail: it lands in the survivor counts, which are plausible
    /// numbers, and the panel reports a comparison built on the wrong
    /// half of the buffer.
    ///
    /// This session already shipped one defect of exactly this shape —
    /// `page_compact.wgsl` reading a two-word table entry with a
    /// one-word stride — and it took a screen full of squares to find.
    #[test]
    fn the_counter_runs_do_not_overlap() {
        for buckets in [1u32, 4, 25, 40] {
            let n = buckets as usize;
            let slots = count_slots(buckets) as usize;
            // Run one: the pages per level. Run two: the survivors,
            // written by the copy at the end of `record`. Run three:
            // the scatter's cells.
            let survivors = n + 5;
            let scatter = n * 2 + 5;
            assert!(
                survivors + n <= scatter,
                "the survivor run runs into the scatter run at {buckets} buckets",
            );
            assert!(
                scatter + n <= slots,
                "the scatter run runs off the end at {buckets} buckets",
            );
        }
        // And the shader addresses the third run the same way `decode`
        // does. A comment claiming they match is not a check.
        assert!(
            EXPAND.contains("page_counts[buckets * 2u + 5u + level]"),
            "`count_scatter` no longer writes where `decode` reads",
        );
    }

    /// Every buffer this pass copies OUT of declares that it can be.
    ///
    /// 🔴 Written after shipping a `copy_buffer_to_buffer` whose source
    /// lacked `COPY_SRC`. It compiles; it passes every test that plants
    /// words and decodes them; and it fails at RUNTIME, once per view
    /// per frame forever, with the shadow pass producing nothing. The
    /// tests around it never ran `record`, which is where the copy is.
    ///
    /// A source check rather than a GPU one, because the question is
    /// about a declaration and answering it on a device would mean
    /// building a whole frame to observe one flag.
    #[test]
    fn every_copied_buffer_can_be_copied_from() {
        let source = include_str!("raster.rs");
        // The buffers this pass copies out of, by the field name the
        // copy uses.
        let mut copied = copied_fields(source);
        assert!(
            !copied.is_empty(),
            "the scan found no copies at all; it has stopped matching the source"
        );
        // 🔴 And every buffer this module hands OUT. The only reason to
        // expose a GPU buffer is for something to read it back, and a
        // reader outside this file is a copy this scan cannot see — that
        // is exactly how `page_list` shipped without the flag, failing
        // only sometimes, because wgpu reports the error whenever it
        // gets round to it.
        let mut labels: Vec<String> = copied
            .iter()
            .map(|field| format!("page_raster_{field}"))
            .collect();
        // The label a buffer carries is not always its field name.
        labels.extend(
            [
                "page_raster_list",
                "page_raster_counts",
                "page_raster_draw_args",
            ]
            .into_iter()
            .map(String::from),
        );
        for label in labels {
            let at = source
                .find(&format!("Some(\"{label}\")"))
                .unwrap_or_else(|| panic!("{label} has no buffer descriptor"));
            let body = &source[at..at + 400];
            let usage = body
                .find("usage:")
                .map(|i| &body[i..body[i..].find(',').map(|j| i + j).unwrap_or(body.len())])
                .unwrap_or("");
            assert!(
                usage.contains("COPY_SRC"),
                "{label} is copied out of and its usage is `{usage}`"
            );
        }
    }

    /// The field each `copy_buffer_to_buffer` reads FROM — the first
    /// `&self.<field>` after the call, whatever the formatter did to the
    /// whitespace between them.
    fn copied_fields(source: &str) -> std::collections::BTreeSet<&str> {
        let mut out = std::collections::BTreeSet::new();
        let mut rest = source;
        while let Some(at) = rest.find("copy_buffer_to_buffer(") {
            let tail = &rest[at + "copy_buffer_to_buffer(".len()..];
            // 🔴 The FIRST argument only. The destination is the third,
            // and it needs COPY_DST rather than COPY_SRC — a scan that
            // took whichever `&self.` came first would demand the wrong
            // flag of the wrong buffer.
            let first = &tail[..tail.find(',').unwrap_or(0)];
            if let Some(field) = first.trim().strip_prefix("&self.") {
                let end = field
                    .find(|c: char| !c.is_alphanumeric() && c != '_')
                    .unwrap_or(field.len());
                if end > 0 {
                    out.insert(&field[..end]);
                }
            }
            rest = tail;
        }
        out
    }

    /// A camera's slice is its own, and nothing else's.
    #[test]
    fn a_view_owns_its_slice() {
        for views in 1..=4u32 {
            let pool = PoolConfig { pages: 2048, views };
            let slice = pool.slice();
            assert!(slice > 0);
            assert_eq!(pool.total(), slice * views, "every view gets one");
            let bases: Vec<u32> = (0..views).map(|v| pool.base(v)).collect();
            for pair in bases.windows(2) {
                assert_eq!(pair[1] - pair[0], slice, "the slices do not overlap");
            }
            assert_eq!(
                pool.base(views - 1) + slice,
                pool.total(),
                "the last slice ends where the pool does"
            );
        }
    }

    /// The pool is a budget, not a per-camera one. Splitting it must not
    /// multiply what it costs.
    #[test]
    fn slicing_does_not_grow_the_atlas() {
        let config = PageConfig::default();
        let one = PoolConfig {
            pages: 2048,
            views: 1,
        }
        .atlas_bytes(config);
        let two = PoolConfig {
            pages: 2048,
            views: 2,
        }
        .atlas_bytes(config);
        assert!(
            two <= one,
            "two cameras cost {two} bytes against one camera's {one}"
        );
    }
}
