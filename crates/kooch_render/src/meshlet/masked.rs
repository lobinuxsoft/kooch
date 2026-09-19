//! Masked materials (#452): a material whose shader assigns `alpha_clip` rasterises in a bin of its
//! own, where its `surface` decides which fragments reach the visibility buffer. Opaque meshlets keep
//! the one material-less draw, which skips masked instances; Nanite bins its programmable rasters
//! the same way.

mod bins;
mod layouts;

use bytemuck::{Pod, Zeroable, bytes_of};

use crate::material::{MaterialPipeline, ShaderKind};
use crate::meshlet::scene::{
    INSTANCE_MASKED, INSTANCE_TRANSPARENT, INSTANCE_TRIMMED, MeshInstance,
};
use crate::meshlet::{MATERIAL_SURFACE_PRELUDE, SURFACE_RECONSTRUCT_SHADER, ShaderPipelines};

pub use bins::MaskedBins;
use layouts::Layouts;

/// Masked materials a frame rasterises by alpha; past this they draw solid.
pub const MASKED_BINS: u32 = 32;
/// Material slots the bin table covers, as the material pool holds.
const MATERIAL_SLOTS: usize = 256;

const RASTER: &str = include_str!("../../shaders/masked_raster.wgsl");
const RASTER_R64: &str = include_str!("../../shaders/masked_raster_r64.wgsl");
const RASTER_R32: &str = include_str!("../../shaders/masked_raster_r32.wgsl");

/// Which visibility buffer the raster writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskedTarget {
    /// The atomic R64 buffer, through a storage binding.
    R64,
    /// The legacy `R32Uint` colour target.
    R32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Pod, Zeroable)]
struct Screen {
    size: [u32; 2],
    material_id: u32,
    bin: u32,
    mip_bias_scale: f32,
    time: f32,
    _pad: [u32; 2],
}

/// What a view's raster needs of the frame, written before it draws.
pub struct MaskedFrame {
    pub view_proj: glam::Mat4,
    pub camera_position: glam::Vec3,
    pub size: (u32, u32),
    pub mip_bias_scale: f32,
    pub time: f32,
}

/// What a raster pass needs to draw the masked bins after its opaque draw.
pub struct MaskedDraw<'a> {
    pub raster: &'a MaskedRaster,
    pub bins: &'a MaskedBins,
    pub materials: &'a MaterialPipeline,
}

impl MaskedDraw<'_> {
    /// See [`MaskedRaster::draw`].
    pub fn draw(
        &self,
        device: &wgpu::Device,
        pass: &mut wgpu::RenderPass<'_>,
        meshlet_bg: &wgpu::BindGroup,
        cull: &crate::meshlet::dispatcher::MeshletCull,
        scene: &crate::meshlet::scene::MeshletScene,
        vbuf64: Option<&wgpu::TextureView>,
    ) {
        let materials = self.materials;
        self.raster.draw(
            device, pass, self.bins, materials, meshlet_bg, cull, scene, vbuf64,
        );
    }
}

/// The masked bins' pipelines, their table and the frame they draw in. Shared by every view; each
/// view bins its own cull into its [`MaskedBins`].
pub struct MaskedRaster {
    target: MaskedTarget,
    depth_format: wgpu::TextureFormat,
    layouts: Layouts,
    pipelines: ShaderPipelines<wgpu::RenderPipeline>,
    table: wgpu::Buffer,
    camera: wgpu::Buffer,
    inti: wgpu::Buffer,
    screen: wgpu::Buffer,
    screen_stride: u64,
    /// This frame's bins, in order: the material slot each draws, and its pipeline.
    bins: Vec<(u32, wgpu::RenderPipeline)>,
}

impl MaskedRaster {
    pub fn new(
        device: &wgpu::Device,
        target: MaskedTarget,
        meshlet_bgl: &wgpu::BindGroupLayout,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let uniform = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        let screen_stride = (std::mem::size_of::<Screen>() as u64).next_multiple_of(align);
        Self {
            target,
            depth_format,
            layouts: Layouts::new(device, target, meshlet_bgl),
            pipelines: ShaderPipelines::new(&[ShaderKind::Surface, ShaderKind::Unlit]),
            table: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("masked_bin_table"),
                size: (MATERIAL_SLOTS * 4) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            camera: uniform("masked_camera", 64),
            inti: uniform("masked_inti", 16),
            screen: uniform("masked_screen", screen_stride * MASKED_BINS as u64),
            screen_stride,
            bins: Vec::new(),
        }
    }

