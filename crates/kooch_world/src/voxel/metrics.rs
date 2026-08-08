//! Telemetry sink — `MetricsPass` aggregates per-LOD freelist state +
//! cumulative alloc/free counters into one 24 B buffer at the tail of
//! the cascade. The host reads it asynchronously via [`Metrics::read`].
//!
//! # Not a hot-loop pass
//!
//! Although [`MetricsPass::record`] composes into the canonical
//! orchestrator (it's the 8th pass after the b18f4aa fix-up), the
//! readback is **opt-in async** — production paths keep the lookup hot
//! loop at zero CPU readback. Call [`Metrics::read`] at telemetry
//! cadence (per-second / on-demand), never per-frame from the render
//! thread.
//!
//! # VRAM accounting
//!
//! `vram_bytes` is computed host-side from [`LOD_LEVELS`] — the atlas
//! geometry is constexpr, so a GPU pass to count it would be wasted
//! work. The shader writes the runtime-varying fields (active counts +
//! cumulative pops/pushes); the host fills `vram_bytes` from the
//! constexpr table at [`Metrics::read`] time.
//!
//! [`LOD_LEVELS`]: super::LOD_LEVELS

use bytemuck::{Pod, Zeroable};

use super::{LOD_COUNT, LOD_LEVELS, METRICS_BUFFER_SIZE, POOL_TEXTURE_FORMAT, SparseGrid};

/// WGSL source of the metrics pass.
pub const METRICS_WGSL: &str = include_str!("../../shaders/sparse_metrics.wgsl");

/// Bytes per `R16Float` texel — the only pool format today.
const POOL_TEXEL_BYTES: u64 = 2;

/// Aggregated runtime metrics for one [`SparseGrid`]. `active_subgrids`
/// is per-LOD; the cumulative counters are grid-wide totals across
/// LODs (the metrics pass sums them).
///
/// `vram_bytes` is host-derived from the constexpr atlas table — see
/// [`Metrics::vram_bytes_from_lod_table`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Metrics {
    pub active_subgrids: [u32; LOD_COUNT as usize],
    pub alloc_count_total: u32,
    pub free_count_total: u32,
    pub vram_bytes: u64,
}

/// Shader-side mirror of `SparseMetrics` in `sparse_metrics.wgsl`.
/// 24 B for `LOD_COUNT = 4`. Read from `metrics_buffer` after a
/// `MetricsPass::record` dispatch + map_async readback.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct MetricsRaw {
    active_lod: [u32; LOD_COUNT as usize],
    alloc_count_total: u32,
    free_count_total: u32,
}

const _: () = assert!(
    std::mem::size_of::<MetricsRaw>() as u64 == METRICS_BUFFER_SIZE,
    "MetricsRaw layout must match METRICS_BUFFER_SIZE",
);

impl Metrics {
    /// Constexpr atlas VRAM footprint summed over [`LOD_LEVELS`]. All
    /// atlases are `R16Float`, so the total is a straight texel count
    /// times two bytes.
    pub const fn vram_bytes_from_lod_table() -> u64 {
        let mut i = 0usize;
        let mut total: u64 = 0;
        while i < LOD_LEVELS.len() {
            let lod = LOD_LEVELS[i];
            total += (lod.atlas_dim_x as u64)
                * (lod.atlas_dim_y as u64)
                * (lod.atlas_dim_z as u64)
                * POOL_TEXEL_BYTES;
            i += 1;
        }
        let _ = POOL_TEXTURE_FORMAT; // keep the format constant referenced.
        total
    }

