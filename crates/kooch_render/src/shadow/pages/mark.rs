//! The marking pass, on the GPU (#866).

use std::sync::{Arc, Mutex};

use glam::{Mat4, Vec3};

use kooch_lighting::{CLUSTER_COMMON, GpuLights, PAGE_TABLE};

use super::pool::{PagePool, PoolConfig, PoolCounts, PoolLife};
use super::{ClipmapConfig, PageConfig};

/// Split across files in declaration order; WGSL has no `#include`.
const SOURCE: &str = concat!(
    include_str!("../../../shaders/page_mark.wgsl"),
    include_str!("../../../shaders/page_mark_views.wgsl"),
    include_str!("../../../shaders/page_mark_froxels.wgsl"),
);
const GROUP: u32 = 8;
/// 0 resident, 1 samples, 2 pairs, 3 mark overflow, 4 unused, 5 pool overflow, 6 unused (was the
/// hash's probe overflow), 7 reuses, 8 fresh claims, 9 unused (was holes walked), 10 free-list
/// overflow, 11 pages kept alive, 12 pages evicted, 13 unused (was tombstones swept).
const COUNTERS: u64 = 25;
/// Words per view in the rank-state buffer: a 32-bucket demand histogram, the plan's three words,
/// then the persistent bias and patience (#943), padded to 40 — then the OCCUPANCY BITMAP, one bit
/// per froxel. Mirrors `RANK_WORDS` in the shader.
const RANK_WORDS: u64 = 8360;
/// First word of the occupancy bitmap within a view's run.
const RANK_OCCUPANCY: u64 = 40;
/// Words of bitmap: 4096 froxels, the grid's own cap.
const OCCUPANCY_WORDS: u64 = 128;
/// Froxels the bitmap covers; mirrors `OCCUPANCY_MAX` in the shader.
const OCCUPANCY_MAX: u32 = 4096;
/// Two words a froxel.
const DEPTH_WORDS: u64 = 8192;

/// `KOOCH_PAGE_MARKING=1`, read once.
pub fn enabled_by_environment() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("KOOCH_PAGE_MARKING")
            .is_ok_and(|v| v != "0" && !v.eq_ignore_ascii_case("off"))
    })
}

/// Olsson §III's cluster/light marking, ON, and `KOOCH_CLUSTER_MARKING=0` turns it off. Read once.
pub fn cluster_marking() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("KOOCH_CLUSTER_MARKING")
            .ok()
            .is_none_or(|v| v != "0" && !v.eq_ignore_ascii_case("off"))
    })
}

impl PageMarker {
    /// Overrides [`cluster_marking`] for this marker.
    pub fn set_cluster_marking(&mut self, on: bool) {
        self.cluster = on;
    }
}

/// What `record` clamps the sampling rate to.
pub const RATE_RANGE: (u32, u32) = (1, 16);

/// What the debug view paints into.
pub const PAINT_FORMAT: wgpu::TextureFormat = crate::meshlet::deferred::DEFERRED_COLOR_FORMAT;

/// Where the debug view writes, and what it has to survive.
#[derive(Clone, Copy)]
pub struct Paint<'a> {
    /// The frame's HDR radiance. Bound whether or not the view is on: a binding declared in the
    /// shader has to be provided, and a second pipeline for the sake of one branch is a second
    /// pipeline to keep in step.
    pub target: &'a wgpu::TextureView,
    pub on: bool,
    /// The target's size, which is the view's OUTPUT size and not the depth buffer's.
    pub size: (u32, u32),
}

/// What one dispatch found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MarkCounts {
    /// Distinct pages, which is the number the census predicts.
    pub resident: u32,
    /// Samples that landed on a surface rather than on sky.
    pub samples: u32,
    /// Sample/light pairs walked.
    pub pairs: u32,
    /// Pairs the distance gate turned away (#944): the light reaches
    /// the sample, but it stands more than `shadow_page_light_reach`
    /// of its own ranges from the camera, so it marks nothing.
    pub culled: u32,
    /// Pairs served by the DISTANT tier (#1009): the light's whole range projects under
    /// `shadow_min_pixels`, so it marked ONE page of its coarsest level rather than a chain.
    pub distant: u32,
    /// The most lights any one occupied froxel had to walk.
    pub peak_lights: u32,
    /// Whether `pairs` counts (froxel, light) or (pixel, light).
    pub by_froxel: bool,
    /// Froxels of this view that held visible surface.
    pub froxels: u32,
    /// Page indices past the end of the mark buffer. 🔴 Non-zero means
    /// every number above is a floor, not a count.
    pub overflow: u32,
    /// What the allocator did with them.
    pub pool: PoolCounts,
    /// The render size the count was taken at.
    pub size: (u32, u32),
    /// Which camera produced it, for the same reason as `size`.
    pub view: u32,
}

