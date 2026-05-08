use wgpu::util::DeviceExt;

use crate::gpu::sort_types::{
    OnesweepConfig, RADIX_PASSES, global_histogram_size_bytes, partition_descriptors_size_bytes,
};

use super::config::{InitConfig, ScanConfig};
use super::{INITIAL_KEYS_CAPACITY, INITIAL_PARTITIONS};

/// Reusable GPU buffers for the sort. Grow on demand, never realloc'd
/// per build (per the production-from-day-1 rule). The keys / values
/// buffers are ping-pong: each radix pass swaps `*_in` and `*_out`.
pub struct SortBuffers {
    pub global_histogram: wgpu::Buffer,
    pub partition_descriptors: wgpu::Buffer,
    pub partitions_capacity: u32,
    pub keys_a: wgpu::Buffer,
    pub keys_b: wgpu::Buffer,
    pub values_a: wgpu::Buffer,
    pub values_b: wgpu::Buffer,
    pub keys_capacity: u64,
    pub config_buffer: wgpu::Buffer,
    pub init_config_buffer: wgpu::Buffer,
    /// One config buffer per pass — `queue.write_buffer` against the
    /// same buffer between dispatches doesn't serialise within a single
    /// submission, so each pass needs its own buffer (or dynamic-offset
    /// uniforms; simplicity wins at 4 × 16 bytes).
    pub scan_config_buffers: [wgpu::Buffer; RADIX_PASSES as usize],
    /// Same per-pass story for the scatter config — each pass needs
    /// its own `pass_shift` written before its dispatch.
    pub scatter_config_buffers: [wgpu::Buffer; RADIX_PASSES as usize],
}

impl SortBuffers {
    pub fn new(device: &wgpu::Device) -> Self {
        let global_histogram = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::sort_global_histogram"),
            size: global_histogram_size_bytes(),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let partition_descriptors = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::sort_partition_descriptors"),
            size: partition_descriptors_size_bytes(INITIAL_PARTITIONS),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let keys_a = make_keys_buffer(device, INITIAL_KEYS_CAPACITY, "keys_a");
        let keys_b = make_keys_buffer(device, INITIAL_KEYS_CAPACITY, "keys_b");
        let values_a = make_keys_buffer(device, INITIAL_KEYS_CAPACITY, "values_a");
        let values_b = make_keys_buffer(device, INITIAL_KEYS_CAPACITY, "values_b");
        let config_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ome_bvh::sort_config"),
            contents: bytemuck::bytes_of(&OnesweepConfig::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let init_config_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ome_bvh::sort_init_config"),
            contents: bytemuck::bytes_of(&InitConfig::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let scan_config_buffers: [wgpu::Buffer; RADIX_PASSES as usize] = std::array::from_fn(|i| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("ome_bvh::sort_scan_config_{i}")),
                contents: bytemuck::bytes_of(&ScanConfig {
                    pass_index: i as u32,
                    _pad0: 0,
                    _pad1: 0,
                    _pad2: 0,
                }),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
        });
        // Scatter configs filled at dispatch time with the live count
        // and partition_count; only `pass_shift` is fixed per pass.
        let scatter_config_buffers: [wgpu::Buffer; RADIX_PASSES as usize] =
            std::array::from_fn(|i| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("ome_bvh::sort_scatter_config_{i}")),
                    contents: bytemuck::bytes_of(&OnesweepConfig::new(0, i as u32)),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                })
            });
        Self {
            global_histogram,
            partition_descriptors,
            partitions_capacity: INITIAL_PARTITIONS,
            keys_a,
            keys_b,
            values_a,
            values_b,
            keys_capacity: INITIAL_KEYS_CAPACITY,
            config_buffer,
            init_config_buffer,
            scan_config_buffers,
            scatter_config_buffers,
        }
    }

    /// Grow the keys/values buffers to fit `count` items, and the
    /// partition descriptors buffer to fit `partition_count` partitions.
    pub fn ensure_capacity(
        &mut self,
        device: &wgpu::Device,
        count: u64,
        partition_count: u32,
    ) {
        if count > self.keys_capacity {
            let new_cap = count.next_power_of_two().max(INITIAL_KEYS_CAPACITY);
            self.keys_a = make_keys_buffer(device, new_cap, "keys_a");
            self.keys_b = make_keys_buffer(device, new_cap, "keys_b");
            self.values_a = make_keys_buffer(device, new_cap, "values_a");
            self.values_b = make_keys_buffer(device, new_cap, "values_b");
            self.keys_capacity = new_cap;
        }
        if partition_count > self.partitions_capacity {
            let new_cap = partition_count.next_power_of_two().max(INITIAL_PARTITIONS);
            self.partition_descriptors = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ome_bvh::sort_partition_descriptors"),
                size: partition_descriptors_size_bytes(new_cap),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.partitions_capacity = new_cap;
        }
    }
}

fn make_keys_buffer(device: &wgpu::Device, capacity: u64, label: &str) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&format!("ome_bvh::sort_{label}")),
        size: capacity * 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
