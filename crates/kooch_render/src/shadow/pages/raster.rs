//! Rasterising depth into the pages marking asked for (#866).

use glam::{Mat4, Vec3};

use crate::meshlet::{
    CullParams, GpuGlobalMeshPool, MeshletCull, MeshletCullPipelines, MeshletScene, SceneCullParams,
};

use super::pool::{PagePool, PoolConfig};
use super::pyramid::PagePyramid;
use super::{ClipmapConfig, PageConfig};

use kooch_core::gpu::{GpuQuery, GpuScopes};

/// The caller's open scope, for the four passes below to nest under.
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

/// Lamp slots the raster addresses — lamp `L`'s pages land in bucket `clipmap.levels + L`, fed by
/// the hierarchical cull's slice for `L` (#939). A light past the cap keeps its pages listed but
/// undrawn, counted with the dropped pages. Mirrors `LAMP_CULLS` in `page_table.wgsl`.
pub const LAMP_CULLS: u32 = 256;

/// Moved-caster spheres a frame may upload for page invalidation. Past it, the scene generation
/// bumps instead — every page redraws once, which is coarse and never wrong. Moved casters the list
/// starts with. GROWN to what the frame actually moved — see [`PageRasterizer::ensure_moved`].
const MOVED_CAPACITY: u32 = 256;

/// Bytes a moved-caster list of `spheres` needs: the count header, then
/// one world sphere each.
fn moved_bytes(spheres: u32) -> u64 {
    (1 + u64::from(spheres)) * 16
}

/// The most moved casters the list will ever be grown to, at sixteen bytes each.
const MOVED_CEILING: u32 = 1 << 20;

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
pub const PAGE_FRONT_FACE: wgpu::FrontFace = wgpu::FrontFace::Cw;

/// How far a caster may be from a page along the sun's own axis, in metres, before it stops writing
/// into it.
pub const SUN_SPAN: f32 = 2000.0;

/// Pages one level may list.
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
    /// 🔴 Pages belonging to local lights, which are marked and allocated and rasterised. Reported
    /// rather than ignored: a pool that looks full for a reason nobody stated is how a budget gets
    /// mis-read, and lamps are what fills this one.
    pub local: u32,
    /// Pages listed for THIS view, the sun's and the lamps' together.
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
    pub sun_rejected: u32,
    /// Which camera this is.
    pub view: u32,
    /// Meshlets the LAMPS' culls kept this frame, over every bucket.
    pub lamp_survivors: u32,
    /// Meshlet/page tests the expansion ran, summed over the levels.
    pub tests: u64,
    /// The level that ran the most tests, and how many.
    pub worst: (u32, u64),
    /// What the OTHER shape of the expansion would have cost: cells a scatter would visit, summed
    /// over the levels.
    pub scatter: u64,
    /// Tests a per-level hybrid would run: the cheaper of the two shapes at every level, summed.
    pub hybrid: u64,
    /// Pages sitting in a bucket whose cull produced NO survivors.
    pub unfilled: u32,
    /// The lowest bucket in that state, so the reading names one.
    /// `u32::MAX` when there is none.
    pub unfilled_first: u32,
    /// How many of [`Self::unfilled`] belong to the SUN's clipmap.
    pub unfilled_sun: u32,
    /// Pages the INVERTED expansion reached, counted where they happen (#1022).
    pub walk: u64,
    /// Descents that ran out of stack.
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
    /// One generation per bucket owner per view — the sun's levels (snapped centre, direction, the
    /// eye's height along the sun's axis), then the lamps (transform, range, cone). The compaction
    /// caches a page whose stamp matches. Never zero.
    gens: wgpu::Buffer,
    /// `[0]` count, then the physical slot of every page THIS view's
    /// compaction listed — what the depth pass clears instead of the
    /// whole layer.
    dirty: wgpu::Buffer,
    /// `[0].x` count, then world spheres of every caster that moved
    /// this frame, old and new bounds alike.
    moved: wgpu::Buffer,
    /// Spheres [`Self::moved`] currently holds. Grown with the scene.
    moved_capacity: u32,
    /// Folded into every generation. Bumped when the moved list overflows its buffer — the coarse,
    /// honest fallback — and when a pair overflow was observed, because a stamped page whose pairs
    /// were dropped cached a hole.
    scene_gen: u32,
    /// The frame the moved list was last uploaded and any overflow
    /// bump applied — once per frame, not per view.
    moved_frame: Option<u32>,
    /// Whether the moved list is currently past [`MOVED_CEILING`].
    flooded: bool,
    /// The scene set this cache holds pages for.
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
    /// The same, dropping what a transparent caster's coverage does not reach (#1224).
    depth_alpha: wgpu::RenderPipeline,
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
    /// The readers' shadow bias, carried in `bias`: the normal step as a multiple of the texel, the
    /// step towards the light in metres, a ceiling on the first in metres (0 = none), and a ceiling
    /// on the receiver's own depth gradient (0 = the term is off).
    bias: [f32; 4],
    /// Whether the shading marches the atlas instead of sampling one
    /// texel through a PCF box (#1017). Carried to the shader in the
    /// raster uniform's spare word, which the shading binds anyway.
    march: bool,
    /// Whether the expansion runs from the GEOMETRY — one thread per surviving meshlet, descending
    /// the page pyramid to the pages it lands in — instead of pairing every listed page against
    /// every survivor (#1022). The sun's buckets only.
    geometry: bool,
    /// The page pyramid the inverted expansion descends. Built every frame after the compaction,
    /// whether or not anything reads it: the binding is part of the layout either way, and a
    /// texture the pass may sample has to hold this frame's answer.
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
    // 🔴 In the key because it GROWS. The lights buffer is reallocated when the scene outgrows it,
    // and a cached bind group holding the old one reads a lamp's range out of freed memory — or out
    // of a buffer that is simply somebody else's now.
    lights: wgpu::Buffer,
    visible: Vec<wgpu::Buffer>,
    /// The lamps' survivor arena — fixed-size today, in the key so a
    /// future growth path cannot silently skip the rebuild.
    lamp_survivors: wgpu::Buffer,
    /// The moved-caster spheres, which GROW with the scene. That future
    /// the field above anticipates arrived here first.
    moved: wgpu::Buffer,
}

