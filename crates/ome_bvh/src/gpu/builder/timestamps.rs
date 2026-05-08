//! `impl BvhTimestamps` — constructs the per-pass timestamp query set
//! plus its resolve / readback companion buffers.

use super::TIMESTAMP_QUERY_COUNT;
use super::types::BvhTimestamps;

impl BvhTimestamps {
    pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("ome_bvh::timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: TIMESTAMP_QUERY_COUNT,
        });
        let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::timestamps_resolve"),
            size: (TIMESTAMP_QUERY_COUNT as u64) * 8,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::timestamps_readback"),
            size: (TIMESTAMP_QUERY_COUNT as u64) * 8,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let period_ns = queue.get_timestamp_period();
        Self {
            query_set,
            resolve_buffer,
            readback_buffer,
            period_ns,
        }
    }
}