    /// Synchronous readback for tests + CLI tools. Submits a copy from
    /// `grid.metrics_buffer()` into a fresh MAP_READ staging buffer,
    /// blocks on the device poll, parses the bytes into [`Metrics`].
    ///
    /// **Caller invariant:** [`MetricsPass::record`] must have run
    /// earlier in a previously-submitted command buffer (or in the
    /// same submission queued before this call). Otherwise the read
    /// returns whatever was last written (zeroes for a fresh grid).
    pub fn read(grid: &SparseGrid, device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kooch_world::voxel::metrics::readback_staging"),
            size: METRICS_BUFFER_SIZE,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("kooch_world::voxel::metrics::readback_encoder"),
        });
        encoder.copy_buffer_to_buffer(grid.metrics_buffer(), 0, &staging, 0, METRICS_BUFFER_SIZE);
        queue.submit(std::iter::once(encoder.finish()));

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).ok();
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("device poll");
        rx.recv()
            .expect("metrics readback channel")
            .expect("map_async ok");

        let view = slice.get_mapped_range();
        let raw: MetricsRaw = *bytemuck::from_bytes::<MetricsRaw>(&view);
        drop(view);
        staging.unmap();

        Metrics {
            active_subgrids: raw.active_lod,
            alloc_count_total: raw.alloc_count_total,
            free_count_total: raw.free_count_total,
            vram_bytes: Self::vram_bytes_from_lod_table(),
        }
    }
}

/// Compiled metrics compute pipeline. Reused across cascade
/// runs — bind groups are rebuilt per [`record`] call so the pass is
/// grid-agnostic.
pub struct MetricsPass {
    pipeline: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
}

impl MetricsPass {
    /// Build the metrics pipeline. Pins `METRICS_MAX_SUBGRIDS` from
    /// the LOD 0 atlas tile capacity (every LOD shares the same
    /// `max_subgrids` by construction).
    pub fn new(device: &wgpu::Device) -> Self {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kooch_world::voxel::metrics::bgl"),
            entries: &BGL_ENTRIES,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kooch_world::voxel::metrics::layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kooch_world::voxel::metrics::shader"),
            source: wgpu::ShaderSource::Wgsl(METRICS_WGSL.into()),
        });
        let constants: &[(&str, f64)] =
            &[("METRICS_MAX_SUBGRIDS", LOD_LEVELS[0].max_subgrids as f64)];
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("kooch_world::voxel::metrics::pipeline"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("metrics_main"),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants,
                zero_initialize_workgroup_memory: true,
            },
            cache: None,
        });
        Self { pipeline, bgl }
    }

    /// Record one `(1, 1, 1)` dispatch into `encoder`. The shader reads
    /// every LOD's counters buffer and writes the aggregated metrics
    /// into `grid.metrics_buffer()`.
    pub fn record(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        grid: &SparseGrid,
    ) {
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kooch_world::voxel::metrics::bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: grid.counters_buffer(0).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: grid.counters_buffer(1).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: grid.counters_buffer(2).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: grid.counters_buffer(3).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: grid.metrics_buffer().as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("kooch_world::voxel::metrics::pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
}

const READ_STORAGE: wgpu::BindGroupLayoutEntry = wgpu::BindGroupLayoutEntry {
    binding: 0,
    visibility: wgpu::ShaderStages::COMPUTE,
    ty: wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: true },
        has_dynamic_offset: false,
        min_binding_size: None,
    },
    count: None,
};

const RW_STORAGE: wgpu::BindGroupLayoutEntry = wgpu::BindGroupLayoutEntry {
    binding: 4,
    visibility: wgpu::ShaderStages::COMPUTE,
    ty: wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: false },
        has_dynamic_offset: false,
        min_binding_size: None,
    },
    count: None,
};

const BGL_ENTRIES: [wgpu::BindGroupLayoutEntry; 5] = [
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        ..READ_STORAGE
    },
    wgpu::BindGroupLayoutEntry {
        binding: 1,
        ..READ_STORAGE
    },
    wgpu::BindGroupLayoutEntry {
        binding: 2,
        ..READ_STORAGE
    },
    wgpu::BindGroupLayoutEntry {
        binding: 3,
        ..READ_STORAGE
    },
    RW_STORAGE,
];

#[cfg(test)]
mod tests;
