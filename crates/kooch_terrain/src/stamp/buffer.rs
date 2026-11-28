//! Every terrain's stamps in one storage buffer, each terrain a range of it, and every field they
//! sample in a second one (#709).

use std::sync::Arc;

use kooch_core::Guid;

use super::{GpuStamp, SHAPE_FIELD};
use crate::field::Field;

/// Words a field's descriptor takes: texel offset, kind, size x/y/z, lo, hi, a spare.
const DESCRIPTOR: usize = 8;

/// The stamps the generate pass reads; a job carries its terrain's range.
pub struct StampBuffer {
    buffer: wgpu::Buffer,
    capacity: u64,
    /// Per terrain index, its stamps and the fields they index.
    lists: Vec<(Arc<[GpuStamp]>, Vec<Arc<Field>>)>,
    ranges: Vec<[u32; 2]>,
    dirty: bool,
    fields: wgpu::Buffer,
    field_capacity: u64,
    /// Each field once, however many terrains name it: what the second buffer holds.
    table: Vec<Arc<Field>>,
}

impl StampBuffer {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            buffer: create(
                device,
                "kooch_terrain::stamps",
                size_of::<GpuStamp>() as u64,
            ),
            capacity: 1,
            lists: Vec::new(),
            ranges: Vec::new(),
            dirty: false,
            fields: create(device, "kooch_terrain::fields", 4 * DESCRIPTOR as u64),
            field_capacity: DESCRIPTOR as u64,
            table: Vec::new(),
        }
    }

    /// Terrain `planet`'s stamps from now on, and the fields their `field` indexes.
    pub fn set(&mut self, planet: u32, stamps: &Arc<[GpuStamp]>, fields: &[(Guid, Arc<Field>)]) {
        let at = planet as usize;
        if self.lists.len() <= at {
            self.lists
                .resize_with(at + 1, || (Arc::from([]), Vec::new()));
        }
        let (held, named) = &mut self.lists[at];
        let same_fields =
            named.len() == fields.len() && named.iter().zip(fields).all(|(a, (_, b))| a.same(b));
        if !same_fields {
            *named = fields.iter().map(|(_, field)| Arc::clone(field)).collect();
            self.dirty = true;
        }
        if !Arc::ptr_eq(held, stamps) && **held != **stamps {
            *held = Arc::clone(stamps);
            self.dirty = true;
        }
    }

    /// Writes what changed since the last call; grows the buffers when they no longer fit. The
    /// texels go up again only when the set of fields changed, not when a stamp moved.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let mut table: Vec<Arc<Field>> = Vec::new();
        let mut global = |field: &Arc<Field>| -> u32 {
            let at = table.iter().position(|held| held.same(field));
            at.unwrap_or_else(|| {
                table.push(Arc::clone(field));
                table.len() - 1
            }) as u32
        };
        self.ranges.clear();
        let mut all = Vec::new();
        for (list, named) in &self.lists {
            self.ranges.push([all.len() as u32, list.len() as u32]);
            let first = all.len();
            all.extend_from_slice(list);
            for stamp in &mut all[first..] {
                if stamp.shape == SHAPE_FIELD
                    && let Some(field) = named.get(stamp.field as usize)
                {
                    stamp.field = global(field);
                }
            }
        }
        let needed = (all.len() as u64).max(1);
        if needed > self.capacity {
            self.capacity = needed.next_power_of_two();
            let bytes = self.capacity * size_of::<GpuStamp>() as u64;
            self.buffer = create(device, "kooch_terrain::stamps", bytes);
        }
        if !all.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&all));
        }
        let same = table.len() == self.table.len()
            && table.iter().zip(&self.table).all(|(a, b)| a.same(b));
        if !same {
            let words = pack(&table);
            let needed = (words.len() as u64).max(DESCRIPTOR as u64);
            let ceiling = device.limits().max_storage_buffer_binding_size / 4;
            if needed > ceiling {
                // Past the binding the buffer would come back invalid and take the render with
                // it; the fields uploaded last stay instead.
                tracing::error!(
                    fields = table.len(),
                    mib = (needed * 4) >> 20,
                    ceiling_mib = (ceiling * 4) >> 20,
                    "terrain fields exceed the storage binding; bake them at a lower resolution"
                );
                return;
            }
            if needed > self.field_capacity {
                self.field_capacity = needed.next_power_of_two().min(ceiling);
                self.fields = create(device, "kooch_terrain::fields", self.field_capacity * 4);
            }
            if !words.is_empty() {
                queue.write_buffer(&self.fields, 0, bytemuck::cast_slice(&words));
            }
            self.table = table;
        }
    }

    /// `[first, count]` of terrain `planet`'s stamps, as of the last upload.
    pub fn range(&self, planet: u32) -> [u32; 2] {
        self.ranges.get(planet as usize).copied().unwrap_or([0, 0])
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    pub fn fields(&self) -> &wgpu::Buffer {
        &self.fields
    }
}

/// The descriptors, then each field's texels two to a word, as `field_at` reads them.
fn pack(table: &[Arc<Field>]) -> Vec<u32> {
    let mut words = vec![0u32; table.len() * DESCRIPTOR];
    for (index, field) in table.iter().enumerate() {
        let start = words.len() as u32;
        words.extend(
            field.texels.chunks(2).map(|pair| {
                u32::from(pair[0]) | (u32::from(pair.get(1).copied().unwrap_or(0)) << 16)
            }),
        );
        let descriptor = [
            start,
            field.kind,
            field.size[0],
            field.size[1],
            field.size[2],
            field.lo.to_bits(),
            field.hi.to_bits(),
            0,
        ];
        words[index * DESCRIPTOR..(index + 1) * DESCRIPTOR].copy_from_slice(&descriptor);
    }
    words
}

fn create(device: &wgpu::Device, label: &str, bytes: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
