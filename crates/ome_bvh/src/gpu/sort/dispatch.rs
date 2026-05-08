use crate::gpu::sort_types::{
    OnesweepConfig, RADIX_BUCKETS, RADIX_PASSES, SORT_WORKGROUP_SIZE,
};

use super::buffers::SortBuffers;
use super::config::{HistogramConfig, InitConfig};
use super::pipelines::SortPipelines;

/// Dispatch helper: clears the global histogram + partition descriptor
/// section for one pass. Caller is responsible for ensuring buffers
/// exist (call `SortBuffers::ensure_capacity` first).
pub fn dispatch_init(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &SortPipelines,
    buffers: &SortBuffers,
    partition_count: u32,
) {
    let cfg = InitConfig {
        histogram_count: RADIX_PASSES * RADIX_BUCKETS,
        descriptor_count: partition_count * RADIX_BUCKETS,
        _pad0: 0,
        _pad1: 0,
    };
    queue.write_buffer(&buffers.init_config_buffer, 0, bytemuck::bytes_of(&cfg));

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ome_bvh::sort_init_bg"),
        layout: &pipelines.init_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.global_histogram.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers.partition_descriptors.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffers.init_config_buffer.as_entire_binding(),
            },
        ],
    });

    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("ome_bvh::sort_init_pass"),
        timestamp_writes: None,
    });
    pass.set_pipeline(&pipelines.init_pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    // 1 workgroup is enough for the small histogram; descriptor cleanup
    // uses the strided loop in the shader.
    pass.dispatch_workgroups(1, 1, 1);
}

/// Dispatch helper: count digits across all 4 passes in one dispatch.
/// `keys_buffer` is the input array of u32 (Morton codes); the output
/// is `buffers.global_histogram`.
pub fn dispatch_histogram(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &SortPipelines,
    buffers: &SortBuffers,
    keys_buffer: &wgpu::Buffer,
    count: u32,
) {
    let cfg = HistogramConfig {
        count,
        _pad0: 0,
        _pad1: 0,
        _pad2: 0,
    };
    queue.write_buffer(&buffers.config_buffer, 0, bytemuck::bytes_of(&cfg));

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ome_bvh::sort_histogram_bg"),
        layout: &pipelines.histogram_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: keys_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers.global_histogram.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffers.config_buffer.as_entire_binding(),
            },
        ],
    });

    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("ome_bvh::sort_histogram_pass"),
        timestamp_writes: None,
    });
    pass.set_pipeline(&pipelines.histogram_pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    let partitions = (count + crate::gpu::sort_types::ITEMS_PER_TILE - 1)
        / crate::gpu::sort_types::ITEMS_PER_TILE;
    pass.dispatch_workgroups(partitions.max(1), 1, 1);
}

/// Dispatch helper: convert one pass's bucket counts into exclusive
/// prefix sums (per-bucket starting offsets in the sorted output).
/// One dispatch per pass; uses the per-pass static config buffer
/// initialised at `SortBuffers` construction.
pub fn dispatch_exclusive_scan(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &SortPipelines,
    buffers: &SortBuffers,
    pass_index: u32,
) {
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ome_bvh::sort_scan_bg"),
        layout: &pipelines.scan_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.global_histogram.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers.scan_config_buffers[pass_index as usize].as_entire_binding(),
            },
        ],
    });

    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("ome_bvh::sort_scan_pass"),
        timestamp_writes: None,
    });
    pass.set_pipeline(&pipelines.scan_pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    // 1 workgroup × 256 threads handles 256 buckets.
    pass.dispatch_workgroups(1, 1, 1);
    let _ = SORT_WORKGROUP_SIZE; // keep referenced for future scatter use
}

