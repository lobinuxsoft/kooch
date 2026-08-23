//! The marking pass, on the GPU (#866).
//!
//! One compute dispatch over the depth buffer. The depth says **where** a
//! surface is; the froxel grid says **which lights** reach it. Both are
//! needed, and the census in [`super`] is what established that the grid
//! alone is the wrong input — on `many_lights.scene` it claims 15 770
//! pages for the sun where the surfaces need 118.
//!
//! # It is an instrument before it is a feature
//!
//! Nothing reads what this writes yet. It counts, reports, and is
//! checked against the CPU census, because the census is a **model** and
//! this is the first thing that can falsify it. The pass is off unless
//! `KOOCH_PAGE_MARKING=1`, the way `KOOCH_CLUSTERING=off` is the grid's
//! A/B: an instrument that runs whether or not anyone asked is a cost
//! nobody attributed.
//!
//! # The mirror
//!
//! Every arithmetic decision in `page_mark.wgsl` has a twin in
//! [`super`] on the CPU. Two counts that disagree mean one of them is
//! wrong, and finding out which is the point.

use std::sync::{Arc, Mutex};

use glam::{Mat4, Vec3};

use kooch_lighting::{CLUSTER_COMMON, GpuLights, PAGE_TABLE};

use super::pool::{PagePool, PoolConfig, PoolCounts, PoolLife};
use super::{ClipmapConfig, PageConfig};

const SOURCE: &str = include_str!("../../../shaders/page_mark.wgsl");
const GROUP: u32 = 8;
/// 0 resident, 1 samples, 2 pairs, 3 mark overflow, 4 unused, 5 pool
/// overflow, 6 unused (was the hash's probe overflow), 7 reuses, 8
/// fresh claims, 9 unused (was holes walked), 10 free-list overflow,
/// 11 pages kept alive, 12 pages evicted, 13 unused (was tombstones
/// swept). The rest spare, because a storage buffer is rounded up
/// anyway.
const COUNTERS: u64 = 16;
/// Words per view in the rank-state buffer: a 32-bucket demand
/// histogram, the cutoff, the quota and the spare, padded to 36.
/// Mirrors `RANK_WORDS` in the shader.
const RANK_WORDS: u64 = 36;

/// `KOOCH_PAGE_MARKING=1`, read once.
///
/// 🔴 A FORCE on top of `RenderSettings::virtual_shadows`, not its
/// default. The comparison it exists for is made on a handheld, over
/// SSH, against a build nobody wants to make twice — the same reason
/// `KOOCH_CLUSTERING` is one.
pub fn enabled_by_environment() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("KOOCH_PAGE_MARKING")
            .is_ok_and(|v| v != "0" && !v.eq_ignore_ascii_case("off"))
    })
}

/// What `record` clamps the sampling rate to.
///
/// ⚠️ Only 1 is correct now that the marks drive a raster: a coarser
/// rate is pixels whose shadow page was never allocated. The range
/// survives for the tests that measure how the count moves with it.
pub const RATE_RANGE: (u32, u32) = (1, 16);

/// What the debug view paints into.
///
/// 🔴 The view's **final** colour target, not the HDR radiance one, and
/// that is the fix for two bugs in one. The radiance target lives inside
/// the R64 stage and this pass cannot reach it; `MeshletView::color_view`
/// is `Rgba8Unorm`, allocated at the view's OUTPUT size, and holds the
/// tonemapped image. Painting there means the debug view needs no
/// exposure divided out and survives the upscaler, because it is written
/// after both.
///
/// ⚠️ It also has to match exactly: wgpu compares the storage class
/// declared in the shader against this layout, and the mismatch surfaces
/// as a stream of *"Storage texture binding 8 expects format ..."*
/// rather than as a wrong image.
pub const PAINT_FORMAT: wgpu::TextureFormat = crate::meshlet::deferred::DEFERRED_COLOR_FORMAT;

