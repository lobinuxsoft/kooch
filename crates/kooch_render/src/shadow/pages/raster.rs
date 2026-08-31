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
//! # 🔴 One cull per lamp, not per lamp VIEW
//!
//! A cull is per view. The sun's clipmap is **17** views; a hundred
//! local lights with six faces and an eight-level chain each would be
//! **4848**, which is the explosion this design refuses. What a lamp
//! actually needs is ONE survivor list: a perspective error metric
//! measured from the light's own position scales with distance by
//! itself, so every face and every level of one lamp can share it —
//! the retired cube path drew smooth shadows from exactly this recipe
//! (#777). The frame therefore runs `17 + punctual lights` culls,
//! capped at [`LAMP_CULLS`], and a lamp's pages bucket by LIGHT where
//! the sun's bucket by level.
//!
//! 🔴 Lamps must NOT borrow the sun's survivor lists. Those are
//! simplified for orthographic boxes centred on the CAMERA: a close
//! lamp's casters fell outside the fine levels' box and its shadow
//! vanished as it approached, and a coarse bucket handed root meshlets
//! that drew a sphere's shadow as a faceted lump. Measured in
//! `roll-a-ball`, both ways.

use glam::{Mat4, Vec3};

use crate::meshlet::{
    CullParams, GpuGlobalMeshPool, MeshletCull, MeshletCullPipelines, MeshletScene, SceneCullParams,
};

use super::pool::{PagePool, PoolConfig};
use super::pyramid::PagePyramid;
use super::{ClipmapConfig, PageConfig};

use kooch_core::gpu::{GpuQuery, GpuScopes};

/// The caller's open scope, for the four passes below to nest under.
///
/// 🔴 The four are a cull, a compact, an expansion and a draw, and they
/// scale with completely different things — levels, resident pages,
/// `(page, meshlet)` pairs, covered texels. One number over the set
/// says the track is expensive without saying which of those grew, so
/// it is a number nothing can act on.
pub type RasterTrack<'a> = Option<(&'a GpuScopes, &'a GpuQuery)>;

/// Opens `label` under `track`, or nothing when there is no profiler.
fn nested(
    track: RasterTrack<'_>,
    label: &str,
    encoder: &mut wgpu::CommandEncoder,
) -> Option<GpuQuery> {
    track.map(|(scopes, parent)| scopes.begin_child(label, encoder, parent))
}

/// Closes what [`nested`] opened.
fn close(track: RasterTrack<'_>, query: Option<GpuQuery>, encoder: &mut wgpu::CommandEncoder) {
    if let (Some((scopes, _)), Some(query)) = (track, query) {
        scopes.end(encoder, query);
    }
}

use kooch_lighting::PAGE_TABLE as TABLE;
// `ClusterLight` is declared here — the expansion tests a candidate
// against the lamp's own cone and the depth pass places its faces.
use kooch_lighting::CLUSTER_COMMON;
use kooch_lighting::{GpuLight, LIGHT_KIND_DIRECTIONAL};
const COMPACT: &str = include_str!("../../../shaders/page_compact.wgsl");
const EXPAND: &str = include_str!("../../../shaders/page_expand.wgsl");
const DEPTH: &str = include_str!("../../../shaders/page_depth.wgsl");
/// Appended to [`DEPTH`] only where `CLIP_DISTANCES` exists.
const DEPTH_CLIPPED: &str = include_str!("../../../shaders/page_depth_clipped.wgsl");

/// Lamp slots the raster addresses — lamp `L`'s pages land in bucket
/// `clipmap.levels + L`, fed by the hierarchical cull's slice for `L`
/// (#939). A light past the cap keeps its pages listed but undrawn,
/// counted with the dropped pages. Mirrors `LAMP_CULLS` in
/// `page_table.wgsl`.
///
/// 256 — the cluster path's own light budget — because `many_lights`
/// runs a hundred casting lamps and the previous 64 dropped a third of
/// them: 121 pages with no shadow, and every unshadowed light washing
/// out its neighbours'. The group-error arena is sized by the frame's
/// ACTIVE lights, not by this cap, so the cap prices buckets and
/// survivor slices only. Slots are buffer order, not ranked — the
/// classic path's `assign_point_slots` ranking is the follow-up named
/// in #939.
pub const LAMP_CULLS: u32 = 256;

/// Moved-caster spheres a frame may upload for page invalidation.
/// Past it, the scene generation bumps instead — every page redraws
/// once, which is coarse and never wrong.
const MOVED_CAPACITY: u32 = 256;

/// FNV-1a over a word, for the content generations. Collisions cache a
/// stale page for one configuration change in four billion; accepted.
fn fnv(mut hash: u32, word: u32) -> u32 {
    for byte in word.to_le_bytes() {
        hash = (hash ^ byte as u32).wrapping_mul(16777619);
    }
    hash
}

