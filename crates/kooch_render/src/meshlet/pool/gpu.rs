//! GPU-side mirror of [`super::GlobalMeshPool`] — six storage buffers the cull / vbuf / deferred
//! shaders read through one bind group, kept across frames and written in place.

use bytemuck::Pod;

use super::GlobalMeshPool;

/// How much slack a reallocation takes, so a pool that keeps growing stops reallocating.
const GROWTH: u64 = 2;

impl GlobalMeshPool {
    /// Allocates the GPU mirror and copies the whole pool into it. Keep the result alive across
    /// frames and call [`GpuGlobalMeshPool::sync`] after that.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> GpuGlobalMeshPool {
        let mut gpu = GpuGlobalMeshPool {
            mesh_descriptors: empty(device, "global_pool_mesh_descriptors"),
            meshlets: empty(device, "global_pool_meshlets"),
            vertices: empty(device, "global_pool_vertices"),
            meshlet_vertices: empty(device, "global_pool_meshlet_vertices"),
            meshlet_triangles: empty(device, "global_pool_meshlet_triangles"),
            mesh_bounds: empty(device, "global_pool_mesh_bounds"),
            mesh_count: 0,
            max_meshlets_per_mesh: 0,
        };
        self.mark_all();
        gpu.sync(device, queue, self);
        gpu
    }
}

impl GpuGlobalMeshPool {
    /// Copies into the buffers only what the pool has written since the last call.
    ///
    /// 🔴 This is the whole point of #1558: an edited block used to throw all six buffers away and
    /// re-upload every mesh in the scene, every frame of a drag. A buffer is only reallocated when
    /// the pool outgrows it, and nothing caches a bind group across frames, so a new buffer
    /// invalidates nothing.
    pub fn sync(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, pool: &mut GlobalMeshPool) {
        self.mesh_count = pool.mesh_count();
        self.max_meshlets_per_mesh = pool.max_meshlets_per_mesh();
        let mut dirty = pool.take_dirty();

        write(
            device,
            queue,
            &mut self.vertices,
            &pool.vertices,
            dirty.vertices.take(),
            "global_pool_vertices",
        );
        write(
            device,
            queue,
            &mut self.meshlet_vertices,
            &pool.meshlet_vertices,
            dirty.meshlet_vertices.take(),
            "global_pool_meshlet_vertices",
        );
        write(
            device,
            queue,
            &mut self.meshlet_triangles,
            &pool.meshlet_triangles,
            dirty.triangles.take(),
            "global_pool_meshlet_triangles",
        );
        write(
            device,
            queue,
            &mut self.meshlets,
            &pool.meshlets,
            dirty.meshlets.take(),
            "global_pool_meshlets",
        );
        let descriptors = dirty.descriptors.take();
        write(
            device,
            queue,
            &mut self.mesh_descriptors,
            &pool.mesh_descriptors,
            descriptors.clone(),
            "global_pool_mesh_descriptors",
        );
        // One vec4 per mesh, parallel to the descriptors, for the lamp cull's light/instance
        // pre-pass (#939) — so it is dirty exactly when they are.
        let bounds: Vec<[f32; 4]> = pool
            .mesh_bounds
            .iter()
            .map(|b| [b.center.x, b.center.y, b.center.z, b.radius])
            .collect();
        write(
            device,
            queue,
            &mut self.mesh_bounds,
            &bounds,
            descriptors,
            "global_pool_mesh_bounds",
        );
    }
}

/// Writes `range` of `data` into `buffer`, or reallocates and writes all of it when `data` has
/// outgrown what is there.
fn write<T: Pod>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffer: &mut wgpu::Buffer,
    data: &[T],
    range: Option<std::ops::Range<usize>>,
    label: &str,
) {
    let stride = std::mem::size_of::<T>() as u64;
    let needed = (data.len() as u64 * stride).max(4).next_multiple_of(4);
    if buffer.size() < needed {
        *buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: needed * GROWTH,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(buffer, 0, &padded(data));
        return;
    }
    let Some(range) = range.filter(|range| range.end <= data.len()) else {
        return;
    };
    // `write_buffer` wants both offset and length on a 4-byte boundary. Every array but the
    // triangle bytes has a stride of at least 4, and the triangle spans are reserved in whole
    // words for the same reason the shader needs them to be.
    let offset = range.start as u64 * stride;
    let bytes = bytemuck::cast_slice(&data[range]);
    if offset % 4 == 0 && bytes.len() % 4 == 0 {
        queue.write_buffer(buffer, offset, bytes);
    } else {
        queue.write_buffer(buffer, 0, &padded(data));
    }
}

/// A one-record buffer: a storage binding cannot be empty, and the pool starts out so.
fn empty(device: &wgpu::Device, label: &str) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// GPU-resident pool, kept across frames. Bound by the cull shader (mesh-descriptor lookups
/// to redirect per-meshlet reads) and the vbuf rasterizer (vertex pull
/// through the same arrays).
pub struct GpuGlobalMeshPool {
    pub mesh_descriptors: wgpu::Buffer,
    pub meshlets: wgpu::Buffer,
    pub vertices: wgpu::Buffer,
    pub meshlet_vertices: wgpu::Buffer,
    pub meshlet_triangles: wgpu::Buffer,
    /// Per-mesh bounds sphere as `(center, radius)` vec4s, parallel to `mesh_descriptors`. Not part
    /// of the shared 5-entry pool layout — only the lamp cull (#939) binds it, in its own layout,
    /// so no existing shader mirror moves.
    pub mesh_bounds: wgpu::Buffer,
    pub mesh_count: u32,
    pub max_meshlets_per_mesh: u32,
}

impl GpuGlobalMeshPool {
    /// What the six buffers actually hold, which is what a VRAM figure means.
    ///
    /// 🔴 Not `GlobalMeshPool::byte_size`: the buffers carry `GROWTH` slack and never shrink, so
    /// the CPU pool's size is neither what was allocated nor what is still held.
    pub fn bytes(&self) -> u64 {
        self.mesh_descriptors.size()
            + self.meshlets.size()
            + self.vertices.size()
            + self.meshlet_vertices.size()
            + self.meshlet_triangles.size()
            + self.mesh_bounds.size()
    }

    /// Bind-group layout: 5 storage buffers, all read-only, all visible to compute + vertex stages
    /// so the same bind group can drive cull + vbuf rasterization once the pool variant of those
    /// shaders lands.
    pub fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        let read_only = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("global_pool_bgl"),
            entries: &[
                entry(0, read_only),
                entry(1, read_only),
                entry(2, read_only),
                entry(3, read_only),
                entry(4, read_only),
            ],
        })
    }

    pub fn bind_group(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("global_pool_bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.mesh_descriptors.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.meshlets.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.vertices.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.meshlet_vertices.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.meshlet_triangles.as_entire_binding(),
                },
            ],
        })
    }
}

fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::VERTEX,
        ty,
        count: None,
    }
}

/// Pads an empty slice to a single zero record so wgpu storage bindings stay non-empty.
fn padded<T: Pod>(slice: &[T]) -> Vec<u8> {
    if slice.is_empty() {
        vec![0u8; std::mem::size_of::<T>().max(4)]
    } else {
        bytemuck::cast_slice(slice).to_vec()
    }
}
