//! PBR material parameters + GPU pool + typed [`Material`] asset.

mod asset;
mod pipeline;
pub mod shader;
mod texture_pool;
mod values;

/// The engine's surface, and what a new `.shader` starts as.
pub use crate::meshlet::{DEFAULT_SURFACE_SHADER, NEW_SURFACE_SHADER};
pub use asset::{MATERIAL_EXTENSION, Material, MaterialLoader, MaterialParseError};
pub use pipeline::{
    DEFAULT_CAPACITY as MATERIAL_POOL_DEFAULT_CAPACITY, FALLBACK_MATERIAL_ID, MATERIAL_TYPE_NAME,
    MaterialPipeline, SurfaceSource, TextureReimports,
};
pub use shader::{
    MAX_PARAM_SCALARS, MAX_PARAM_TEXTURES, ParamKind, SHADER_EXTENSION, SHADER_TYPE_NAME, Shader,
    ShaderKind, ShaderLoader, ShaderParam, TextureDefault, masks,
};
pub use texture_pool::MaterialTexturePool;
pub use values::{PackedParams, ParamValue, ParamValues, TextureRef, retain_declared};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

/// Sentinel written to a `texture_indices` slot when the material has
/// no map for that channel. The shader tests against this to decide
/// whether to sample or fall back to the scalar coefficient.
pub const NO_TEXTURE: u32 = u32::MAX;

/// PBR scalar parameters for a single material slot.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MaterialParams {
    pub base_color: [f32; 4],
    pub metallic_roughness_emissive_pad: [f32; 4],
    pub texture_indices: [u32; 4],
    pub uv_scale_offset: [f32; 4],
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
            uv_scale_offset: [1.0, 1.0, 0.0, 0.0],
        }
    }

    /// Sets the texture transform: `scale` tiles, `offset` slides.
    pub fn with_uv(mut self, scale: [f32; 2], offset: [f32; 2]) -> Self {
        self.uv_scale_offset = [scale[0], scale[1], offset[0], offset[1]];
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

    #[cfg(test)]
    pub fn albedo_index(&self) -> u32 {
        self.texture_indices[0]
    }

    #[cfg(test)]
    pub fn normal_index(&self) -> u32 {
        self.texture_indices[1]
    }

    #[cfg(test)]
    pub fn metal_roughness_index(&self) -> u32 {
        self.texture_indices[2]
    }
}

/// GPU-resident pool of [`MaterialParams`]. Caller indexes into it via the material id baked into
/// the per-meshlet rendering call (PR-7 keeps material assignment per-render call; per-meshlet
/// assignment lands with bindless).
pub struct MaterialPool {
    buffer: wgpu::Buffer,
    /// Each slot's shader values, `MAX_PARAM_SCALARS` `f32`s per slot (#1158).
    values: wgpu::Buffer,
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
        let values = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("material_values"),
            size: materials.len() as u64 * u64::from(MAX_PARAM_SCALARS) * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            values,
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

    /// The shader values, bound beside [`Self::buffer`].
    pub fn values(&self) -> &wgpu::Buffer {
        &self.values
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

    /// Updates a slot's shader values.
    pub fn write_values(
        &self,
        queue: &wgpu::Queue,
        slot: u32,
        values: &[f32; MAX_PARAM_SCALARS as usize],
    ) {
        let offset = u64::from(slot) * u64::from(MAX_PARAM_SCALARS) * 4;
        queue.write_buffer(&self.values, offset, bytemuck::cast_slice(values));
    }
}

#[cfg(test)]
mod tests;