const FNV_SEED: u32 = 2166136261;

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
    pool.slots()
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
    /// allocated and rasterised. Reported rather than ignored: a pool
    /// that looks full for a reason nobody stated is how a budget gets
    /// mis-read, and lamps are what fills this one.
    pub local: u32,
    /// Pages listed for THIS view, the sun's and the lamps' together.
    ///
    /// They share buckets: a bucket is an octave of world texel size, so
    /// a lamp and the sun that want the same fineness are in the same
    /// list. [`Self::local`] is how many of them came from lamps.
    pub listed: u32,
    /// `(page, meshlet)` pairs the draw covered.
    pub pairs: u32,
    /// Pairs past the list's capacity.
    pub overflow: u32,
    /// Resident pages whose content stamp still matched — the pages the
    /// cache made free this frame.
    pub cached: u32,
    /// Lamp pairs the receiver bound turned away (#940): the caster's
    /// nearest point lay beyond every receiver the page shades, so
    /// drawing it could change nothing.
    pub depth_rejected: u32,
    /// The same, for the SUN (#949) — counted apart on purpose.
    ///
    /// 🔴 A lamp bounds by radius and the sun by its own axis, so the
    /// two answer different questions about a scene: lamps say how
    /// much of a room is behind the walls that light it, the sun says
    /// how deep the world is. Summed together they say neither, and
    /// the number this was built to read is whether a COMPACT scene
    /// leaves the sun's bound with nothing to reject.
    pub sun_rejected: u32,
    /// Which camera this is.
    pub view: u32,
    /// Meshlets the LAMPS' culls kept this frame, over every bucket.
    ///
    /// 🔴 Zero here with lamp pages resident is the failure this whole
    /// path keeps producing: the pages exist, their bucket has nothing
    /// to draw, so they are stamped `PAGE_EMPTY` and cleared — and a
    /// cleared page is far depth under reversed-Z, which every reader
    /// answers "nothing occludes". Every lamp stops casting and every
    /// other counter reads healthy.
    pub lamp_survivors: u32,
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
    /// Pages sitting in a bucket whose cull produced NO survivors.
    ///
    /// 🔴 These render LIT, and that is the whole reason the counter
    /// exists. `cs_expand_args` sizes the expansion as `pages *
    /// meshlets`, so a bucket holding pages and no meshlets dispatches
    /// zero threads and emits no pairs — while the pages themselves are
    /// still resident and still cleared. A cleared page stores 0, which
    /// is FAR under reversed-Z, so every reader over it answers
    /// "nothing occludes here": a bright patch, with the page present,
    /// allocated and correctly keyed.
    ///
    /// By sight it is indistinguishable from a missing page or from a
    /// bias that overshot. That is why it is a number.
    pub unfilled: u32,
    /// The lowest bucket in that state, so the reading names one.
    /// `u32::MAX` when there is none.
    pub unfilled_first: u32,
    /// How many of [`Self::unfilled`] belong to the SUN's clipmap.
    ///
    /// 🔴 The split is the reading, not a refinement of it. A LAMP with
    /// resident pages and no survivors is usually telling the truth:
    /// the marking makes a page resident because a RECEIVER asked to be
    /// shadowed there, and if no caster is within that light's reach
    /// then nothing occludes and an empty page answers correctly. It is
    /// wasted raster, not a wrong picture.
    ///
    /// The sun is the opposite. Its clipmap covers the whole view, so a
    /// level with pages and no survivors means its cull threw away
    /// geometry the marking had already committed pages to — and those
    /// pages render lit with a caster standing in them.
    pub unfilled_sun: u32,
    /// Pages the INVERTED expansion reached, counted where they happen
    /// (#1022).
    ///
    /// 🔴 The only honest figure for that shape. [`Self::tests`] is
    /// `pages * meshlets` — a product of two counters the CPU already
    /// had, exact for the paired shape and meaningless for this one,
    /// which touches a page only when the pyramid says something under
    /// the rectangle is being drawn. Zero while the expansion runs the
    /// paired way.
    pub walk: u64,
    /// Descents that ran out of stack.
    ///
    /// 🔴 Must be zero, and it is not a performance number. A descent
    /// that cannot push DROPS the subtree — a caster that stops being
    /// drawn into pages that asked for it, which is the exact artefact
    /// this whole line of work is chasing. The bound is `3 * depth + 4`
    /// and the stack is bigger than that today; the counter is there
    /// for the day the page size changes.
    pub walk_overflow: u32,
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
    bias: [f32; 4],
    /// `x` the atlas layer this pass is attached to, `y` its view.
    ///
    /// 🔴 A pass owns ONE layer, and with a view spread across several
    /// the draws have to know which. A page whose slot lands elsewhere
    /// emits a degenerate triangle rather than drawing at the same
    /// texels of the wrong layer — which is what "it does not fail, it
    /// corrupts" meant (#1016).
    layer: [u32; 4],
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
    /// One generation per bucket owner per view — the sun's levels
    /// (snapped centre, direction, the eye's height along the sun's
    /// axis), then the lamps (transform, range, cone). The compaction
    /// caches a page whose stamp matches. Never zero.
    gens: wgpu::Buffer,
    /// `[0]` count, then the physical slot of every page THIS view's
    /// compaction listed — what the depth pass clears instead of the
    /// whole layer.
    dirty: wgpu::Buffer,
    /// `[0].x` count, then world spheres of every caster that moved
    /// this frame, old and new bounds alike.
    moved: wgpu::Buffer,
    /// Folded into every generation. Bumped when the moved list
    /// overflows its buffer — the coarse, honest fallback — and when a
    /// pair overflow was observed, because a stamped page whose pairs
    /// were dropped cached a hole.
    scene_gen: u32,
    /// The frame the moved list was last uploaded and any overflow
    /// bump applied — once per frame, not per view.
    moved_frame: Option<u32>,
    /// Whether the moved list is currently past [`MOVED_CAPACITY`].
    ///
    /// Kept only so the report fires on the EDGE. The condition is a
    /// per-frame one and a line per frame at 150 Hz is not a report,
    /// it is a denial of service on the console.
    flooded: bool,
    /// The scene set this cache holds pages for.
    ///
    /// 🔴 `None` until the first frame, so a fresh rasterizer does not
    /// void a cache it has not filled. After that a mismatch means the
    /// world was replaced, and every stamp in the pool describes
    /// geometry that may no longer exist (#971).
    scene_epoch: Option<u32>,

    compact_bgl: wgpu::BindGroupLayout,
    compact: wgpu::ComputePipeline,
    expand_args_pass: wgpu::ComputePipeline,
    /// Fills `PAGE_LOD` so the reader jumps instead of walking.
    lod_offsets: wgpu::ComputePipeline,
    draw_args_pass: wgpu::ComputePipeline,

    expand_bgl: wgpu::BindGroupLayout,
    storage_bgl: wgpu::BindGroupLayout,
    expand: wgpu::ComputePipeline,

    depth_bgl: wgpu::BindGroupLayout,
    depth: wgpu::RenderPipeline,
    invalidate: wgpu::ComputePipeline,
    invalidate_bgl: wgpu::BindGroupLayout,
    /// One quad per dirty page at far depth, depth test `Always` —
    /// the per-page replacement for the whole-layer clear the cache
    /// retired.
    page_clear: wgpu::RenderPipeline,
    clear_bgl: wgpu::BindGroupLayout,

    /// This frame's index, for the age debug view. See `views.w`.
    frame: u32,
    /// The readers' PCF footprint width in texels, carried in
    /// `world.w`. 1 = bilinear. See `inti_page_filter` (#941).
    softness: u32,
    /// The readers' shadow bias, carried in `bias`: the normal step as
    /// a multiple of the texel, the step towards the light in metres, a
    /// ceiling on the first in metres (0 = none), and a ceiling on the
    /// receiver's own depth gradient (0 = the term is off).
    bias: [f32; 4],
    /// Whether the shading marches the atlas instead of sampling one
    /// texel through a PCF box (#1017). Carried to the shader in the
    /// raster uniform's spare word, which the shading binds anyway.
    march: bool,
    /// Whether the expansion runs from the GEOMETRY — one thread per
    /// surviving meshlet, descending the page pyramid to the pages it
    /// lands in — instead of pairing every listed page against every
    /// survivor (#1022). The sun's buckets only.
    geometry: bool,
    /// The page pyramid the inverted expansion descends. Built every
    /// frame after the compaction, whether or not anything reads it:
    /// the binding is part of the layout either way, and a texture the
    /// pass may sample has to hold this frame's answer.
    pyramid: PagePyramid,
    /// Triangles a meshlet may hold — the builder's cap, and the fixed
    /// vertex count the indirect draw issues.
    triangles: u32,
    culls: Vec<MeshletCull>,
    /// Whether the level culls enter per instance or per rectangle cell.
    /// See [`Self::set_two_level`].
    two_level: bool,
    /// The one hierarchical cull every lamp shares (#939). Its
    /// survivors land in fixed slices the expansion indexes by slot;
    /// no cull object, bind group or dispatch exists per lamp.
    lamp_cull: super::lamp_cull::LampCull,
    /// The frame [`Self::lamp_cull`] last recorded for. Its passes are
    /// view-independent, so the second camera of a frame reuses the
    /// first one's survivors instead of re-culling.
    lamp_frame: Option<u32>,
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
    invalidate: wgpu::BindGroup,
    clear: wgpu::BindGroup,
    /// One per clipmap level: each level's cull owns its own visible
    /// list, so this is the one thing a single dispatch could not
    /// replace without the culls sharing an output buffer.
    visible: Vec<wgpu::BindGroup>,
    /// The lamps' shared survivor arena — ONE group for every lamp
    /// bucket, because a slot's slice is arithmetic, not a binding.
    lamp_visible: wgpu::BindGroup,
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
    slots: wgpu::Buffer,
    instances: wgpu::Buffer,
    descriptors: wgpu::Buffer,
    // 🔴 In the key because it GROWS. The lights buffer is reallocated
    // when the scene outgrows it, and a cached bind group holding the
    // old one reads a lamp's range out of freed memory — or out of a
    // buffer that is simply somebody else's now.
    lights: wgpu::Buffer,
    visible: Vec<wgpu::Buffer>,
    /// The lamps' survivor arena — fixed-size today, in the key so a
    /// future growth path cannot silently skip the rebuild.
    lamp_survivors: wgpu::Buffer,
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
        // Sun buckets plus one bucket per lamp; every per-bucket buffer
        // is sized for both halves.
        let buckets = levels + LAMP_CULLS;

        let module = |label: &str, body: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(format!("{TABLE}\n{body}").into()),
            })
        };
        let compact_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("page_compact"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{CLUSTER_COMMON}\n{TABLE}\n{COMPACT}").into(),
            ),
        });
        // The expansion reaches for `ClusterLight` too: a lamp's page is
        // a frustum from the light's own position and range, not a slab.
        let expand_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("page_expand"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{CLUSTER_COMMON}\n{TABLE}\n{}\n{EXPAND}",
                    super::pyramid::OVERLAP
                )
                .into(),
            ),
        });
        // The depth pass builds a lamp's frustum from the light record.
        //
        // 🔴 `enable` has to be the FIRST thing in a module and wgpu
        // rejects the directive outright without the feature, so the
        // clipped path is a different SOURCE rather than a branch inside
        // one. `clipped` decides both halves — the prefix here and the
        // entry point below — and they cannot disagree.
        let clipped = device.features().contains(wgpu::Features::CLIP_DISTANCES);
        let enable = if clipped {
            "enable clip_distances;\n"
        } else {
            ""
        };
        let tail = if clipped { DEPTH_CLIPPED } else { "" };
        let depth_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("page_depth"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{enable}{CLUSTER_COMMON}\n{TABLE}\n{DEPTH}\n{tail}").into(),
            ),
        });

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
        let lod_offsets = compute("cs_lod_offsets", &compact_module, &compact_layout_pipeline);
        let draw_args_pass = compute("cs_draw_args", &compact_module, &compact_layout_pipeline);
        let invalidate_bgl = invalidate_layout(device);
        let invalidate_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("page_invalidate_layout"),
                bind_group_layouts: &[Some(&invalidate_bgl)],
                immediate_size: 0,
            });
        let invalidate = compute(
            "cs_invalidate",
            &compact_module,
            &invalidate_pipeline_layout,
        );

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
                entry_point: Some(if clipped {
                    "vs_page_clipped"
                } else {
                    "vs_page"
                }),
                buffers: &[],
                compilation_options: Default::default(),
            },
            // 🔴 A fragment stage where `shadow_depth` has none, and it
            // is not an oversight: it is the per-page scissor the
            // hardware cannot give per instance. See `page_depth.wgsl`.
            //
            // 🔴 …unless the clipper can be given the page's own four
            // edges, in which case there is nothing left to discard and
            // the stage goes away — which is also what puts a depth-only
            // pass on the hardware's double-rate path (#952).
            fragment: (!clipped).then(|| wgpu::FragmentState {
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

        let clear_bgl = clear_layout(device);
        let clear_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("page_clear_layout"),
                bind_group_layouts: &[Some(&clear_bgl)],
                immediate_size: 0,
            });
        let page_clear = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("page_clear"),
            layout: Some(&clear_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &depth_module,
                entry_point: Some("vs_page_clear"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            // No fragment and no scissor needed: the quad's corners ARE
            // the page's rect, so nothing rasterises past it.
            fragment: None,
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: PAGE_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                // A clear, as a draw: it always wins.
                depth_compare: Some(wgpu::CompareFunction::Always),
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
                // Four words per listing: page, slot, bound (#940), spare.
                size: bucket(pool) as u64 * buckets as u64 * 16,
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
                // Two draws: the pairs, then one quad per dirty page.
                size: 32,
                usage: storage | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            pairs: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_pairs"),
                size: PAIR_CAPACITY as u64 * 16,
                // COPY_SRC so a test can read the pairs back: the one
                // claim #1022 makes is that the two shapes emit the
                // SAME ones, and that is only checkable by comparing
                // the lists.
                usage: storage | wgpu::BufferUsages::COPY_SRC,
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
                size: level_stride * buckets as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            level_stride,
            gens: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_gens"),
                size: atlas_layers(pool) as u64 * buckets as u64 * 4,
                usage: storage,
                mapped_at_creation: false,
            }),
            dirty: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_dirty"),
                // A header word, then at most one slot per page a view
                // owns — the most one compaction can list.
                size: (1 + pool.slots() as u64) * 4,
                usage: storage,
                mapped_at_creation: false,
            }),
            moved: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_moved"),
                size: (1 + MOVED_CAPACITY as u64) * 16,
                usage: storage,
                mapped_at_creation: false,
            }),
            scene_gen: 0,
            scene_epoch: None,
            moved_frame: None,
            flooded: false,
            compact_bgl,
            compact,
            expand_args_pass,
            lod_offsets,
            draw_args_pass,
            expand_bgl,
            storage_bgl,
            expand,
            depth_bgl,
            depth,
            invalidate,
            invalidate_bgl,
            page_clear,
            clear_bgl,
            frame: 0,
            softness: 1,
            // What `inti_pbr.wgsl` held as constants before the
            // settings could reach it, so a project with no settings
            // file renders exactly as it did.
            bias: [1.8, 0.02, 0.0, 4.0],
            march: false,
            geometry: false,
            pyramid: PagePyramid::new(device, config, clipmap),
            triangles: max_triangles_per_meshlet.max(1),
            two_level: crate::meshlet::MeshletLodSettings::default().two_level,
            culls: (0..levels)
                .map(|_| MeshletCull::new(device, 1, max_triangles_per_meshlet))
                .collect(),
            lamp_cull: super::lamp_cull::LampCull::new(device),
            lamp_frame: None,
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
    /// The uniform slice a VIEW's readers bind — its first layer's.
    ///
    /// The shading samples the whole atlas through `page_place`, which
    /// resolves a layer from the slot, so any of the view's slices
    /// describes the world identically. Only the depth passes care
    /// which layer they are attached to; they use [`Self::layer_span`].
    pub fn uniform_span(&self, view: u32) -> (u64, u64) {
        self.layer_span(self.layer_of(view, 0))
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

    /// Voids every cached page when the world was replaced.
    ///
    /// # 🔴 Why the other invalidations cannot cover this
    ///
    /// Every one of them answers a *continuous* question. The moved
    /// list carries what shifted since last frame; `age_view` evicts
    /// what nobody asked for; the clipmap recycles the ring that
    /// scrolled out. All three assume the world persists and only some
    /// of it changed.
    ///
    /// Loading a scene breaks that assumption. **Despawning is not
    /// moving**: the outgoing entities did not shift, they stopped
    /// existing, so nothing put them on the moved list and their pages
    /// stayed resident — holding depth for geometry that no longer had
    /// anything to cast it. Sampled as the incoming scene's occlusion,
    /// it looked like large straight shadows that matched nothing on
    /// screen (#971).
    ///
    /// Loading is the mirror and just as quiet: entities that were
    /// never anywhere did not move either, so an additive load casts
    /// nothing until something unrelated forces a redraw.
    ///
    /// ⚠️ One bump per change, never per caster. UE5 invalidates per
    /// instance because it streams a world continuously; doing that on
    /// a load is how they reached a `DEVICE_HUNG` after level
    /// streaming. A scene change is rare and explicit — one full
    /// redraw is the cheap answer, and the right one until the
    /// by-reach invalidation of #866 exists.
    pub fn set_scene_epoch(&mut self, epoch: u32) {
        if self.scene_epoch == Some(epoch) {
            return;
        }
        // Not on the first frame: a rasterizer that has drawn nothing
        // has nothing to void, and bumping here would throw away the
        // pages the very first scene just filled.
        if self.scene_epoch.is_some() {
            // 🔴 info, not debug. This fires once per scene load — rare,
            // and the single line that answers "did the cache get
            // voided?" when shadows look wrong after a scene change.
            // Hidden behind debug it cost a diagnosis on the day it
            // shipped.
            tracing::info!(
                target: "kooch_render::shadow",
                epoch,
                generation = self.scene_gen.wrapping_add(1),
                "the scene set changed; voiding the page cache",
            );
            self.scene_gen = self.scene_gen.wrapping_add(1);
        }
        self.scene_epoch = Some(epoch);
    }

    /// The readers' PCF footprint width, from the settings. Takes
    /// effect at the next `write_uniform`.
    pub fn set_softness(&mut self, texels: u32) {
        self.softness = texels.max(1);
    }

    /// The readers' shadow bias, from the settings. Takes effect at the
    /// next `write_uniform`, the way the softness does.
    /// Which direction the expansion runs: pages against survivors, or
    /// one survivor down the pyramid to its pages.
    pub fn set_geometry(&mut self, on: bool) {
        self.geometry = on;
    }

    /// Which reader the shading uses: the PCF box, or the march.
    pub fn set_march(&mut self, on: bool) {
        self.march = on;
    }

    pub fn set_bias(&mut self, normal: f32, depth: f32, max_world: f32, slope: f32) {
        self.bias = [
            normal.max(0.0),
            depth.max(0.0),
            max_world.max(0.0),
            slope.max(0.0),
        ];
    }

    pub fn triangles_per_meshlet(&self) -> u32 {
        self.triangles
    }

    /// The draw arguments the compaction wrote, for whoever reads them
    /// back. `COPY_SRC` so a test can.
    pub fn draw_args_buffer(&self) -> &wgpu::Buffer {
        &self.draw_args
    }

    /// Buckets in `page_list`: the sun's clipmap levels first — one
    /// octave of world texel size each, anchored so level `L` lands on
    /// bucket `L` — then [`LAMP_CULLS`] buckets, one per lamp slot.
    ///
    /// 🔴 A bucket exists to name a survivor list, and a survivor
    /// list is a LOD picked for a VIEW. Lamps briefly shared the sun's
    /// buckets by octave; that borrowed geometry culled to the camera's
    /// orthographic boxes and broke lamp shadows both ways — see the
    /// module doc.
    pub fn buckets(&self) -> u32 {
        self.clipmap.levels + LAMP_CULLS
    }

    /// Slots in [`Self::counts_buffer`].
    pub fn count_slots(&self) -> u32 {
        count_slots(self.buckets())
    }

    /// Reads the counters out of a mapped copy of [`Self::counts_buffer`].
    pub fn decode(&self, words: &[u32], view: u32) -> RasterCounts {
        let levels = self.buckets() as usize;
        let cap = bucket(self.pool);
        let mut tests = 0u64;
        let mut worst = (0u32, 0u64);
        let mut scatter = 0u64;
        let mut hybrid = 0u64;
        let mut unfilled = 0u32;
        let mut unfilled_sun = 0u32;
        let mut unfilled_first = u32::MAX;
        let mut lamp_survivors = 0u64;
        for level in 0..levels {
            let pages = words[level].min(cap) as u64;
            let meshlets = words.get(levels + 5 + level).copied().unwrap_or(0) as u64;
            // 🔴 The lamps' half of the survivor mirror, summed. `unfilled`
            // says pages were cleared for want of geometry; only this says
            // whether the culls found any to begin with. One is a cull that
            // rejected everything, the other is a count that never arrived,
            // and they need opposite fixes.
            if (level as u32) >= self.clipmap.levels {
                lamp_survivors += meshlets;
            }
            let cells = words.get(levels * 2 + 5 + level).copied().unwrap_or(0) as u64;
            let work = pages * meshlets;
            // 🔴 Pages with nothing to draw into them. See `unfilled`,
            // and `unfilled_sun` for why the two halves read
            // differently.
            if pages > 0 && meshlets == 0 {
                unfilled += pages as u32;
                unfilled_first = unfilled_first.min(level as u32);
                if (level as u32) < self.clipmap.levels {
                    unfilled_sun += pages as u32;
                }
            }
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
            unfilled,
            unfilled_first,
            unfilled_sun,
            lamp_survivors: u32::try_from(lamp_survivors).unwrap_or(u32::MAX),
            pages: words[..levels].iter().map(|&n| n.min(cap)).sum(),
            // 🔴 Every listed page, the sun's and the lamps' alike,
            // because they share buckets now: a lamp and the sun that
            // want the same fineness are in the same list. `local` still
            // says how many of them came from lamps.
            listed: words[..levels].iter().map(|&n| n.min(cap)).sum(),
            dropped: words[levels],
            local: words[levels + 1],
            pairs: words[levels + 2].min(PAIR_CAPACITY),
            overflow: words[levels + 3],
            cached: words[levels + 4],
            depth_rejected: words.get(levels * 3 + 5).copied().unwrap_or(0),
            sun_rejected: words.get(levels * 3 + 6).copied().unwrap_or(0),
            walk: words.get(levels * 3 + 7).copied().unwrap_or(0) as u64,
            walk_overflow: words.get(levels * 3 + 8).copied().unwrap_or(0),
            view,
        }
    }
}

