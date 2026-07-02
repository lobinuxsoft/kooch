//! GPU-resident meshlet mesh — storage buffers + bind group layout for
//! the upcoming compute culling shader.
//!
//! Splitting CPU [`MeshletMesh`] from GPU [`GpuMeshletMesh`] mirrors the
//! pattern we use for `Mesh` / `GpuMesh`: same asset can sit in CPU
//! tools (mesh viewer, exporter) and be uploaded to wgpu when the
//! renderer needs it.
//!
//! # Layout
//!
//! Four storage buffers, each `bytemuck::cast_slice`-uploaded:
//!
//! | Buffer | Contents | Stride |
//! |---|---|---|
//! | `vertices` | `Vec<MeshVertex>` (the shared vertex pool) | 32 B |
//! | `meshlet_vertices` | `Vec<u32>` (per-meshlet indices into `vertices`) | 4 B |
//! | `meshlet_triangles` | `Vec<u8>` (3-byte triangles, packed contiguously) | 1 B |
//! | `descriptors` | `Vec<MeshletDescriptor>` (per-meshlet metadata) | 80 B |

use bytemuck::Zeroable;
use wgpu::util::DeviceExt;

use crate::mesh::MeshVertex;

use super::asset::{MeshletDescriptor, MeshletMesh};

/// GPU-resident counterpart of [`MeshletMesh`]. Owns four
/// `wgpu::Buffer`s; dropping it releases all four.
///
/// Bind group layout is `bind_group_layout()`; buffers are bound at
/// slots 0..3 via [`bind_group`](Self::bind_group).
pub struct GpuMeshletMesh {
    pub vertices: wgpu::Buffer,
    pub meshlet_vertices: wgpu::Buffer,
    pub meshlet_triangles: wgpu::Buffer,
    pub descriptors: wgpu::Buffer,
    /// Number of meshlets — used for the compute culling dispatch
    /// (one workgroup per meshlet typically).
    pub meshlet_count: u32,
    /// Vertex count in the shared pool.
    pub vertex_count: u32,
    /// Total triangle count summed across every meshlet.
    pub triangle_count: u32,
}

impl GpuMeshletMesh {
    /// Bytes used by the vertex stream.
    pub fn vertex_bytes(&self) -> u64 {
        self.vertex_count as u64 * std::mem::size_of::<MeshVertex>() as u64
    }

    /// Bytes used by the descriptor stream.
    pub fn descriptor_bytes(&self) -> u64 {
        self.meshlet_count as u64 * MeshletDescriptor::SIZE as u64
    }
}

impl MeshletMesh {
    /// Uploads the meshlet mesh to GPU as four storage buffers
    /// (`STORAGE | COPY_DST`). The caller binds them via the layout
    /// from [`meshlet_bind_group_layout`].
    pub fn upload(&self, device: &wgpu::Device) -> GpuMeshletMesh {
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_vertex_pool"),
            contents: bytemuck::cast_slice(&self.vertices),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let meshlet_vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_vertices"),
            contents: bytemuck::cast_slice(&self.meshlet_vertices),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let padded_triangles = pad_to_4(&self.meshlet_triangles);
        let meshlet_triangles = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_triangles"),
            contents: &padded_triangles,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        // wgpu storage buffers must align to 4 bytes; the triangle
        // stream may have an odd byte count (3 bytes per triangle), so
        // we pad to the next 4-byte boundary.
        let descriptors = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_descriptors"),
            contents: bytemuck::cast_slice(&self.meshlets),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        GpuMeshletMesh {
            vertices,
            meshlet_vertices,
            meshlet_triangles,
            descriptors,
            meshlet_count: self.meshlet_count(),
            vertex_count: self.total_vertex_count(),
            triangle_count: self.total_triangle_count(),
        }
    }
}

fn pad_to_4(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    while out.len() % 4 != 0 {
        out.push(0);
    }
    out
}

/// Bind group layout slot indices.
pub mod binding {
    pub const VERTICES: u32 = 0;
    pub const MESHLET_VERTICES: u32 = 1;
    pub const MESHLET_TRIANGLES: u32 = 2;
    pub const DESCRIPTORS: u32 = 3;
}