/// Where the debug view writes, and what it has to survive.
#[derive(Clone, Copy)]
pub struct Paint<'a> {
    /// The frame's HDR radiance. Bound whether or not the view is on:
    /// a binding declared in the shader has to be provided, and a
    /// second pipeline for the sake of one branch is a second pipeline
    /// to keep in step.
    pub target: &'a wgpu::TextureView,
    pub on: bool,
    /// The target's size, which is the view's OUTPUT size and not the
    /// depth buffer's.
    ///
    /// 🔴 They differ whenever `render_scale` is below 100, and one
    /// thread per depth pixel then covers a block of output pixels. The
    /// shader fills the whole block; writing one would leave a grid of
    /// dots over an unpainted frame.
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
    /// Page indices past the end of the mark buffer. 🔴 Non-zero means
    /// every number above is a floor, not a count.
    pub overflow: u32,
    /// What the allocator did with them.
    pub pool: PoolCounts,
    /// The render size the count was taken at.
    ///
    /// 🔴 Carried with the number rather than left to the reader,
    /// because a page count without its resolution is not a reading —
    /// this project has already had to retract a table that mixed 1080p
    /// with 720p. It also explains the two figures the editor logs: the
    /// View and the Game tab are two cameras at two sizes.
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
    ///
    /// 🔴 `map_async` before the encoder is submitted is a validation
    /// error — *"buffer is still mapped"* out of `Queue::submit` — which
    /// is why `ClusterReadback` splits the copy from the map and why
    /// this does too. [`Self::poll`] is the after-submit half.
    pending: Option<usize>,
    config: PageConfig,
    clipmap: ClipmapConfig,
    /// Slots and views the mark buffer is sized for.
    capacity: (u32, u32),
    /// The frame index and the eviction threshold. See [`PoolLife`].
    life: PoolLife,
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

        Self {
            layout,
            pipeline,
            clear,
            paint,
            plan,
            preempt,
            adopt,
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
            last: None,
        }
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
    ///
    /// 🔴 The table is rebuilt, not migrated, and the NEXT frame is
    /// flagged as a rebuild so `age_view` evicts everything before
    /// anything reads it. A resize changes how a slot maps to an atlas
    /// texel — `per_row` and `slice` both move — so a carried-over entry
    /// points at a page that now belongs to someone else. Every buffer
    /// here is fresh, so this only has to say so out loud.
    pub fn set_pool(&mut self, device: &wgpu::Device, config: PoolConfig) -> bool {
        let changed = self.pool.resize(device, config);
        self.life.rebuilt |= changed;
        changed
    }

    /// Stamps the frame every page requested from here on belongs to.
    ///
    /// 🔴 Takes the count rather than counting itself, and that is the
    /// point: `record` runs once per CAMERA and a page's age has to be
    /// measured in frames. A marker that incremented on its own would
    /// age the first view's pages out from under it while the second
    /// view marked, in the same frame — silently, and only in the
    /// editor. `Time::frame_count` is already the stamp the light frame
    /// is shared on.
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

    /// Drops the cached count.
    ///
    /// 🔴 Sticky by design — the ring is a frame or two behind, so a
    /// frame with nothing new keeps reporting the last real answer. That
    /// is right while the pass runs and wrong the moment it stops: a
    /// count nobody measured this frame is not a reading.
    pub fn forget(&mut self) {
        self.last = None;
    }

    /// Pages one light can address, which is the mark buffer's stride.
    fn stride(&self) -> u32 {
        stride(self.config, self.clipmap)
    }

    /// Records the dispatch, sizing the mark buffer if the scene grew.
    ///
    /// Call **after** the pass that writes depth and after the froxel
    /// grid: this reads both.
    #[allow(clippy::too_many_arguments)]
    #[profiling::function]
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        lights: &GpuLights,
        depth: &wgpu::TextureView,
        world_from_clip: Mat4,
        eye: Vec3,
        sun: Option<Vec3>,
        viewport: (u32, u32),
        // Which camera this dispatch is for. Decides the slice of the
        // pool it allocates from, its region of the mark bitmap and the
        // high part of every page id it writes.
        view: u32,
        rate: u32,
        // Shadow texels per screen pixel, as a percentage.
        density: u32,
        paint: Paint<'_>,
    ) {
        let count = lights.light_count().max(1);
        // One slot past the lights, for the sun: it is not in the grid
        // — it has no position to cluster — so it gets a region of its
        // own at the tail rather than a light index. PADDED, so a light
        // added or removed does not move every page id in the table —
        // see `padded_lights`.
        let padded = padded_lights(count);
        let slots = padded + 1;
        let views = self.pool.config().slices();
        let view = view.min(views - 1);
        if (slots, views) != self.capacity {
            self.marks = marks_buffer(device, self.config, self.clipmap, slots, views);
            self.rank = rank_buffer(device, views);
            self.capacity = (slots, views);
        }
        // The flat table is one entry per addressable page, so its size
        // follows the address space. A growth replaces the buffers —
        // every entry gone — so the next frame is flagged as a rebuild
        // and `age_view` evicts the nothing that is left, keeping the
        // allocator honest.
        let view_span = span(self.config, self.clipmap, slots);
        let entries = u32::try_from(view_span * views as u64).unwrap_or(u32::MAX);
        if self.pool.ensure_entries(device, entries) {
            self.life.rebuilt = true;
        }

        // 🔴 Painting forces one thread per pixel. At any coarser rate
        // the view would be a grid of dots over an unpainted frame,
        // which reads as "the pass is broken" rather than as "you asked
        // for one sample in sixteen".
        let rate = if paint.on {
            1
        } else {
            rate.clamp(RATE_RANGE.0, RATE_RANGE.1)
        };
        queue.write_buffer(
            &self.view,
            0,
            bytemuck::bytes_of(&PageMarkView {
                world_from_clip: world_from_clip.to_cols_array_2d(),
                eye_and_base: [eye.x, eye.y, eye.z, self.clipmap.base],
                sun: sun
                    .map(|d| {
                        let d = d.normalize_or_zero();
                        [d.x, d.y, d.z, 1.0]
                    })
                    .unwrap_or([0.0, -1.0, 0.0, 0.0]),
                chain: [
                    self.config.page,
                    self.config.virtual_size,
                    self.config.levels(),
                    self.clipmap.levels,
                ],
                strides: [
                    self.config.side(0),
                    self.config.local_face_pages(),
                    self.stride(),
                    count,
                ],
                // 🔴 `sampling.y` is the SUN'S SLOT — the padded light
                // count, not the real one. The real count stays in
                // `strides.w` for the marking loop's guard.
                sampling: [rate, padded, u32::from(paint.on), view],

                pool: [
                    self.pool.entries(),
                    self.pool.config().total(),
                    self.pool.config().per_row(),
                    self.pool.config().slice(),
                ],
                life: self.life.words(),
                // How many output pixels one depth pixel covers, per
                // axis. 1 when nothing is upscaling.
                paint: [
                    paint.size.0 as f32 / viewport.0.max(1) as f32,
                    paint.size.1 as f32 / viewport.1.max(1) as f32,
                    paint.size.0 as f32,
                    paint.size.1 as f32,
                ],
                // The reciprocal, because the shader scales the world
                // size a pixel may ask a texel to match.
                density: [100.0 / density.clamp(1, 400) as f32, 0.0, 0.0, 0.0],
            }),
        );

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("page_mark_bind_group"),
            layout: &self.layout,
            entries: &[
                buffer_entry(0, lights.clusters().view_uniform()),
                buffer_entry(1, lights.clusters().cells()),
                buffer_entry(2, lights.clusters().indices()),
                buffer_entry(3, lights.light_buffer()),
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                buffer_entry(5, &self.view),
                buffer_entry(6, &self.marks),
                buffer_entry(7, &self.counters),
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(paint.target),
                },
                buffer_entry(9, &self.rank),
                buffer_entry(10, self.pool.slots()),
                buffer_entry(11, self.pool.alloc()),
            ],
        });

        // 🔴 This VIEW'S bits, not the whole bitmap. A view's pages are
        // a contiguous run — that is what `stride` is rounded to a
        // multiple of 32 for — so the reset is an offset clear.
        let words = view_span.div_ceil(32) * 4;
        encoder.clear_buffer(&self.marks, words * view as u64, Some(words));
        // This view's demand histogram and plan. Demand is a frame's
        // question; the residency it decides persists in the table.
        encoder.clear_buffer(
            &self.rank,
            view as u64 * RANK_WORDS * 4,
            Some(RANK_WORDS * 4),
        );
        // Every counter here is a per-view quantity now, the pool's
        // claims included: a view allocates out of its own slice.
        encoder.clear_buffer(&self.counters, 0, None);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("shadow pages: mark"),
                timestamp_writes: None,
            });
            // The table is flat and a view's entries are contiguous, so
            // the ageing walks exactly this view's span — the other
            // camera's pages are outside the dispatch.
            pass.set_pipeline(&self.clear);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(
                u32::try_from(view_span)
                    .unwrap_or(u32::MAX)
                    .div_ceil(GROUP * GROUP),
                1,
                1,
            );
            pass.set_pipeline(&self.pipeline);
            let threads = (viewport.0.div_ceil(rate), viewport.1.div_ceil(rate));
            pass.dispatch_workgroups(threads.0.div_ceil(GROUP), threads.1.div_ceil(GROUP), 1);
            // The seat passes (#942), in an order that is the
            // algorithm: rank the demand, clear what the plan does not
            // fund, seat what it does. Dispatch boundaries are the
            // barriers between them.
            let entries = u32::try_from(view_span)
                .unwrap_or(u32::MAX)
                .div_ceil(GROUP * GROUP);
            pass.set_pipeline(&self.plan);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(&self.preempt);
            pass.dispatch_workgroups(entries, 1, 1);
            pass.set_pipeline(&self.adopt);
            pass.dispatch_workgroups(entries, 1, 1);
        }
        // Kept for `record_paint`, which runs after the shading and needs
        // every one of these bindings — the depth, the view uniform and
        // the colour target included.
        self.bound = Some(bind_group);
        self.pending = self.readback.record(
            encoder,
            &self.counters,
            Label {
                size: viewport,
                view,
                capacity: self.pool.config().slice(),
            },
        );
    }

    /// Paints the debug view over the frame's FINAL colour.
    ///
    /// 🔴 A dispatch of its own, recorded after the shading. The marking
    /// runs at the top of the frame now so the raster can fill the atlas
    /// before anything samples it — but at the top of the frame the
    /// colour buffer still holds the last frame's image, which the fused
    /// pass is about to overwrite. Painted there, the view would be
    /// erased every frame and read as broken.
    ///
    /// Does nothing when the view is off, and nothing before the first
    /// [`Self::record`] has built the bindings.
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

    /// Maps what this frame recorded and picks up whatever earlier
    /// frames returned.
    ///
    /// Call once a frame, **after** the encoder has been submitted.
    pub fn poll(&mut self) {
        if let Some(slot) = self.pending.take() {
            self.readback.submit(slot);
        }
        if let Some(counts) = self.readback.take() {
            self.last = Some(counts);
        }
    }
}