/// Layers the atlas really has, which is the view count the pool was
/// built for.
fn atlas_layers(pool: PoolConfig) -> u32 {
    pool.layers()
}

/// Pairs one frame may draw.
///
/// 4 MiB at four words a pair, and a ceiling rather than a guess:
/// `RasterCounts::overflow` says when it was reached, which is the
/// difference between a bound and a silent truncation.
///
/// 🔴 A quarter of what it was, and still two orders of magnitude past
/// what a frame emits — the roll-a-ball stress scene measures around a
/// thousand. It came down when the pair grew to carry its page and slot
/// directly: the indirection through `page_list` cost the vertex stage
/// a storage binding it did not have, and 1M pairs was a number nobody
/// had ever approached.
pub const PAIR_CAPACITY: u32 = 1 << 18;

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
    //
    // Plus four at the very end: pairs the receiver bound rejected, the
    // lamps' (#940) and the sun's (#949), kept apart because they
    // measure different properties of a scene — then the inverted
    // expansion's own two, the pages its descents reached and the
    // descents that ran out of stack (#1022). The first of those is
    // the only honest cost figure for that shape: `pages * meshlets`
    // is the product the paired one pays and says nothing about a walk
    // that visits what the pyramid points at.
    buckets * 3 + 9
}

/// The sun's frame: right, up, and the direction it shines along.
///
/// Mirrors `sun_basis` in `page_table.wgsl` term for term. The sun has
/// no position, so this is the only place its orientation means
/// anything, and a second copy free to pick a different `up` would cull
/// against one grid and rasterise into another.
fn sun_frame(sun: Vec3) -> (Vec3, Vec3, Vec3) {
    let f = sun.normalize_or(Vec3::NEG_Y);
    let up = if f.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    let right = f.cross(up).normalize();
    (right, right.cross(f), f)
}