/// Mirrors `PageView` in `page_mark.wgsl`, field for field.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PageMarkView {
    world_from_clip: [[f32; 4]; 4],
    eye_and_base: [f32; 4],
    sun: [f32; 4],
    chain: [u32; 4],
    strides: [u32; 4],
    sampling: [u32; 4],
    pool: [u32; 4],
    paint: [f32; 4],
    life: [u32; 4],
    density: [f32; 4],
    /// x how far, in PAGES, a receiver dilates its request. Mirrors
    /// `halo` in `page_mark.wgsl`.
    halo: [f32; 4],
}

/// The pass, its buffers, and the ring that brings the count home.
pub struct PageMarker {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    /// Ages this view's table entries and evicts what went unrequested,
    /// and only this view's. See `age_view` in the shader for why that
    /// cannot be a `clear_buffer`.
    clear: wgpu::ComputePipeline,
    /// Paints the debug view, in a dispatch of its own that runs AFTER
    /// the shading. See `paint_view` in the shader for why it cannot go
    /// with the marking any more.
    paint: wgpu::ComputePipeline,
    /// The three seat passes (#942): rank the frame's demand, clear the
    /// seats the plan does not fund, then seat what it does. Allocation
    /// lives HERE now, not in the marking — first-come is not an order.
    plan: wgpu::ComputePipeline,
    preempt: wgpu::ComputePipeline,
    adopt: wgpu::ComputePipeline,
    /// The resolution feedback (#943): one step per frame toward the
    /// coarsest marking that fits the slice.
    bias: wgpu::ComputePipeline,
    /// Populates the occupancy bitmap into `counters[9]`.
    census: wgpu::ComputePipeline,
    /// Olsson §III's cluster/light marking, behind `KOOCH_CLUSTER_MARKING`.
    froxel_mark: wgpu::ComputePipeline,
    /// Whether to run it.
    cluster: bool,
    /// The bind group the marking built, kept so the paint dispatch can
    /// reuse it without rebuilding every resource binding.
    bound: Option<wgpu::BindGroup>,
    view: wgpu::Buffer,
    marks: wgpu::Buffer,
    counters: wgpu::Buffer,
    /// The seating plan (#942): per-view demand histogram by rank plus
    /// the cutoff the plan chose. Cleared per view per frame.
    rank: wgpu::Buffer,
    /// The physical pool and its table, written by the same dispatch
    /// that marks. See [`pool`](super::pool) for why the allocation
    /// happens here and not in a pass of its own.
    pool: PagePool,
    readback: Readback,
    /// A slot holding a copy that has been recorded but not yet mapped.
    pending: Option<usize>,
    config: PageConfig,
    clipmap: ClipmapConfig,
    /// Slots and views the mark buffer is sized for.
    capacity: (u32, u32),
    /// The frame index and the eviction threshold. See [`PoolLife`].
    life: PoolLife,
    /// The coverage gate (#944), in projected screen pixels. 0 = off,
    /// which is what a directly-constructed marker measures with.
    coverage: u32,
    /// The distance gate, in multiples of a light's own range. 0 = off.
    reach: u32,
    /// How far, in PAGES, a receiver dilates its request (#1022).
    /// 0 = off, which is what a directly-constructed marker measures
    /// with. Epic's `PageDilationOffset`.
    halo: f32,
    last: Option<MarkCounts>,
}