/// Pages one LOCAL light addresses: six faces of a chain that starts at
/// `local_floor` — the levels under the floor cannot be marked, so
/// addressing them would spend table entries on pages that cannot
/// exist. At the defaults this is 2 046 pages against the 131 070 a
/// full chain would cost, and the flat table is only affordable at the
/// small number.
///
/// 🔴 Rounded up to a multiple of 32. The mark bitmap is emptied one
/// VIEW at a time and `clear_buffer` takes byte offsets, so a view's
/// first bit has to land on a word boundary or the clear reaches into
/// the neighbour's. The rounding costs at most 31 bits per light.
pub(super) fn stride(config: PageConfig, _clipmap: ClipmapConfig) -> u32 {
    let local = config.local_face_pages() * super::CUBE_FACES as u32;
    local.div_ceil(32) * 32
}

/// Light slots the address space is laid out for, PADDED so that adding
/// a light does not move every page id.
///
/// 🔴 The sun's region starts at `padded * stride` and view N's span
/// starts at `N * span`: a raw count would shift both on every light
/// added or removed, which is a full pool rebuild per change. Padding
/// to a step makes the layout stable until the scene crosses the step.
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
            // Binding 9 held the hash table's keys, was retired with the
            // flat table, and is spent again on the seating plan (#942)
            // — which puts this layout AT the eight-per-stage
            // storage-buffer downlevel limit. The next buffer this pass
            // wants has to fold into an existing one.
            storage(9, false),
            storage(10, false),
            storage(11, false),
        ],
    })
}