    /// Gives each masked material in `instances` a bin, and flags its instances so the opaque draw
    /// leaves them out. A material whose shader never compiled, or past [`MASKED_BINS`], keeps no
    /// bin and draws solid: never both skipped and undrawn.
    pub fn assign(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        materials: Option<&MaterialPipeline>,
        instances: &mut [MeshInstance],
    ) {
        let mut table = [0u32; MATERIAL_SLOTS];
        let mut refused = [false; MATERIAL_SLOTS];
        self.bins.clear();
        for instance in instances.iter_mut() {
            instance.flags &= !INSTANCE_MASKED;
            let slot = instance.material_id as usize;
            // A trimmed instance carries the cut in its geometry (#452): it draws opaque.
            let drawn = INSTANCE_TRANSPARENT | INSTANCE_TRIMMED;
            if instance.flags & drawn != 0 || slot >= MATERIAL_SLOTS || refused[slot] {
                continue;
            }
            if table[slot] == 0 {
                let pipeline = (self.bins.len() < MASKED_BINS as usize)
                    .then_some(materials)
                    .flatten()
                    .and_then(|m| self.pipeline(device, m, slot as u32));
                let Some(pipeline) = pipeline else {
                    refused[slot] = true;
                    continue;
                };
                self.bins.push((slot as u32, pipeline));
                table[slot] = self.bins.len() as u32;
            }
            instance.flags |= INSTANCE_MASKED;
        }
        queue.write_buffer(&self.table, 0, bytemuck::cast_slice(&table));
    }

    /// Whether any material rasterises masked this frame.
    pub fn any(&self) -> bool {
        !self.bins.is_empty()
    }

    /// Writes what the bins draw with. Before the view's raster, after [`Self::assign`].
    pub fn frame(&self, queue: &wgpu::Queue, frame: &MaskedFrame) {
        if !self.any() {
            return;
        }
        queue.write_buffer(
            &self.camera,
            0,
            bytes_of(&frame.view_proj.to_cols_array_2d()),
        );
        let position = frame.camera_position;
        queue.write_buffer(
            &self.inti,
            0,
            bytemuck::cast_slice(&[position.x, position.y, position.z, 0.0]),
        );
        let mut bytes = vec![0u8; (self.screen_stride * self.bins.len() as u64) as usize];
        for (bin, (slot, _)) in self.bins.iter().enumerate() {
            let at = bin * self.screen_stride as usize;
            let screen = Screen {
                size: [frame.size.0, frame.size.1],
                material_id: *slot,
                bin: bin as u32,
                mip_bias_scale: frame.mip_bias_scale,
                time: frame.time,
                _pad: [0; 2],
            };
            bytes[at..at + std::mem::size_of::<Screen>()].copy_from_slice(bytes_of(&screen));
        }
        queue.write_buffer(&self.screen, 0, &bytes);
    }

    /// The pipeline for `slot`'s shader, when it is masked and compiles.
    fn pipeline(
        &self,
        device: &wgpu::Device,
        materials: &MaterialPipeline,
        slot: u32,
    ) -> Option<wgpu::RenderPipeline> {
        let (guid, surface) = materials.slot_surface(slot)?;
        if !surface.masked {
            return None;
        }
        self.pipelines.get(guid, surface, false, |surface| {
            let source = compose_masked_shader(self.target, &surface.params_wgsl, &surface.source);
            layouts::pipeline(
                device,
                &self.layouts,
                self.target,
                self.depth_format,
                &source,
            )
        })
    }
}

/// A masked shader's raster: the reconstruction the shading pass uses, the raster frame, the
/// contract, the shader, then the target's write. Stands in for a WGSL `#import`.
pub fn compose_masked_shader(target: MaskedTarget, params: &str, surface: &str) -> String {
    let tail = match target {
        MaskedTarget::R64 => RASTER_R64,
        MaskedTarget::R32 => RASTER_R32,
    };
    [
        SURFACE_RECONSTRUCT_SHADER,
        RASTER,
        MATERIAL_SURFACE_PRELUDE,
        params,
        surface,
        tail,
    ]
    .join("\n")
}

#[cfg(test)]
mod tests;