/// The world point a level's cull box is centred on.
///
/// # 🔴 The page WINDOW's centre, and not the camera
///
/// This used to be the eye, and the difference is a band of geometry
/// that is culled while its pages are drawn.
///
/// `sun_window` puts a level's window at `floor(plane / width) - 64`
/// pages, so the window runs from `p - f - 64w` to `p - f + 64w` where
/// `f` is how far the camera sits into its own page. A box centred on
/// the eye runs from `p - 64w` to `p + 64w`. The two are the same SIZE
/// and offset by `f`: the window's lowest band, up to a whole page
/// wide, lies outside the box on every axis.
///
/// A caster whose bounds fall entirely in that band is culled — while
/// the pages covering it were marked by their receivers and get drawn
/// anyway, empty. An empty page stores far depth under reversed-Z, so
/// every reader over it answers "nothing occludes here": a lit band at
/// each level's edge, which is a ring at a fixed distance from the
/// camera. And `f` changes as the camera moves, so the ring crawls.
///
/// The depth axis gets the same treatment for the same reason:
/// `sun_drift` measures a page's stored depth from `floor(along / width
/// + 0.5) * width`, so the box's near and far planes are centred there
/// rather than on the camera.
fn level_origin(base: f32, side: u32, level: u32, eye: Vec3, sun: Vec3) -> Vec3 {
    let (right, up, f) = sun_frame(sun);
    let s = side.max(1) as f32;
    let width = base * (level as f32).exp2() / s;
    let plane = glam::Vec2::new(eye.dot(right), eye.dot(up));
    let low = (plane / width).floor() - glam::Vec2::splat((s * 0.5).floor());
    let centre = (low + glam::Vec2::splat(s * 0.5)) * width;
    // ⚠️ `floor(x + 0.5)` and never `round`, mirroring `sun_drift`:
    // WGSL rounds halves to even and Rust rounds them away from zero.
    let along = (eye.dot(f) / width + 0.5).floor() * width;
    right * centre.x + up * centre.y + f * along
}

