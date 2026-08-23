//! GPU-side mirror of [`super::GlobalMeshPool`] — five storage buffers
//! the cull / vbuf / deferred shaders read through one bind group.

use bytemuck::Pod;
use wgpu::util::DeviceExt;

use super::GlobalMeshPool;

impl GlobalMeshPool {
    /// Uploads the current CPU state to GPU buffers. Call after a
    /// batch of [`Self::register`] invocations and before the next
    /// frame's scene cull.
    pub fn upload(&self, device: &wgpu::Device) -> GpuGlobalMeshPool {
        let mesh_descriptors = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("global_pool_mesh_descriptors"),
            contents: &pad_bytes(&self.mesh_descriptors),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let meshlets = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("global_pool_meshlets"),
            contents: &pad_bytes(&self.meshlets),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("global_pool_vertices"),
            contents: &pad_bytes(&self.vertices),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let meshlet_vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("global_pool_meshlet_vertices"),
            contents: &pad_bytes(&self.meshlet_vertices),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let triangle_bytes = if self.meshlet_triangles.is_empty() {
            vec![0u8; 4]
        } else {
            let mut tris = self.meshlet_triangles.clone();
            while tris.len() % 4 != 0 {
                tris.push(0);
            }
            tris
        };
        let meshlet_triangles = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("global_pool_meshlet_triangles"),
            contents: &triangle_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        // One vec4 per mesh: bounds sphere as (center, radius). Uploaded
        // for the lamp cull's light/instance pre-pass (#939); parallel
        // to `mesh_descriptors` the way the CPU vec is (#847).
        let bounds: Vec<[f32; 4]> = self
            .mesh_bounds
            .iter()
            .map(|b| [b.center.x, b.center.y, b.center.z, b.radius])
            .collect();
        let mesh_bounds = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("global_pool_mesh_bounds"),
            contents: &pad_bytes(&bounds),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        GpuGlobalMeshPool {
            mesh_descriptors,
            meshlets,
            vertices,
            meshlet_vertices,
            meshlet_triangles,
            mesh_bounds,
            mesh_count: self.mesh_count(),
            max_meshlets_per_mesh: self.max_meshlets_per_mesh(),
        }
    }
}

/// GPU-resident pool. Bound by the cull shader (mesh-descriptor lookups
/// to redirect per-meshlet reads) and the vbuf rasterizer (vertex pull
/// through the same arrays).
pub struct GpuGlobalMeshPool {
    pub mesh_descriptors: wgpu::Buffer,
    pub meshlets: wgpu::Buffer,
    pub vertices: wgpu::Buffer,
    pub meshlet_vertices: wgpu::Buffer,
    pub meshlet_triangles: wgpu::Buffer,
    /// Per-mesh bounds sphere as `(center, radius)` vec4s, parallel to
    /// `mesh_descriptors`. Not part of the shared 5-entry pool layout —
    /// only the lamp cull (#939) binds it, in its own layout, so no
    /// existing shader mirror moves.
    pub mesh_bounds: wgpu::Buffer,
    pub mesh_count: u32,
    pub max_meshlets_per_mesh: u32,
}

impl GpuGlobalMeshPool {
    /// Bind-group layout: 5 storage buffers, all read-only, all
    /// visible to compute + vertex stages so the same bind group can
    /// drive cull + vbuf rasterization once the pool variant of those
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

/// Pads an empty slice to a single zero record so wgpu storage
/// bindings stay non-empty. Helper kept here because it's only used
/// by the GPU upload path.
fn pad_bytes<T: Pod>(slice: &[T]) -> Vec<u8> {
    if slice.is_empty() {
        vec![0u8; std::mem::size_of::<T>().max(4)]
    } else {
        bytemuck::cast_slice(slice).to_vec()
    }
}