/// The three-slot ring the counters come home in.
///
/// The same state machine `ClusterReadback` and `MeshletStageCounters`
/// use, and for the same reason: reading sixteen bytes back
/// synchronously would stall the frame.
struct Readback {
    slots: Vec<(wgpu::Buffer, Arc<Mutex<SlotState>>)>,
    /// What each slot's dispatch was: its render size, its camera and
    /// the pool slice it allocated from.
    ///
    /// 🔴 Captured when the copy is RECORDED, not when it comes back.
    /// The ring is two or three frames deep and two cameras take turns,
    /// so reading the marker's current view at map time labels every
    /// number with whichever camera happened to run last — a reading
    /// attributed to the wrong camera is worse than no reading.
    labels: Vec<Label>,
    next: usize,
}

#[derive(Clone, Copy, Default)]
struct Label {
    size: (u32, u32),
    view: u32,
    capacity: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotState {
    Writable,
    InFlight,
    Ready,
}

impl Readback {
    fn new(device: &wgpu::Device) -> Self {
        let slots = (0..3)
            .map(|i| {
                (
                    device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(&format!("page_mark_readback_{i}")),
                        size: COUNTERS * 4,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    Arc::new(Mutex::new(SlotState::Writable)),
                )
            })
            .collect();
        Self {
            slots,
            labels: vec![Label::default(); 3],
            next: 0,
        }
    }

