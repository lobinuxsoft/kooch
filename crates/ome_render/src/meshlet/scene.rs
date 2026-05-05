//! Scene-wide GPU-driven meshlet pipeline state.
//!
//! Phase 1.E.1: introduces the `MeshInstance` POD and the storage buffer
//! the scene-wide cull dispatch enumerates. Single-mesh multi-instance
//! for now; the global mesh pool that lets one cull dispatch enumerate
//! meshlets across *different* meshes lands in 1.E.1b.
//!
//! # Why this exists (post Phase 1.D close audit)
//!
//! Phase 1.D delivered per-meshlet primitives (cull, vbuf, deferred,
//! materials) but every test in `tests/` rasterizes a single
//! `GpuMeshletMesh` per dispatch. That path is correct DOD-shape but
//! NOT GPU-driven in spirit: the CPU still enumerates meshes one at a
//! time. `feedback_gpu_driven_spirit.md` and the project's planet-
//! scale + GPU-driven constraints demand: hot loop on GPU, scene
//! enumeration on GPU, indirect dispatch fed by compute. This module
//! is the foundation.

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

/// Sentinel value for [`MeshInstance::lod_force_level`] meaning
/// "no force — let the normal LOD selector decide". Stored as
/// `i32::MIN` so any sensible level (positive small int) cannot
/// collide with it.
pub const LOD_FORCE_NONE: i32 = i32::MIN;

/// Per-instance scene record consumed by `cs_cull_scene`.
///
/// Layout (80 B, multiple of 16):
/// - `transform` (mat4, 64 B): world-space transform of this instance.
/// - `mesh_id` (u32): index into the global mesh pool (stub for 1.E.1b
///   — single-mesh path ignores this).
/// - `material_id` (u32): material pool index this instance shades against.
/// - `lod_bias` (f32): per-instance LOD bias for the screen-space-error
///   selector (1.E follow-up).
/// - `lod_force_level` (i32): when ≥ 0, the cull short-circuits the
///   LOD selector and emits only meshlets whose `lod_level` matches
///   this value. [`LOD_FORCE_NONE`] = normal selector. Drives the
///   side-by-side LOD inspector (#467).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MeshInstance {
    pub transform: [[f32; 4]; 4],
    pub mesh_id: u32,
    pub material_id: u32,
    pub lod_bias: f32,
    pub lod_force_level: i32,
}

impl MeshInstance {
    pub fn new(transform: Mat4, mesh_id: u32, material_id: u32) -> Self {
        Self {
            transform: transform.to_cols_array_2d(),
            mesh_id,
            material_id,
            lod_bias: 0.0,
            lod_force_level: LOD_FORCE_NONE,
        }
    }

    /// Convenience: instance with the LOD selector overridden to
    /// emit only meshlets at `level`. Used by the editor's side-by-
    /// side LOD inspector to render each chain layer in isolation.
    pub fn with_lod_force_level(mut self, level: i32) -> Self {
        self.lod_force_level = level;
        self
    }

    pub fn transform_mat4(&self) -> Mat4 {
        Mat4::from_cols_array_2d(&self.transform)
    }
}

impl Default for MeshInstance {
    fn default() -> Self {
        Self::new(Mat4::IDENTITY, 0, 0)
    }
}

/// Per-frame scene parameters consumed by `cs_cull_scene` — the
/// per-meshlet `CullParams` already carries the camera state; this
/// adds the instance-count + per-mesh meshlet-count needed for the
/// 1D thread → (instance, meshlet) decoding.
///
/// Layout (16 B):
/// - `instance_count`: number of valid `MeshInstance` slots in the buffer.
/// - `meshlets_per_mesh`: meshlet count of the (single) mesh registered
///   for this scene. 1.E.1b replaces this with a per-mesh-id lookup.
/// - `_pad0`, `_pad1`: keep 16-byte alignment.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub struct SceneCullParams {
    pub instance_count: u32,
    pub meshlets_per_mesh: u32,
    pub _pad0: u32,
    pub _pad1: u32,
}

impl SceneCullParams {
    pub fn new(instance_count: u32, meshlets_per_mesh: u32) -> Self {
        Self {
            instance_count,
            meshlets_per_mesh,
            _pad0: 0,
            _pad1: 0,
        }
    }
}

