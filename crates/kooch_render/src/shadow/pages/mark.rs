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

use super::pool::{PagePool, PoolConfig, PoolCounts};
use super::{ClipmapConfig, PageConfig};

const SOURCE: &str = include_str!("../../../shaders/page_mark.wgsl");
const GROUP: u32 = 8;
/// 0 resident, 1 samples, 2 pairs, 3 mark overflow, 4 claims, 5 pool
/// overflow, 6 probe overflow. 7 spare, because a storage buffer is
/// rounded up anyway.
const COUNTERS: u64 = 8;

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
    density: [f32; 4],
}

/// The pass, its buffers, and the ring that brings the count home.
pub struct PageMarker {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    view: wgpu::Buffer,
    marks: wgpu::Buffer,
    counters: wgpu::Buffer,
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
    /// Lights the mark buffer is sized for.
    capacity: u32,
    /// The render size of the dispatch now in flight.
    size: (u32, u32),
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
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("page_mark"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("mark_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self {
            layout,
            pipeline,
            view: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_mark_view"),
                size: std::mem::size_of::<PageMarkView>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            marks: marks_buffer(device, config, clipmap, 1),
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
            pending: None,
            config,
            clipmap,
            capacity: 1,
            size: (0, 0),
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
    /// The table is rebuilt, not migrated: it is emptied every frame
    /// anyway, so there is nothing in it worth carrying across.
    pub fn set_pool(&mut self, device: &wgpu::Device, config: PoolConfig) -> bool {
        self.pool.resize(device, config)
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
        rate: u32,
        // Shadow texels per screen pixel, as a percentage.
        density: u32,
        paint: Paint<'_>,
    ) {
        let count = lights.light_count().max(1);
        // One slot past the lights, for the sun: it is not in the grid
        // — it has no position to cluster — so it gets a stride of its
        // own rather than a light index.
        let slots = count + 1;
        if slots > self.capacity {
            self.marks = marks_buffer(device, self.config, self.clipmap, slots);
            self.capacity = slots;
        }

        // 🔴 Painting forces one thread per pixel. At any coarser rate
        // the view would be a grid of dots over an unpainted frame,
        // which reads as "the pass is broken" rather than as "you asked
        // for one sample in sixteen".
        self.size = viewport;
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
                    self.config.face_pages(),
                    self.stride(),
                    count,
                ],
                sampling: [rate, count, u32::from(paint.on), 0],
                pool: [
                    self.pool.config().entries(),
                    self.pool.config().pages,
                    self.pool.config().per_row(),
                    0,
                ],
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
                buffer_entry(9, self.pool.keys()),
                buffer_entry(10, self.pool.slots()),
            ],
        });

        encoder.clear_buffer(&self.marks, 0, None);
        encoder.clear_buffer(&self.counters, 0, None);
        self.pool.clear(encoder);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("shadow pages: mark"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            let threads = (viewport.0.div_ceil(rate), viewport.1.div_ceil(rate));
            pass.dispatch_workgroups(threads.0.div_ceil(GROUP), threads.1.div_ceil(GROUP), 1);
        }
        self.pending = self.readback.record(encoder, &self.counters);
    }

    /// Maps what this frame recorded and picks up whatever earlier
    /// frames returned.
    ///
    /// Call once a frame, **after** the encoder has been submitted.
    pub fn poll(&mut self) {
        if let Some(slot) = self.pending.take() {
            self.readback.submit(slot);
        }
        if let Some(counts) = self.readback.take(self.size, self.pool.config().pages) {
            self.last = Some(counts);
        }
    }
}

/// Pages one light can address: the six faces of a mip chain, or the
/// clipmap, whichever is longer.
///
/// Mirrors `PageCensus::new`. One stride for every light — a per-kind
/// stride would save bits and cost a prefix sum to find a light's base.
pub(super) fn stride(config: PageConfig, clipmap: ClipmapConfig) -> u32 {
    let local = config.face_pages() * super::CUBE_FACES as u32;
    let sun = clipmap.levels * config.side(0).pow(2);
    local.max(sun)
}

fn marks_buffer(
    device: &wgpu::Device,
    config: PageConfig,
    clipmap: ClipmapConfig,
    slots: u32,
) -> wgpu::Buffer {
    let bits = stride(config, clipmap) as u64 * slots as u64;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("page_mark_bits"),
        size: bits.div_ceil(32).max(1) * 4,
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
            storage(9, false),
            storage(10, false),
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
    next: usize,
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
        Self { slots, next: 0 }
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
    ) -> Option<usize> {
        let index = self.acquire()?;
        encoder.copy_buffer_to_buffer(counters, 0, &self.slots[index].0, 0, COUNTERS * 4);
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

    fn take(&mut self, size: (u32, u32), capacity: u32) -> Option<MarkCounts> {
        for (buffer, state) in &self.slots {
            if *state.lock().unwrap() != SlotState::Ready {
                continue;
            }
            let counts = {
                let view = buffer.slice(..).get_mapped_range();
                let words: &[u32] = bytemuck::cast_slice(&view);
                MarkCounts {
                    resident: words[0],
                    samples: words[1],
                    pairs: words[2],
                    overflow: words[3],
                    pool: PoolCounts {
                        claims: words[4],
                        overflow: words[5],
                        probes: words[6],
                        capacity,
                    },
                    size,
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
mod tests {
    use super::*;

    /// The same guard the raster has, on the mirror that has existed
    /// longest.
    ///
    /// 🔴 `PageView` is described as mirroring `PageMarkView` "field for
    /// field", and a comment saying so is not a check. The identical
    /// claim in the raster was false: a `vec3<u32>` of padding made the
    /// shader's struct twice the Rust one, and it surfaced as a
    /// per-frame bind error rather than as a failing test.
    #[test]
    fn the_view_mirror_matches_the_shader() {
        let source = format!("{CLUSTER_COMMON}\n{PAGE_TABLE}\n{SOURCE}");
        let module = naga::front::wgsl::parse_str(&source).expect("the shader parses");
        let mut layouter = naga::proc::Layouter::default();
        layouter
            .update(module.to_ctx())
            .expect("the shader has a layout");
        let size = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some("PageView"))
            .map(|(handle, _)| layouter[handle].size)
            .expect("`PageView` is declared");
        assert_eq!(size as usize, std::mem::size_of::<PageMarkView>());
    }
}