impl PageMarker {
    pub fn new(device: &wgpu::Device, config: PageConfig, clipmap: ClipmapConfig) -> Self {
        let layout = layout(device);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("page_mark"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{CLUSTER_COMMON}\n{PAGE_TABLE}\n{SOURCE}").into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("page_mark_pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let compute = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let pipeline = compute("mark_main");
        let clear = compute("age_view");
        let paint = compute("paint_view");
        let plan = compute("plan_view");
        let preempt = compute("preempt_view");
        let adopt = compute("adopt_view");
        let bias = compute("bias_view");
        let census = compute("count_froxels");
        let froxel_mark = compute("mark_froxels");

        Self {
            layout,
            pipeline,
            clear,
            paint,
            plan,
            preempt,
            adopt,
            bias,
            census,
            froxel_mark,
            cluster: cluster_marking(),
            bound: None,
            view: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_mark_view"),
                size: std::mem::size_of::<PageMarkView>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            marks: marks_buffer(device, config, clipmap, 1, 1),
            rank: rank_buffer(device, 1),
            pool: PagePool::new(device, PoolConfig::default()),
            counters: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_mark_counters"),
                size: COUNTERS * 4,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            readback: Readback::new(device),
            // The first frame is a rebuild: every buffer above was just
            // created and `clear_buffer` has not run over any of them.
            life: PoolLife {
                rebuilt: true,
                ..Default::default()
            },
            pending: None,
            config,
            clipmap,
            capacity: (1, 1),
            coverage: 0,
            halo: 0.0,
            reach: 0,
            last: None,
        }
    }

    /// How far, in pages, a receiver dilates its page request.
    pub fn set_halo(&mut self, pages: f32) {
        self.halo = pages.max(0.0);
    }

    /// Projected radius, in screen pixels, under which a local light is DISTANT: one page per cube
    /// face instead of a chain (#1009). The sun is never demoted.
    pub fn set_coverage(&mut self, pixels: u32) {
        self.coverage = pixels;
    }

    /// How far a light may cast from, in multiples of its own range.
    /// Zero is no limit. See `ShadowSettings::page_light_reach`.
    pub fn set_reach(&mut self, ranges: u32) {
        self.reach = ranges;
    }

    /// The last count that came back, a frame or two old.
    pub fn last(&self) -> Option<MarkCounts> {
        self.last
    }

    /// The physical pool and its table.
    pub fn pool(&self) -> &PagePool {
        &self.pool
    }

    /// Resizes the pool, and reports whether anything changed.
    pub fn set_pool(&mut self, device: &wgpu::Device, config: PoolConfig) -> bool {
        let changed = self.pool.resize(device, config);
        self.life.rebuilt |= changed;
        changed
    }

    /// Stamps the frame every page requested from here on belongs to.
    pub fn set_frame(&mut self, frame: u32) {
        // A rebuild is consumed by the frame that follows it, not by the
        // camera that follows it: both views have to evict.
        if frame != self.life.frame {
            self.life.rebuilt = false;
            self.life.frame = frame;
        }
    }

    /// This frame's residency policy.
    pub fn life(&self) -> PoolLife {
        self.life
    }

    /// Frames a page may go unrequested before it is evicted. See
    /// [`PoolLife`] for why the default is zero.
    pub fn set_max_age(&mut self, frames: u32) {
        self.life.max_age = frames;
    }

    /// Frees every page in the table, on the next frame.
    pub fn void(&mut self) {
        self.life.rebuilt = true;
    }

    /// Drops the cached count.
    pub fn forget(&mut self) {
        self.last = None;
    }

    /// Pages one light can address, which is the mark buffer's stride.
    fn stride(&self) -> u32 {
        stride(self.config, self.clipmap)
    }

    /// Paints the debug view over the frame's FINAL colour.
    pub fn record_paint(&self, encoder: &mut wgpu::CommandEncoder, viewport: (u32, u32)) {
        let Some(bound) = self.bound.as_ref() else {
            return;
        };
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("shadow pages: paint"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.paint);
        pass.set_bind_group(0, bound, &[]);
        pass.dispatch_workgroups(viewport.0.div_ceil(GROUP), viewport.1.div_ceil(GROUP), 1);
    }

    /// Maps what this frame recorded and picks up whatever earlier frames returned.
    pub fn poll(&mut self) {
        if let Some(slot) = self.pending.take() {
            self.readback.submit(slot);
        }
        if let Some(mut counts) = self.readback.take() {
            // Stamped here rather than in the readback, which does not
            // know which path recorded the frame it is decoding.
            counts.by_froxel = self.cluster;
            self.last = Some(counts);
        }
    }
}

/// Pages one LOCAL light addresses: six faces of a chain that starts at `local_floor` — the levels
/// under the floor cannot be marked, so addressing them would spend table entries on pages that
/// cannot exist.
pub(super) fn stride(config: PageConfig, _clipmap: ClipmapConfig) -> u32 {
    let local = config.local_face_pages() * super::CUBE_FACES as u32;
    local.div_ceil(32) * 32
}

/// Light slots the address space is laid out for, PADDED so that adding a light does not move every
/// page id.
pub(super) fn padded_lights(count: u32) -> u32 {
    count.max(1).next_multiple_of(64)
}

/// Pages one VIEW addresses: `slots - 1` padded light slots, then the
/// sun's clipmap — every level a full grid, at the tail.
pub(super) fn span(config: PageConfig, clipmap: ClipmapConfig, slots: u32) -> u64 {
    let lights = slots.max(2) as u64 - 1;
    let sun = clipmap.levels as u64 * (config.side(0) as u64).pow(2);
    // The whole span on a word boundary, like the stride: view N's bits
    // start at `N * span` and the bitmap is cleared per view.
    (lights * stride(config, clipmap) as u64 + sun).div_ceil(32) * 32
}

fn marks_buffer(
    device: &wgpu::Device,
    config: PageConfig,
    clipmap: ClipmapConfig,
    slots: u32,
    views: u32,
) -> wgpu::Buffer {
    let bits = span(config, clipmap, slots) * views.max(1) as u64;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("page_mark_bits"),
        size: bits.div_ceil(32).max(1) * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// One `RANK_WORDS` run per view. Persistent only within a frame.
fn rank_buffer(device: &wgpu::Device, views: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("page_rank_state"),
        size: views as u64 * RANK_WORDS * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn buffer_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let uniform = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("page_mark_layout"),
        entries: &[
            uniform(0),
            storage(1, true),
            storage(2, true),
            storage(3, true),
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            uniform(5),
            storage(6, false),
            storage(7, false),
            wgpu::BindGroupLayoutEntry {
                binding: 8,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: PAINT_FORMAT,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
            // Binding 9 held the hash table's keys, was retired with the flat table, and is spent
            // again on the seating plan (#942) — which puts this layout AT the eight-per-stage
            // storage-buffer downlevel limit.
            storage(9, false),
            storage(10, false),
            storage(11, false),
        ],
    })
}

mod readback;
mod record;

use super::raster;
use readback::{Label, Readback};

#[cfg(test)]
mod tests;