    /// Copies the counters into a free slot, if there is one.
    ///
    /// `None` means every slot is still in flight and the frame simply
    /// skips the readback: the cached count is one frame older, which is
    /// the same kind of stale it already was.
    fn record(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        counters: &wgpu::Buffer,
        label: Label,
    ) -> Option<usize> {
        let index = self.acquire()?;
        encoder.copy_buffer_to_buffer(counters, 0, &self.slots[index].0, 0, COUNTERS * 4);
        self.labels[index] = label;
        Some(index)
    }

    /// Asks wgpu to map the slot. Call **after** the encoder carrying
    /// the copy has been submitted.
    fn submit(&self, index: usize) {
        let (buffer, state) = &self.slots[index];
        *state.lock().unwrap() = SlotState::InFlight;
        let flag = state.clone();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                if result.is_ok() {
                    *flag.lock().unwrap() = SlotState::Ready;
                }
                // A map error is device-loss territory. Leaving the slot
                // in flight means later frames skip it rather than
                // panicking on wgpu's driver thread.
            });
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

    fn take(&mut self) -> Option<MarkCounts> {
        for (index, (buffer, state)) in self.slots.iter().enumerate() {
            if *state.lock().unwrap() != SlotState::Ready {
                continue;
            }
            let Label {
                size,
                view,
                capacity,
            } = self.labels[index];
            let counts = {
                let mapped = buffer.slice(..).get_mapped_range();
                let words: &[u32] = bytemuck::cast_slice(&mapped);
                MarkCounts {
                    resident: words[0],
                    samples: words[1],
                    pairs: words[2],
                    overflow: words[3],
                    pool: PoolCounts {
                        claims: words[8],
                        overflow: words[5],
                        reused: words[7],
                        leaked: words[10],
                        alive: words[11],
                        evicted: words[12],
                        denied: words[13],
                        preempted: words[14],
                        cutoff: words[15],
                        capacity,
                    },
                    size,
                    view,
                }
            };
            buffer.unmap();
            *state.lock().unwrap() = SlotState::Writable;
            return Some(counts);
        }
        None
    }
}

#[cfg(test)]
mod tests;