/// The clipmap level's orthographic clip-from-world.
///
/// 🔴 Built to agree with `sun_basis` and `sun_page_rect` in the shader,
/// term for term. This matrix decides which meshlets survive and those
/// two decide where they land, so a disagreement is geometry culled for
/// one page and drawn into another. Free rather than a method so a test
/// can hold it against `sun_window`'s own arithmetic without a device.
fn level_clip(clipmap: ClipmapConfig, side: u32, level: u32, eye: Vec3, sun: Vec3) -> Mat4 {
    let (right, up, f) = sun_frame(sun);
    let rotation = Mat4::from_cols(
        glam::Vec4::new(right.x, up.x, f.x, 0.0),
        glam::Vec4::new(right.y, up.y, f.y, 0.0),
        glam::Vec4::new(right.z, up.z, f.z, 0.0),
        glam::Vec4::W,
    );
    let half = clipmap.extent(level) * 0.5;
    // Reversed-Z orthographic: 1 at the near plane, 0 at the far,
    // matching `page_depth.wgsl`'s `1 - (z + span) / (2 * span)`.
    let projection = Mat4::from_cols(
        glam::Vec4::new(1.0 / half, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, 1.0 / half, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, -1.0 / (2.0 * SUN_SPAN), 0.0),
        glam::Vec4::new(0.0, 0.0, 0.5, 1.0),
    );
    projection
        * rotation
        * Mat4::from_translation(-level_origin(clipmap.base, side, level, eye, sun))
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
            depth_or_array_layers: pool.layers(),
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: PAGE_DEPTH_FORMAT,
        // COPY_SRC is for the end-to-end rig
        // (`a_lamp_page_holds_what_its_light_sees`), which reads pages
        // back and checks them against the scene — the class of defect
        // that until then was only ever caught by a person staring at a
        // broken frame. The flag costs nothing at runtime.
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// The content stamp of every sun clipmap level, for the cache gate.
///
/// A level's stamp turns over when the camera crosses one of ITS pages —
/// on the plane, or along the sun — or when the sun turns, or when the
/// scene changes. All three spatial terms share that level's page width,
/// which is what lets a coarse level stay resident while a fine one
/// churns, and a clipmap is only worth its levels if that is true.
///
/// 🔴 A free function over the config rather than a method, because the
/// question it answers — *does this camera move void the cache?* — has
/// nothing to do with a GPU and should not need one to ask.
fn sun_gens(clipmap: ClipmapConfig, side: f32, scene_gen: u32, eye: Vec3, sun: Vec3) -> Vec<u32> {
    // Mirrors `sun_basis` in `page_table.wgsl`, term for term.
    let f = sun.normalize_or(Vec3::NEG_Y);
    // Only the sun's own axis is left: the basis' two in-plane vectors
    // went with the snapped centre this no longer hashes.
    let along = eye.dot(f);
    (0..clipmap.levels as usize)
        .map(|level| {
            let width = clipmap.base * (level as f32).exp2() / side;
            let mut h = FNV_SEED;
            for word in [
                // 🔴 The snapped CENTRE is deliberately absent, and used
                // to be two of these words. It turned a whole level's
                // content over every time the camera crossed one of its
                // pages — for pages whose world footprint had not moved
                // at all. `sun_cell` keys a page by its absolute world
                // position now and `cs_compact` folds that index into
                // the stamp per page, so scrolling the window
                // invalidates only the ring that wrapped.
                //
                // ⚠️ `floor(x + 0.5)`, mirroring `sun_drift` in
                // `page_table.wgsl` exactly. `f32::round` would disagree
                // with WGSL's on halves, and disagreeing by one step is
                // a stamp that says "still valid" over depth measured
                // from somewhere else.
                (along / width + 0.5).floor().to_bits(),
                f.x.to_bits(),
                f.y.to_bits(),
                f.z.to_bits(),
                scene_gen,
            ] {
                h = fnv(h, word);
            }
            // `| 1`: a stamp of zero means "no content" and must never
            // match a generation.
            h | 1
        })
        .collect()
}

impl PageRasterizer {
    /// Threads the compaction needs: one per entry of one view's span.
    fn compact_threads(&self, light_count: u32) -> u32 {
        let slots = super::mark::padded_lights(light_count) + 1;
        u32::try_from(super::mark::span(self.config, self.clipmap, slots)).unwrap_or(u32::MAX)
    }

    /// One generation per bucket owner, for the cache gate. A sun
    /// level's changes when its snapped centre or the sun's direction
    /// changes, and when the eye crosses one of THIS level's pages along
    /// the sun's axis — all three on the same grid, so a level's content
    /// lives exactly as long as its addressing does.
    ///
    /// 🔴 The along-sun term used to be `eye.dot(f)` raw, and it had to
    /// be: the depth a page stored was measured from the unsnapped
    /// camera. So every level's stamp turned over on any camera movement
    /// at all and the cache never once hit. See `sun_drift` in
    /// `page_table.wgsl`.
    ///
    /// A lamp's changes with anything that moves its shadow: position,
    /// direction, range, kind, cone.
    fn write_gens(&self, queue: &wgpu::Queue, view: u32, eye: Vec3, sun: Vec3, lamps: &[GpuLight]) {
        let gens = self.gens_for(eye, sun, lamps);
        queue.write_buffer(
            &self.gens,
            view as u64 * self.buckets() as u64 * 4,
            bytemuck::cast_slice(&gens),
        );
    }

    /// [`Self::write_gens`] without the upload — the arithmetic alone,
    /// so a test can ask what a camera move does to the cache without a
    /// queue to write into.
    pub(crate) fn gens_for(&self, eye: Vec3, sun: Vec3, lamps: &[GpuLight]) -> Vec<u32> {
        let levels = self.clipmap.levels as usize;
        let buckets = self.buckets() as usize;
        let mut gens = vec![0u32; buckets];
        let side = self.config.side(0) as f32;
        gens[..levels].copy_from_slice(&sun_gens(self.clipmap, side, self.scene_gen, eye, sun));
        for slot in 0..LAMP_CULLS as usize {
            let mut h = FNV_SEED;
            if let Some(lamp) = lamps.get(slot) {
                for word in [
                    lamp.position[0].to_bits(),
                    lamp.position[1].to_bits(),
                    lamp.position[2].to_bits(),
                    lamp.direction[0].to_bits(),
                    lamp.direction[1].to_bits(),
                    lamp.direction[2].to_bits(),
                    lamp.range.to_bits(),
                    lamp.kind,
                    lamp.spot_scale.to_bits(),
                    lamp.spot_offset.to_bits(),
                ] {
                    h = fnv(h, word);
                }
            }
            h = fnv(h, self.scene_gen);
            gens[levels + slot] = h | 1;
        }
        gens
    }

    /// Uploads the frame's moved-caster spheres — once, not per view —
    /// or, past the buffer, bumps the scene generation so everything
    /// redraws instead of something staying silently stale.
    fn write_moved(&mut self, queue: &wgpu::Queue, moved: &[[f32; 4]]) {
        if self.moved_frame == Some(self.frame) {
            return;
        }
        self.moved_frame = Some(self.frame);
        if moved.len() > MOVED_CAPACITY as usize {
            // 🔴 Said out loud, because the fallback is silent and
            // total: past the cap the scene generation bumps, which
            // voids EVERY page every frame it happens. The panel then
            // reports a pool at 100 % hit — the slots are reused — over
            // a raster redrawing all of them, and the two readings
            // together look like a working cache. A scene that trips
            // this permanently has no page cache at all.
            if !self.flooded {
                self.flooded = true;
                tracing::warn!(
                    target: "kooch_render::shadow",
                    moved = moved.len(),
                    capacity = MOVED_CAPACITY,
                    "the moved-caster list overflowed; every page redraws while it does",
                );
            }
            self.scene_gen = self.scene_gen.wrapping_add(1);
            queue.write_buffer(&self.moved, 0, bytemuck::bytes_of(&[0.0f32; 4]));
            return;
        }
        if self.flooded {
            self.flooded = false;
            tracing::info!(
                target: "kooch_render::shadow",
                moved = moved.len(),
                "the moved-caster list fits again; the page cache is live",
            );
        }
        let mut data = Vec::with_capacity(1 + moved.len());
        data.push([moved.len() as f32, 0.0, 0.0, 0.0]);
        data.extend_from_slice(moved);
        queue.write_buffer(&self.moved, 0, bytemuck::cast_slice(&data));
    }

    /// The uniform every raster pass reads. Written once a frame,
    /// before any of them.
    /// 🔴 One per LAYER of the view, not one per view (#1016). A depth
    /// pass attaches a single layer and its draws test their page
    /// against `layer.x`, so each layer needs its own slice with its
    /// own number in it. The buffer was already sized by
    /// `atlas_layers`, so there has always been room.
    fn write_uniform(&self, queue: &wgpu::Queue, view: u32, eye: Vec3, sun: Vec3, lights: u32) {
        for local in 0..self.pool.layers_per_view() {
            self.write_layer_uniform(queue, view, local, eye, sun, lights);
        }
    }

    fn write_layer_uniform(
        &self,
        queue: &wgpu::Queue,
        view: u32,
        local: u32,
        eye: Vec3,
        sun: Vec3,
        lights: u32,
    ) {
        let d = sun.normalize_or(Vec3::NEG_Y);
        // The sun's region starts after the PADDED light slots, the way
        // marking lays the space out — see `padded_lights` for why the
        // padding and not the raw count.
        let sun_slot = super::mark::padded_lights(lights);
        let stride = super::mark::stride(self.config, self.clipmap);
        let view_span = super::mark::span(self.config, self.clipmap, sun_slot + 1);
        let layer = self.layer_of(view, local);
        queue.write_buffer(
            &self.uniform,
            self.layer_span(layer).0,
            bytemuck::bytes_of(&RasterUniform {
                space: [
                    stride,
                    self.config.local_face_pages(),
                    self.config.side(0),
                    sun_slot,
                ],
                views: [
                    view.min(atlas_layers(self.pool) - 1),
                    u32::try_from(view_span).unwrap_or(u32::MAX),
                    self.pool.slice(),
                    // 🔴 Only the age debug view reads this. A page's age
                    // is a difference against the current frame, and the
                    // shading pass has no other way to know what frame
                    // it is in.
                    self.frame,
                ],
                pool: [
                    u32::try_from(view_span * atlas_layers(self.pool) as u64).unwrap_or(u32::MAX),
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
                    // The readers' PCF footprint width (#941). In the
                    // raster's own uniform because the shading binds
                    // this exact buffer — one write serves both.
                    self.softness.max(1) as f32,
                ],
                eye: [eye.x, eye.y, eye.z, 0.0],
                sun: [d.x, d.y, d.z, 1.0],
                // Same reason as the softness above: the shading binds
                // this exact buffer, so one write serves both.
                bias: self.bias,
                layer: [layer, view, u32::from(self.march), u32::from(self.geometry)],
            }),
        );
    }

    /// The atlas layer a view's `local`-th layer is, globally.
    ///
    /// Slots are global and a view's are contiguous, so its layers are
    /// contiguous too — which is what lets `slot / slice` name a layer
    /// without being told whose it is.
    pub fn layer_of(&self, view: u32, local: u32) -> u32 {
        let per_view = self.pool.layers_per_view();
        (view.min(self.pool.view_count() - 1) * per_view + local.min(per_view - 1))
            .min(atlas_layers(self.pool) - 1)
    }

    /// The uniform slice a LAYER reads, for the pass attached to it.
    pub fn layer_span(&self, layer: u32) -> (u64, u64) {
        (
            self.uniform_stride * layer.min(atlas_layers(self.pool) - 1) as u64,
            std::mem::size_of::<RasterUniform>() as u64,
        )
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
        lamps: &[GpuLight],
    ) {
        self.write_uniform(queue, view, eye, sun, lamps.len() as u32);
        self.write_gens(
            queue,
            view.min(atlas_layers(self.pool) - 1),
            eye,
            sun,
            lamps,
        );
        encoder.clear_buffer(&self.counts, 0, None);
        encoder.clear_buffer(&self.expand_args, 0, None);
        encoder.clear_buffer(&self.draw_args, 0, None);
        encoder.clear_buffer(&self.dirty, 0, Some(4));
        let bind_group = self.compact_bind_group(device, page_pool);
        let offset = [self.uniform_span(view).0 as u32];
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("shadow pages: compact"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.compact);
        pass.set_bind_group(0, &bind_group, &offset);
        pass.dispatch_workgroups(self.compact_threads(lamps.len() as u32).div_ceil(64), 1, 1);
        pass.set_pipeline(&self.expand_args_pass);
        pass.dispatch_workgroups(self.buckets().div_ceil(64), 1, 1);
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
                entry(2, page_pool.slots()),
                entry(3, &self.page_list),
                entry(4, &self.counts),
                entry(5, &self.expand_args),
                entry(6, &self.visible_counts),
                entry(7, &self.draw_args),
                entry(8, &self.gens),
                entry(9, &self.dirty),
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
        lights: &wgpu::Buffer,
    ) {
        let keys = BoundKeys {
            slots: page_pool.slots().clone(),
            instances: instances.clone(),
            descriptors: descriptors.clone(),
            lights: lights.clone(),
            visible: self
                .culls
                .iter()
                .map(|c| c.visible_meshlets_buffer().clone())
                .collect(),
            lamp_survivors: self.lamp_cull.survivors().clone(),
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
                    entry(6, lights),
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(self.pyramid.view()),
                    },
                ],
            }),
            depth: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("page_depth_bg"),
                layout: &self.depth_bgl,
                entries: &[
                    self.uniform_entry(0),
                    entry(1, lights),
                    entry(2, &self.pairs),
                ],
            }),
            invalidate: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("page_invalidate_bg"),
                layout: &self.invalidate_bgl,
                entries: &[
                    self.uniform_entry(0),
                    entry(2, page_pool.slots()),
                    entry(10, &self.moved),
                    entry(11, lights),
                ],
            }),
            clear: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("page_clear_bg"),
                layout: &self.clear_bgl,
                entries: &[self.uniform_entry(0), entry(3, &self.dirty)],
            }),
            visible: self
                .culls
                .iter()
                .map(|c| storage("page_expand_visible_bg", c.visible_meshlets_buffer()))
                .collect(),
            lamp_visible: storage("page_expand_lamp_bg", self.lamp_cull.survivors()),
            instances: storage("page_raster_instances_bg", instances),
            descriptors: storage("page_raster_descriptors_bg", descriptors),
            keys,
        });
    }

    /// The `(page, slot, meshlet)` pairs the expansion emitted.
    ///
    /// 🔴 For the one test that can hold the inversion honest: the
    /// paired shape and the geometry-first one must produce the same
    /// SET — the order is whatever the atomics handed out, and comparing
    /// it would be testing the scheduler.
    pub fn pairs_buffer(&self) -> &wgpu::Buffer {
        &self.pairs
    }

    /// The compacted pages, for whoever reads them back.
    /// COPY_SRC so a test can read it.
    pub fn page_list_buffer(&self) -> &wgpu::Buffer {
        &self.page_list
    }

    /// Grows every clipmap level's cull to the scene. The lamps'
    /// shared arena sizes itself at record time, when the frame's
    /// active light count is known.
    pub fn ensure_capacity(
        &mut self,
        device: &wgpu::Device,
        meshlets: u32,
        groups: u32,
        chunks: u32,
    ) {
        for cull in &mut self.culls {
            // 🔴 FIRST, and that ordering is the whole of it: nothing
            // reads these culls' reject buffer — the debug overlay is
            // wired to the camera's — and `ensure_capacity` decides its
            // size from this flag. Set afterwards, the allocation had
            // already happened at full size and never shrank, because
            // `ensure_capacity` returns early once capacity fits.
            cull.set_rejects(false);
            cull.ensure_capacity(device, meshlets.max(1));
            cull.ensure_group_capacity(device, groups.max(1));
            cull.ensure_chunk_capacity(device, chunks.max(1));
        }
    }

    /// Chooses the cull's dispatch shape for every clipmap level (#1002).
    ///
    /// 🔴 The camera got the two-level cull and this path did not, which
    /// is where the cost actually is: the clipmap runs SEVENTEEN culls a
    /// frame, each one a full `instances x heaviest mesh` rectangle.
    /// Measured at 7.4 ms of a 10.9 ms GPU frame on `dense.scene` —
    /// 68 % of it — and toggling the setting moved it by 0.02 ms,
    /// because the setting did not reach here at all.
    ///
    /// Drop-in for what it draws: `min_screen_pixels` is 0 on this path,
    /// so `cs_cull_instances` runs the frustum test and nothing else.
    /// Rejecting a caster for being small ON THE CAMERA is how a shadow
    /// loses the object throwing it, and that test stays off here.
    pub fn set_two_level(&mut self, two_level: bool) {
        self.two_level = two_level;
    }

    /// The clipmap level's orthographic clip-from-world.
    fn level_clip(&self, level: u32, eye: Vec3, sun: Vec3) -> Mat4 {
        level_clip(self.clipmap, self.config.side(0), level, eye, sun)
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
        // The lights as uploaded, CPU-side and in buffer order: a
        // lamp's cull needs its position and range HERE, and its slot
        // is its bucket.
        lamps: &[GpuLight],
        // The same lights on the GPU, for the expansion's cone test
        // and the depth pass's face placement.
        lights: &wgpu::Buffer,
        // World spheres of every caster that moved this frame — old
        // and new bounds alike — for the cache's invalidation pass.
        moved: &[[f32; 4]],
        lod_target: f32,
        track: RasterTrack<'_>,
    ) {
        let levels = self.clipmap.levels;
        let buckets = self.buckets();
        let light_count = lamps.len() as u32;
        let view = view.min(atlas_layers(self.pool) - 1);
        self.write_moved(queue, moved);
        self.write_uniform(queue, view, eye, sun, light_count);
        self.write_gens(queue, view, eye, sun, lamps);
        let uniform_offset = self.uniform_span(view).0 as u32;

        // 🔴 Cleared BEFORE anything writes it: a bucket whose cull does
        // not run this frame — a directional slot, a lamp past the cap —
        // must read zero survivors, and an unwritten storage buffer is
        // not zero, it is whatever the allocator handed over.
        //
        // 🔴 THE SUN'S SPAN ONLY, and the restriction is the whole point.
        // This runs once per VIEW; the lamps' cull runs once per FRAME,
        // guarded by `lamp_frame`. Clearing the whole buffer here meant
        // the second camera wiped the lamp survivor counts and then
        // skipped the cull that refills them, so every lamp bucket read
        // zero for that view — and a page whose bucket has no survivors
        // is stamped `PAGE_EMPTY` and CLEARED. A cleared page is far
        // depth under reversed-Z, so every reader over it answers that
        // nothing occludes.
        //
        // In the editor that is two viewports over one world: the sun
        // kept its shadows, because its culls are per view and rerun
        // after the clear, and every lamp in the scene silently stopped
        // casting. The `Lamp shadow pages` views said it exactly — no
        // white in `faces`, so every page was resident, and uniform
        // green in `occlusion`, so every page was empty.
        //
        // `LampCull::record` already clears its own span, and its
        // comment already said this one "covers the sun's span only".
        // It did not.
        encoder.clear_buffer(&self.visible_counts, 0, Some(levels as u64 * 4));
        let cull_query = nested(track, "page lamp cull", encoder);
        // 1b. The lamps' shared hierarchical cull (#939) — Olsson et
        //     al.'s light/instance pre-pass, then one group-coherent
        //     meshlet pass for every lamp at once. View-independent,
        //     so the frame's second camera reuses the first one's
        //     survivors instead of re-running four dispatches.
        //
        // 🔴 The sun's survivor lists are NOT a substitute. They are
        // simplified for orthographic boxes centred on the CAMERA:
        // borrowing them culled a close lamp's casters away entirely
        // and handed far buckets root meshlets — sphere shadows drawn
        // as faceted lumps. Measured in `roll-a-ball`, both ways.
        let lamp_slots = lamps.len().min(LAMP_CULLS as usize);
        if self.lamp_frame != Some(self.frame) {
            self.lamp_frame = Some(self.frame);
            profiling::scope!("cull: lamps");
            self.lamp_cull.record(
                device,
                queue,
                encoder,
                mesh_pool,
                instances,
                lights,
                &self.visible_counts,
                levels,
                lamp_slots as u32,
                scene_params.instance_count,
                scene_params.meshlets_per_mesh,
                scene_params.group_capacity,
                lod_target,
            );
        }
        close(track, cull_query, encoder);
        // The bucket uniforms for the expansion's lamp dispatches —
        // constant values, cheap to restate per view.
        for (slot, lamp) in lamps.iter().enumerate().take(lamp_slots) {
            if lamp.kind == LIGHT_KIND_DIRECTIONAL {
                continue;
            }
            let bucket = levels + slot as u32;
            queue.write_buffer(
                &self.levels,
                bucket as u64 * self.level_stride,
                bytemuck::bytes_of(&ExpandLevel {
                    level: bucket,
                    _pad: [0; 3],
                }),
            );
        }

        // Per view, all of it: the page list, the pair list and the
        // dispatch arguments describe THIS camera's clipmap and nothing
        // else. The table, the pool and the atlas are the shared things,
        // and none of them is cleared here.
        encoder.clear_buffer(&self.counts, 0, None);
        encoder.clear_buffer(&self.expand_args, 0, None);
        encoder.clear_buffer(&self.draw_args, 0, None);
        encoder.clear_buffer(&self.dirty, 0, Some(4));

        self.ensure_bound(device, page_pool, instances, &mesh_pool.meshlets, lights);
        let bound = self.bound.as_ref().expect("just built");

        let pages_query = nested(track, "page table", encoder);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("shadow pages: invalidate and compact"),
                timestamp_writes: None,
            });
            // 1c. Invalidation, BEFORE the compaction reads the stamps:
            //     every page a moved caster reaches loses its content
            //     stamp and redraws like a fresh one.
            pass.set_pipeline(&self.invalidate);
            pass.set_bind_group(0, &bound.invalidate, &[uniform_offset]);
            pass.dispatch_workgroups(self.compact_threads(light_count).div_ceil(64), 1, 1);
            // 2. The flat table becomes a dense list, bucketed by
            //    octave — the dispatch covers exactly this camera's
            //    span, so the other view's pages are never walked.
            pass.set_pipeline(&self.compact);
            pass.set_bind_group(0, &bound.compact, &[uniform_offset]);
            pass.dispatch_workgroups(self.compact_threads(light_count).div_ceil(64), 1, 1);
            // 2b. The reader's jump table, after the compaction because
            //     "readable" means stamped and the stamp is what the
            //     compaction writes.
            let clipmap = levels * self.config.side(0).pow(2);
            pass.set_pipeline(&self.lod_offsets);
            pass.dispatch_workgroups(clipmap.div_ceil(64), 1, 1);
        }

        // 2b. The page pyramid, over the listing the compaction just
        //     wrote (#1022). Its own passes, and that is why the
        //     compute pass above ENDS here: the seed reads the third
        //     table word, which `cs_compact` is what writes, and the
        //     expansion below samples the texture this builds.
        //
        //     Built whether or not the inverted expansion is on. The
        //     texture is in the layout either way, and a pass that may
        //     sample it must not find the frame before's answer in it.
        {
            let sun_slot = super::mark::padded_lights(light_count);
            let stride = super::mark::stride(self.config, self.clipmap);
            let span = super::mark::span(self.config, self.clipmap, sun_slot + 1);
            let base = u32::try_from(view as u64 * span).unwrap_or(u32::MAX) + sun_slot * stride;
            self.pyramid
                .build(device, queue, encoder, page_pool.slots(), base);
        }

        close(track, pages_query, encoder);

        // 3. The culls, AFTER the page table is final.
        //
        // 🔴 The order is the whole point, and it used to be the other
        // way round. Unreal build their per-page draw commands from a
        // loop over INSTANCES that asks whether the pages an instance
        // covers are resident — a question that only exists once the
        // table has been built, which is why their page management runs
        // first. Culling before the table is built is what forces a
        // spatial gate that knows nothing about pages: a box, decided
        // apart from the marking, free to disagree with it.
        //
        // Nothing reads the pyramid here YET. The passes are in the
        // order that lets them, which is the half that could not be
        // added later without moving everything.
        //
        // A level is a texel density and a density is a LOD, so this is
        // where the LOD cut lives — the half Unreal also run per view.
        //
        // ⚠️ `visible_counts` is NOT cleared here. The lamps' cull wrote
        // its buckets before the compaction, which reads them for the
        // empty-page gate; a clear at this point would wipe them after
        // that read and hand the expansion zero lamp survivors.
        //
        // 🔴 Seventeen-plus full cull dispatches per view per frame,
        // each writing a uniform and recording its own passes. This is
        // the only part of the track that is CPU work rather than GPU
        // work, and it was invisible to the profiler until this scope
        // existed.
        let cull_query = nested(track, "page cull", encoder);
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
                if self.two_level {
                    self.culls[level as usize].dispatch_scene_pool_atomic_chunked(
                        cull_pipelines,
                        device,
                        queue,
                        encoder,
                        mesh_pool,
                        scene,
                        &params,
                        scene_params,
                    );
                } else {
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
                }
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
        close(track, cull_query, encoder);

        let expand_query = nested(track, "page expand", encoder);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("shadow pages: expand"),
                timestamp_writes: None,
            });
            // 4a. The dispatch sizes, HERE and not with the
            //     compaction: they are a page count times a survivor
            //     count, and the survivors only exist once the culls
            //     above have run. Sized on the GPU because neither
            //     number ever reaches the CPU.
            pass.set_pipeline(&self.expand_args_pass);
            pass.set_bind_group(0, &bound.compact, &[uniform_offset]);
            pass.dispatch_workgroups(buckets.div_ceil(64), 1, 1);

            // 4b. Pairs. One indirect dispatch per level, sized by the
            //     dispatch above rather than by a CPU guess. The only
            //     thing that changes between levels is two dynamic
            //     offsets and the visible list — no bind group is built
            //     here.
            pass.set_pipeline(&self.expand);
            pass.set_bind_group(1, &bound.descriptors, &[]);
            pass.set_bind_group(3, &bound.instances, &[]);
            // The sun's buckets against its level culls, then each
            // lamp's bucket against ITS OWN cull. `bound.visible` holds
            // them in the same order — levels first, lamp slots after —
            // so the bucket index is the bind-group index throughout.
            for level in 0..levels {
                pass.set_bind_group(
                    0,
                    &bound.expand,
                    &[uniform_offset, level * self.level_stride as u32],
                );
                pass.set_bind_group(2, &bound.visible[level as usize], &[]);
                pass.dispatch_workgroups_indirect(&self.expand_args, level as u64 * 12);
            }
            // The lamps: ONE bind group — the shared survivor arena —
            // and a slot's slice is arithmetic inside the shader, so
            // the only thing that changes per bucket is the dynamic
            // offset.
            pass.set_bind_group(2, &bound.lamp_visible, &[]);
            for (slot, lamp) in lamps.iter().enumerate().take(lamp_slots) {
                if lamp.kind == LIGHT_KIND_DIRECTIONAL {
                    continue;
                }
                let bucket = levels + slot as u32;
                pass.set_bind_group(
                    0,
                    &bound.expand,
                    &[uniform_offset, bucket * self.level_stride as u32],
                );
                pass.dispatch_workgroups_indirect(&self.expand_args, bucket as u64 * 12);
            }

            // 4. One draw for the whole clipmap, so its instance count
            //    is the whole pair list.
            pass.set_pipeline(&self.draw_args_pass);
            pass.set_bind_group(0, &bound.compact, &[uniform_offset]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        close(track, expand_query, encoder);

        let depth_query = nested(track, "page depth", encoder);
        // 🔴 One pass per LAYER of this view (#1016). A render pass
        // attaches a single layer, so a view spread across several
        // needs one each — and the draws inside test their page against
        // the layer they are in, because a page's rect is the same
        // texels of every layer. One pass while the pool fits a layer,
        // which is what the editor's two views still do.
        for local in 0..self.pool.layers_per_view() {
            let layer = self.layer_of(view, local);
            let layer_offset = self.layer_span(layer).0 as u32;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow pages: depth"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    // 🔴 THIS camera's layer, LOADED — never cleared.
                    // The cache is the layer's content: a resident page
                    // whose stamp still matches keeps last frame's
                    // depth, and only the dirty pages' rects are wiped,
                    // by the quad draw below. The array still keeps one
                    // camera out of the other's pages.
                    view: &self.layers[layer as usize],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // The per-page clear: one quad per dirty page at far depth
            // (reversed-Z 0 — "nothing between here and the light"),
            // depth test Always. Then the pairs draw over clean rects.
            pass.set_pipeline(&self.page_clear);
            pass.set_bind_group(0, &bound.clear, &[layer_offset]);
            pass.draw_indirect(&self.draw_args, 16);
            pass.set_pipeline(&self.depth);
            pass.set_bind_group(0, &bound.depth, &[layer_offset]);
            pass.set_bind_group(1, meshlet_bg, &[]);
            pass.set_bind_group(2, &bound.instances, &[]);
            pass.draw_indirect(&self.draw_args, 0);
        }
        close(track, depth_query, encoder);

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
        let counts = self.decode(&words, view);
        // A pair overflow dropped geometry from pages the compaction
        // had already stamped — a hole the cache would keep. One
        // generation bump redraws everything once. 🔴 Reached only
        // when something polls (the editor's panel does); a shipped
        // build that never polls carries the hazard, noted in #477.
        if counts.overflow > 0 {
            self.scene_gen = self.scene_gen.wrapping_add(1);
        }
        Some(counts)
    }
}