/// Owns the scene-wide instance storage buffer + an upload helper.
/// Capacity is fixed at construction; growth re-creates the buffer
/// (cheap relative to the per-frame upload).
pub struct MeshletScene {
    instance_buffer: wgpu::Buffer,
    capacity: u32,
    bgl: wgpu::BindGroupLayout,
}

impl MeshletScene {
    /// Allocates an instance buffer sized for `capacity` slots.
    pub fn new(device: &wgpu::Device, capacity: u32) -> Self {
        assert!(capacity > 0, "MeshletScene capacity must be non-zero");
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_scene_instances"),
            size: capacity as u64 * std::mem::size_of::<MeshInstance>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = Self::bind_group_layout(device);
        Self {
            instance_buffer,
            capacity,
            bgl,
        }
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn instance_buffer(&self) -> &wgpu::Buffer {
        &self.instance_buffer
    }

    /// Bind group layout for `cs_cull_scene` group(2): instance buffer +
    /// `SceneCullParams` UBO.
    pub fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_scene_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(
                            std::mem::size_of::<SceneCullParams>() as u64,
                        ),
                    },
                    count: None,
                },
            ],
        })
    }

    /// Cached layout for the dispatcher's pipeline-layout building.
    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.bgl
    }

    /// Uploads `instances[..]` into the GPU buffer (offset 0). Caller
    /// is responsible for keeping `instances.len() <= capacity`.
    pub fn upload_instances(&self, queue: &wgpu::Queue, instances: &[MeshInstance]) {
        assert!(
            instances.len() as u32 <= self.capacity,
            "instance count {} exceeds scene capacity {}",
            instances.len(),
            self.capacity,
        );
        if instances.is_empty() {
            return;
        }
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(instances));
    }
}

/// Decodes the packed `(instance_id, meshlet_id)` value the scene cull
/// shader writes into `visible_meshlets`. CPU mirror of the WGSL
/// extract logic so tests can verify expected pairs without
/// reimplementing the bit math.
pub fn decode_scene_visible_id(packed: u32) -> (u32, u32) {
    // bit 16..32 = instance_id, bit 0..16 = meshlet_id
    (packed >> 16, packed & 0xFFFF)
}

/// Inverse of [`decode_scene_visible_id`]. Both must be < 0x1_0000.
pub fn encode_scene_visible_id(instance_id: u32, meshlet_id: u32) -> u32 {
    debug_assert!(instance_id < (1 << 16));
    debug_assert!(meshlet_id < (1 << 16));
    (instance_id << 16) | meshlet_id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_instance_layout_is_pod_80_bytes() {
        assert_eq!(std::mem::size_of::<MeshInstance>(), 80);
    }

    #[test]
    fn scene_cull_params_layout_is_pod_16_bytes() {
        assert_eq!(std::mem::size_of::<SceneCullParams>(), 16);
    }

    #[test]
    fn mesh_instance_round_trip_transform() {
        let m = Mat4::from_translation(glam::Vec3::new(1.0, 2.0, 3.0));
        let inst = MeshInstance::new(m, 7, 11);
        let recovered = inst.transform_mat4();
        // Compare every column to avoid a float-eq trap on Mat4.
        for col in 0..4 {
            for row in 0..4 {
                assert_eq!(
                    inst.transform[col][row],
                    recovered.col(col)[row]
                );
            }
        }
        assert_eq!(inst.mesh_id, 7);
        assert_eq!(inst.material_id, 11);
    }

    #[test]
    fn encode_decode_round_trip() {
        for instance_id in [0u32, 1, 42, 0xFFFF] {
            for meshlet_id in [0u32, 1, 100, 0xFFFF] {
                let packed = encode_scene_visible_id(instance_id, meshlet_id);
                assert_eq!(decode_scene_visible_id(packed), (instance_id, meshlet_id));
            }
        }
    }

    #[test]
    fn decode_extracts_high_low_halves() {
        let packed = (5u32 << 16) | 12u32;
        assert_eq!(decode_scene_visible_id(packed), (5, 12));
    }
}
