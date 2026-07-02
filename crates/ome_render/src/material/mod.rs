//! PBR material parameters + GPU pool + typed [`Material`] asset.
//!
//! Three layers stack here:
//! - [`MaterialParams`] — the 32-byte POD the GPU shader reads from
//!   the `MaterialPool` storage buffer. Indexed by `material_id`.
//! - [`Material`] — the CPU-side asset users author / pick from the
//!   inspector. RON-serializable; survives in `Assets<Material>` and
//!   is referenced by GUID from `MeshRenderer.material`.
//! - [`MaterialPool`] — the GPU buffer. The runtime sync system keeps
//!   it in lockstep with `Assets<Material>`; the deferred shader
//!   reads `materials[inst.material_id].base_color` and modulates
//!   the normal-debug shading by it.
//!
//! Texture arrays / true bindless sampling lands in the Phase 1 follow-
//! up alongside #130 PBR shading, when wgpu's [`Features::
//! TEXTURE_BINDING_ARRAY`] + non-uniform indexing become a hard
//! requirement.

mod asset;
mod pipeline;

pub use asset::{Material, MaterialLoader, MaterialParseError};
pub use pipeline::{
    DEFAULT_CAPACITY as MATERIAL_POOL_DEFAULT_CAPACITY, FALLBACK_MATERIAL_ID, MATERIAL_TYPE_NAME,
    MaterialPipeline,
};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

/// Sentinel written to a `texture_indices` slot when the material has
/// no map for that channel. The shader tests against this to decide
/// whether to sample or fall back to the scalar coefficient.
pub const NO_TEXTURE: u32 = u32::MAX;

/// PBR scalar parameters for a single material slot.
///
/// Layout (48 B, multiple of 16 for std140):
/// - `base_color` (vec4): RGB albedo + alpha (linear-space).
/// - `metallic_roughness_emissive_pad` (vec4): metallic, roughness,
///   emissive intensity, _pad. Packed together so the struct stays
///   16-byte aligned for the storage-buffer stride.
/// - `texture_indices` (uvec4): albedo, normal, metal_roughness pool
///   indices + _pad. [`NO_TEXTURE`] means "no map — use the scalar".
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MaterialParams {
    pub base_color: [f32; 4],
    pub metallic_roughness_emissive_pad: [f32; 4],
    pub texture_indices: [u32; 4],
}

impl Default for MaterialParams {
    fn default() -> Self {
        Self::new([1.0, 1.0, 1.0, 1.0], 0.0, 0.5, 0.0)
    }
}

impl MaterialParams {
    pub fn new(base_color: [f32; 4], metallic: f32, roughness: f32, emissive: f32) -> Self {
        Self {
            base_color,
            metallic_roughness_emissive_pad: [metallic, roughness, emissive, 0.0],
            texture_indices: [NO_TEXTURE; 4],
        }
    }

    /// Assigns the resolved pool indices for the three texture channels.
    /// Pass [`NO_TEXTURE`] for any channel without a map.
    pub fn with_texture_indices(mut self, albedo: u32, normal: u32, metal_roughness: u32) -> Self {
        self.texture_indices = [albedo, normal, metal_roughness, 0];
        self
    }

    pub fn base_color(&self) -> [f32; 4] {
        self.base_color
    }

    pub fn metallic(&self) -> f32 {
        self.metallic_roughness_emissive_pad[0]
    }

    pub fn roughness(&self) -> f32 {
        self.metallic_roughness_emissive_pad[1]
    }

    pub fn emissive(&self) -> f32 {
        self.metallic_roughness_emissive_pad[2]
    }

    pub fn albedo_index(&self) -> u32 {
        self.texture_indices[0]
    }

    pub fn normal_index(&self) -> u32 {
        self.texture_indices[1]
    }

    pub fn metal_roughness_index(&self) -> u32 {
        self.texture_indices[2]
    }
}

/// GPU-resident pool of [`MaterialParams`]. Caller indexes into it via
/// the material id baked into the per-meshlet rendering call (PR-7 keeps
/// material assignment per-render call; per-meshlet assignment lands
/// with bindless).
pub struct MaterialPool {
    buffer: wgpu::Buffer,
    capacity: u32,
    bgl: wgpu::BindGroupLayout,
}

impl MaterialPool {
    /// Builds a pool sized for `materials.len()` slots and uploads the
    /// initial values. `materials` must be non-empty — wgpu rejects
    /// zero-sized storage buffer bindings.
    pub fn new(device: &wgpu::Device, materials: &[MaterialParams]) -> Self {
        assert!(
            !materials.is_empty(),
            "MaterialPool requires at least one material"
        );
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("material_pool"),
            contents: bytemuck::cast_slice(materials),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let bgl = Self::bind_group_layout(device);
        Self {
            buffer,
            capacity: materials.len() as u32,
            bgl,
        }
    }

    /// Bind group layout: one storage buffer at binding(0). Used in
    /// the deferred shader's group(2).
    pub fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material_pool_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        })
    }

    pub fn bind_group(&self, device: &wgpu::Device) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material_pool_bg"),
            layout: &self.bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.buffer.as_entire_binding(),
            }],
        })
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.bgl
    }

    /// Updates a slot in place. Useful for live-edit; PR-7 itself does
    /// not call this, but the deferred shader needs the latest state
    /// after a future hot-reload path.
    pub fn write(&self, queue: &wgpu::Queue, slot: u32, params: &MaterialParams) {
        let offset = slot as u64 * std::mem::size_of::<MaterialParams>() as u64;
        queue.write_buffer(&self.buffer, offset, bytemuck::bytes_of(params));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_params_layout_is_pod_48_bytes() {
        // 16 B base_color + 16 B (metallic, rough, emissive, pad)
        // + 16 B (albedo, normal, metal_rough, pad) = 48 B.
        assert_eq!(std::mem::size_of::<MaterialParams>(), 48);
        assert_eq!(std::mem::align_of::<MaterialParams>(), 4);
    }

    #[test]
    fn default_material_is_white_diffuse_mid_roughness() {
        let m = MaterialParams::default();
        assert_eq!(m.base_color, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(m.metallic(), 0.0);
        assert_eq!(m.roughness(), 0.5);
        assert_eq!(m.emissive(), 0.0);
        // No maps by default — every channel carries the sentinel.
        assert_eq!(m.albedo_index(), NO_TEXTURE);
        assert_eq!(m.normal_index(), NO_TEXTURE);
        assert_eq!(m.metal_roughness_index(), NO_TEXTURE);
    }

    #[test]
    fn texture_indices_round_trip_with_sentinel() {
        let m = MaterialParams::default().with_texture_indices(3, NO_TEXTURE, 7);
        assert_eq!(m.albedo_index(), 3);
        assert_eq!(m.normal_index(), NO_TEXTURE);
        assert_eq!(m.metal_roughness_index(), 7);
        assert_eq!(m.texture_indices[3], 0, "pad slot stays zero");
    }

    #[test]
    fn new_packs_scalars_correctly() {
        let m = MaterialParams::new([0.2, 0.4, 0.8, 1.0], 0.7, 0.3, 1.5);
        assert_eq!(m.base_color(), [0.2, 0.4, 0.8, 1.0]);
        assert_eq!(m.metallic(), 0.7);
        assert_eq!(m.roughness(), 0.3);
        assert_eq!(m.emissive(), 1.5);
    }
}
