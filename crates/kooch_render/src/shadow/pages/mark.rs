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

use kooch_lighting::{CLUSTER_COMMON, GpuLights};

use super::{ClipmapConfig, PageConfig};

const SOURCE: &str = include_str!("../../../shaders/page_mark.wgsl");
const GROUP: u32 = 8;
const COUNTERS: u64 = 4;

/// `KOOCH_PAGE_MARKING=1`, read once.
///
/// Only the **default** of [`PageMarkingSettings`](super::PageMarkingSettings):
/// the panel owns it after that, the way the froxel grid's checkbox
/// owns `ClusterSettings::enabled`.
pub fn enabled_by_environment() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("KOOCH_PAGE_MARKING")
            .is_ok_and(|v| v != "0" && !v.eq_ignore_ascii_case("off"))
    })
}

/// `KOOCH_PAGE_MARKING_RATE`, read once. The default of
/// [`PageMarkingSettings::rate`](super::PageMarkingSettings::rate).
pub fn rate_from_environment() -> u32 {
    static RATE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *RATE.get_or_init(|| {
        std::env::var("KOOCH_PAGE_MARKING_RATE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1)
            .clamp(RATE_RANGE.0, RATE_RANGE.1)
    })
}

/// What the panel's slider allows, and what `record` clamps to.
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
    paint: [f32; 4],
}

/// The pass, its buffers, and the ring that brings the count home.
pub struct PageMarker {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    view: wgpu::Buffer,
    marks: wgpu::Buffer,
    counters: wgpu::Buffer,
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
    last: Option<MarkCounts>,
}

impl PageMarker {
    pub fn new(device: &wgpu::Device, config: PageConfig, clipmap: ClipmapConfig) -> Self {
        let layout = layout(device);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("page_mark"),
            source: wgpu::ShaderSource::Wgsl(format!("{CLUSTER_COMMON}\n{SOURCE}").into()),
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
            last: None,
        }
    }

    /// The last count that came back, a frame or two old.
    pub fn last(&self) -> Option<MarkCounts> {
        self.last
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
                // How many output pixels one depth pixel covers, per
                // axis. 1 when nothing is upscaling.
                paint: [
                    paint.size.0 as f32 / viewport.0.max(1) as f32,
                    paint.size.1 as f32 / viewport.1.max(1) as f32,
                    paint.size.0 as f32,
                    paint.size.1 as f32,
                ],
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
            ],
        });

        encoder.clear_buffer(&self.marks, 0, None);
        encoder.clear_buffer(&self.counters, 0, None);
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
        if let Some(counts) = self.readback.take() {
            self.last = Some(counts);
        }
    }
}

/// Pages one light can address: the six faces of a mip chain, or the
/// clipmap, whichever is longer.
///
/// Mirrors `PageCensus::new`. One stride for every light — a per-kind
/// stride would save bits and cost a prefix sum to find a light's base.
fn stride(config: PageConfig, clipmap: ClipmapConfig) -> u32 {
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

    fn take(&mut self) -> Option<MarkCounts> {
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
                }
            };
            buffer.unmap();
            *state.lock().unwrap() = SlotState::Writable;
            return Some(counts);
        }
        None
    }
}
