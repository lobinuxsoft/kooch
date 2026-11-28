//! Every terrain's stamps in one storage buffer, each terrain a range of it.

use std::sync::Arc;

use super::GpuStamp;

/// The stamps the generate pass reads; a job carries its terrain's range.
pub struct StampBuffer {
    buffer: wgpu::Buffer,
    capacity: u64,
    /// Per terrain index, its stamps.
    lists: Vec<Arc<[GpuStamp]>>,
    ranges: Vec<[u32; 2]>,
    dirty: bool,
}

impl StampBuffer {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            buffer: create(device, 1),
            capacity: 1,
            lists: Vec::new(),
            ranges: Vec::new(),
            dirty: false,
        }
    }

    /// Terrain `planet`'s stamps from now on.
    pub fn set(&mut self, planet: u32, stamps: &Arc<[GpuStamp]>) {
        let at = planet as usize;
        if self.lists.len() <= at {
            self.lists.resize_with(at + 1, || Arc::from([]));
        }
        if !Arc::ptr_eq(&self.lists[at], stamps) && *self.lists[at] != **stamps {
            self.lists[at] = Arc::clone(stamps);
            self.dirty = true;
        }
    }

    /// Writes what changed since the last call; grows the buffer when it no longer fits.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.ranges.clear();
        let mut all = Vec::new();
        for list in &self.lists {
            self.ranges.push([all.len() as u32, list.len() as u32]);
            all.extend_from_slice(list);
        }
        let needed = (all.len() as u64).max(1);
        if needed > self.capacity {
            self.capacity = needed.next_power_of_two();
            self.buffer = create(device, self.capacity);
        }
        if !all.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&all));
        }
    }

    /// `[first, count]` of terrain `planet`'s stamps, as of the last upload.
    pub fn range(&self, planet: u32) -> [u32; 2] {
        self.ranges.get(planet as usize).copied().unwrap_or([0, 0])
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}

fn create(device: &wgpu::Device, stamps: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("kooch_terrain::stamps"),
        size: stamps * size_of::<GpuStamp>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