impl PageRasterizer {
    /// The depth atlas every resident page is rasterised into: the whole
    /// array, one layer per camera.
    pub fn atlas(&self) -> &wgpu::TextureView {
        &self.atlas_view
    }

    /// Where a camera's slice of the uniform starts and how far it runs. Where this camera's slice
    /// of the uniform starts.
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
    pub fn set_scene_epoch(&mut self, epoch: u32) {
        if self.scene_epoch == Some(epoch) {
            return;
        }
        // Not on the first frame: a rasterizer that has drawn nothing
        // has nothing to void, and bumping here would throw away the
        // pages the very first scene just filled.
        if self.scene_epoch.is_some() {
            // 🔴 info, not debug. This fires once per scene load — rare, and the single line that
            // answers "did the cache get voided?" when shadows look wrong after a scene change.
            // Hidden behind debug it cost a diagnosis on the day it shipped.
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

    /// The readers' shadow bias, from the settings. Takes effect at the next `write_uniform`, the
    /// way the softness does. Which direction the expansion runs: pages against survivors, or one
    /// survivor down the pyramid to its pages.
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

    /// Buckets in `page_list`: the sun's clipmap levels first — one octave of world texel size
    /// each, anchored so level `L` lands on bucket `L` — then [`LAMP_CULLS`] buckets, one per lamp
    /// slot.
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
            // 🔴 The lamps' half of the survivor mirror, summed. `unfilled` says pages were cleared
            // for want of geometry; only this says whether the culls found any to begin with.
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
            // The choice a hybrid would make at this level, which is the only place the choice can
            // be made: the two shapes cross over somewhere in the middle of the chain and neither
            // end knows where.
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
            // 🔴 Every listed page, the sun's and the lamps' alike, because they share buckets now:
            // a lamp and the sun that want the same fineness are in the same list. `local` still
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
pub const PAIR_CAPACITY: u32 = 1 << 18;

fn count_slots(buckets: u32) -> u32 {
    // Per level, then: bucket overflow, local pages skipped, pairs, pair overflow, pages owned by
    // another view — and THEN the survivors each level's cull produced, copied in from
    // `visible_counts`.
    buckets * 3 + 9
}

impl PageRasterizer {
    /// Maps this frame's counters and picks up whatever earlier frames
    /// returned. Call **after** the encoder has been submitted.
    pub fn poll(&mut self) -> Option<RasterCounts> {
        let (words, view) = self.readback.poll()?;
        let counts = self.decode(&words, view);
        // A pair overflow dropped geometry from pages the compaction had already stamped — a hole
        // the cache would keep. One generation bump redraws everything once.
        if counts.overflow > 0 {
            self.scene_gen = self.scene_gen.wrapping_add(1);
        }
        Some(counts)
    }
}

mod alpha;

use super::{lamp_cull, mark, pyramid};
#[cfg(test)]
use geometry::sun_frame;
use geometry::{atlas_texture, level_clip, sun_gens};
pub(super) use layouts::{buffer_entry, entry, uniform_entry};
use layouts::{
    clear_layout, compact_layout, depth_layout, expand_layout, invalidate_layout, storage_layout,
};
pub use readback::{RasterReadback, SlotState};

mod geometry;
mod layouts;
mod new;
mod readback;
mod record;
mod state;

#[cfg(test)]
mod tests;