pub(super) fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

pub(super) fn buffer_entry(
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

pub(super) fn uniform_entry(
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
            // Binding 1 held the hash table's keys and is retired: the
            // flat table's entry index IS the page id.
            // 🔴 Writable now: the compaction records each page's place
            // in `page_list` back into its table entry, which is the
            // only pass that knows both. See `PAGE_CELL`.
            buffer_entry(2, false, c),
            buffer_entry(3, false, c),
            buffer_entry(4, false, c),
            buffer_entry(5, false, c),
            buffer_entry(6, true, c),
            buffer_entry(7, false, c),
            // The generations the cache gate compares stamps against,
            // and the dirty list the per-page clear draws from — the
            // eighth storage buffer, which is the whole downlevel
            // budget again.
            buffer_entry(8, true, c),
            buffer_entry(9, false, c),
        ],
    })
}

/// `cs_invalidate`'s own layout: the table, the moved spheres and the
/// lights — none of which the other compact entries touch.
fn invalidate_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let c = wgpu::ShaderStages::COMPUTE;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("page_invalidate_bgl"),
        entries: &[
            uniform_entry(0, true, c),
            buffer_entry(2, false, c),
            buffer_entry(10, true, c),
            buffer_entry(11, true, c),
        ],
    })
}

/// The per-page clear's layout: the uniform for the atlas arithmetic
/// and the dirty list naming the slots to wipe.
fn clear_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let v = wgpu::ShaderStages::VERTEX;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("page_clear_bgl"),
        entries: &[uniform_entry(0, true, v), buffer_entry(3, true, v)],
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
            // 🔴 Here rather than in a group of its own: `max_bind_groups`
            // is FOUR and this pass already binds four. It is also the
            // eighth storage buffer of the stage, which is the entire
            // downlevel budget.
            buffer_entry(6, true, c),
            // Which is why the pyramid is a TEXTURE. There is no ninth
            // storage buffer to give it, and textures are a separate
            // budget — the same constraint that decided Unreal's shape.
            wgpu::BindGroupLayoutEntry {
                binding: 7,
                visibility: c,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
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
mod tests;