/// Dispatch helper: per-pass scatter via decoupled-lookback chained
/// scan + scatter to output. The caller must clear partition
/// descriptors via `dispatch_init` BEFORE this scatter dispatch (the
/// onesweep state machine assumes every descriptor starts INVALID).
///
/// `pass_index` selects: which radix digit (`pass_shift = pass_index *
/// 8`), which scatter config buffer to use, and which `keys_in/out` +
/// `values_in/out` buffer pair (ping-pong: even passes read `_a`,
/// write `_b`; odd passes swap).
pub fn dispatch_scatter(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &SortPipelines,
    buffers: &SortBuffers,
    count: u32,
    pass_index: u32,
) {
    let cfg = OnesweepConfig::new(count, pass_index);
    queue.write_buffer(
        &buffers.scatter_config_buffers[pass_index as usize],
        0,
        bytemuck::bytes_of(&cfg),
    );

    let (keys_in, keys_out, values_in, values_out) = if pass_index % 2 == 0 {
        (&buffers.keys_a, &buffers.keys_b, &buffers.values_a, &buffers.values_b)
    } else {
        (&buffers.keys_b, &buffers.keys_a, &buffers.values_b, &buffers.values_a)
    };

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ome_bvh::sort_scatter_bg"),
        layout: &pipelines.scatter_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: keys_in.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: values_in.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: keys_out.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: values_out.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: buffers.global_histogram.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: buffers.partition_descriptors.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: buffers.scatter_config_buffers[pass_index as usize]
                    .as_entire_binding(),
            },
        ],
    });

    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("ome_bvh::sort_scatter_pass"),
        timestamp_writes: None,
    });
    pass.set_pipeline(&pipelines.scatter_pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    pass.dispatch_workgroups(cfg.partition_count.max(1), 1, 1);
}

/// Full sort orchestration recorded into the caller-supplied encoder:
/// init → histogram (all passes) → exclusive scan (4 dispatches) →
/// scatter (4 dispatches with ping-pong).
///
/// Caller is responsible for placing the input keys in `buffers.keys_a`
/// (and matching `values_a` if needed) before submitting the encoder.
/// On submission completion, sorted keys land in `buffers.keys_a` and
/// the permuted values in `buffers.values_a` (even pass count → result
/// returns to the `_a` slot).
///
/// **Important**: the scan over the global histogram is destructive
/// (in-place: counts → exclusive prefix). Each scatter pass needs the
/// scanned histogram for its specific pass index; we run all 4 scans
/// in one batch BEFORE any scatter, then scatter consumes them in
/// order. The descriptor table is cleared by `dispatch_init` at the
/// start; subsequent passes need re-clearing because each scatter
/// publishes new descriptors. We re-issue clear-only between passes.
pub fn dispatch_sort_into(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &SortPipelines,
    buffers: &SortBuffers,
    count: u32,
) {
    let partitions = (count + crate::gpu::sort_types::ITEMS_PER_TILE - 1)
        / crate::gpu::sort_types::ITEMS_PER_TILE;

    // 1. Clear histogram + descriptors.
    dispatch_init(device, queue, encoder, pipelines, buffers, partitions);

    // 2. Compute per-pass histograms (all 4 in one dispatch).
    dispatch_histogram(
        device,
        queue,
        encoder,
        pipelines,
        buffers,
        &buffers.keys_a,
        count,
    );

    // 3. Exclusive scan each pass's histogram in place.
    for pass_index in 0..RADIX_PASSES {
        dispatch_exclusive_scan(device, encoder, pipelines, buffers, pass_index);
    }

    // 4. Scatter each pass with ping-pong. Re-clear descriptors before
    // each pass (the onesweep state machine starts from all-INVALID).
    // CRITICAL: clear ONLY descriptors, NOT the global histogram —
    // the histogram holds the prefix-summed offsets that every scatter
    // pass reads. `encoder.clear_buffer` is encoder-ordered (unlike
    // `queue.write_buffer`) so the clear lands before the dispatch.
    for pass_index in 0..RADIX_PASSES {
        if pass_index > 0 {
            encoder.clear_buffer(&buffers.partition_descriptors, 0, None);
        }
        dispatch_scatter(device, queue, encoder, pipelines, buffers, count, pass_index);
    }
}

/// Standalone sort orchestration — creates its own encoder and returns
/// it for the caller to submit. Wrapper around [`dispatch_sort_into`]
/// used by the existing onesweep unit tests; production code records
/// the sort into a shared encoder via `dispatch_sort_into` so morton +
/// sort + lbvh share a single submission.
pub fn dispatch_sort(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipelines: &SortPipelines,
    buffers: &SortBuffers,
    count: u32,
) -> wgpu::CommandEncoder {
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ome_bvh::sort_encoder"),
        });
    dispatch_sort_into(device, queue, &mut encoder, pipelines, buffers, count);
    encoder
}