/// Builds the bind group layout that future compute / mesh shaders
/// consume. Visibility set to compute + vertex so both culling and
/// rasterization stages can read the same layout.
pub fn meshlet_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let read_only = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: true },
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("meshlet_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: binding::VERTICES,
                visibility: wgpu::ShaderStages::COMPUTE
                    | wgpu::ShaderStages::VERTEX
                    | wgpu::ShaderStages::FRAGMENT,
                ty: read_only,
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: binding::MESHLET_VERTICES,
                visibility: wgpu::ShaderStages::COMPUTE
                    | wgpu::ShaderStages::VERTEX
                    | wgpu::ShaderStages::FRAGMENT,
                ty: read_only,
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: binding::MESHLET_TRIANGLES,
                visibility: wgpu::ShaderStages::COMPUTE
                    | wgpu::ShaderStages::VERTEX
                    | wgpu::ShaderStages::FRAGMENT,
                ty: read_only,
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: binding::DESCRIPTORS,
                visibility: wgpu::ShaderStages::COMPUTE
                    | wgpu::ShaderStages::VERTEX
                    | wgpu::ShaderStages::FRAGMENT,
                ty: read_only,
                count: None,
            },
        ],
    })
}

/// Helper that constructs a bind group from a [`GpuMeshletMesh`] using
/// the layout from [`meshlet_bind_group_layout`].
pub fn meshlet_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    mesh: &GpuMeshletMesh,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("meshlet_bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: binding::VERTICES,
                resource: mesh.vertices.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: binding::MESHLET_VERTICES,
                resource: mesh.meshlet_vertices.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: binding::MESHLET_TRIANGLES,
                resource: mesh.meshlet_triangles.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: binding::DESCRIPTORS,
                resource: mesh.descriptors.as_entire_binding(),
            },
        ],
    })
}

/// Builds the meshlet bind group from the multi-mesh
/// [`super::pool::GpuGlobalMeshPool`] using the same
/// [`meshlet_bind_group_layout`]. Per-meshlet `vertex_offset` and
/// `triangle_offset` were already rebased into pool-global coordinates
/// by [`super::pool::GlobalMeshPool::register`], so the rasterizer +
/// deferred shaders need no per-mesh remapping — they index the
/// concatenated buffers as if every meshlet lived in one giant mesh.
pub fn pool_meshlet_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    pool: &super::pool::GpuGlobalMeshPool,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pool_meshlet_bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: binding::VERTICES,
                resource: pool.vertices.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: binding::MESHLET_VERTICES,
                resource: pool.meshlet_vertices.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: binding::MESHLET_TRIANGLES,
                resource: pool.meshlet_triangles.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: binding::DESCRIPTORS,
                resource: pool.meshlets.as_entire_binding(),
            },
        ],
    })
}

/// Used by tests + descriptor placeholder construction.
pub fn zeroed_descriptor() -> MeshletDescriptor {
    MeshletDescriptor::zeroed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meshlet::asset::DEFAULT_MAX_VERTICES;

    #[test]
    fn pad_to_4_is_idempotent_when_already_aligned() {
        assert_eq!(pad_to_4(&[1, 2, 3, 4]), vec![1, 2, 3, 4]);
        assert_eq!(pad_to_4(&[]), Vec::<u8>::new());
    }

    #[test]
    fn pad_to_4_rounds_up() {
        assert_eq!(pad_to_4(&[1, 2, 3]), vec![1, 2, 3, 0]);
        assert_eq!(pad_to_4(&[1, 2, 3, 4, 5]), vec![1, 2, 3, 4, 5, 0, 0, 0]);
    }

    #[test]
    fn binding_slot_constants_are_distinct() {
        let slots = [
            binding::VERTICES,
            binding::MESHLET_VERTICES,
            binding::MESHLET_TRIANGLES,
            binding::DESCRIPTORS,
        ];
        let mut sorted = slots.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), slots.len(), "binding slots collided");
    }

    #[test]
    fn descriptor_size_matches_max_vertex_constant_at_compile_time() {
        // Defensive — DEFAULT_MAX_VERTICES is referenced by the
        // builder; confirm it stays within u8 since meshlet-local
        // triangle indices are u8 (0..max_vertices-1 fits when
        // max_vertices <= 256).
        assert!(DEFAULT_MAX_VERTICES <= 256);
    }

    #[test]
    fn zeroed_descriptor_is_safe_default() {
        let d = zeroed_descriptor();
        assert_eq!(d.vertex_count, 0);
        assert_eq!(d.triangle_count, 0);
        assert_eq!(d.bounding_radius, 0.0);
    }
}
