//! FSR 3.1, transliterated (#481, and it closes step 6).
//!
//! The engine's second temporal upscaler, and the one #481 named from
//! the start. SGSR 2 got built first because it is small and cheap;
//! this one is neither, and it is here because it has the quality —
//! feature locking, reactivity, an exact disocclusion test — that a
//! two-pass upscaler structurally cannot have.
//!
//! # Where the passes came from
//!
//! `Kits/FidelityFX/upscalers/fsr3/` of the AMD FSR SDK 2.3.0, MIT.
//! The schedule in `ffx_fsr3upscaler.cpp:1166` is seven dispatches;
//! this is four of them, and the three that are missing are named in
//! the header of `fsr3_prepare_reactivity.wgsl` along with what their
//! absence costs.
//!
//! | Ours | Theirs |
//! |---|---|
//! | `prepare_inputs` | `pipelinePrepareInputs` |
//! | `farthest_depth_mip1` | one level of `pipelineLumaPyramid` |
//! | `prepare_reactivity` | `pipelinePrepareReactivity` |
//! | `luma_instability` | `pipelineLumaInstability` |
//! | `accumulate` | `pipelineAccumulate` |
//! | — | `pipelineShadingChangePyramid`, `pipelineShadingChange` |
//! | `sharpen.rs` (already shipped, #876) | `pipelineRCAS` |
//!
//! # It has the same oracle SGSR 2 had
//!
//! 🎯 **At a ratio of 1:1 a temporal upscaler IS a TAA.** Run it
//! un-upscaled against the resolve that already ships, on the same
//! frames, and a port that is wrong shows up as a difference from a
//! known-good image rather than as a vague softness. That is what
//! separates "did I port it correctly" from "does the resolution split
//! work", and it is the answer to the risk #481 wrote down — that a
//! transliteration has nothing to diff against.
//!
//! # Licence
//!
//! MIT, Advanced Micro Devices. The copyright header stays in every
//! ported file and the full text is in `NOTICE`. MIT asks for
//! attribution and nothing else.

mod draw;
mod new;
mod targets;

#[cfg(test)]
mod tests;

use bytemuck::{Pod, Zeroable};

use targets::Targets;

use super::sgsr2::UpscaleInputs;
use crate::meshlet::deferred::HDR_COLOR_FORMAT;

const COMMON_SOURCE: &str = include_str!("../../../shaders/fsr3_common.wgsl");
const PREPARE_INPUTS_SOURCE: &str = include_str!("../../../shaders/fsr3_prepare_inputs.wgsl");
const REDUCE_SOURCE: &str = include_str!("../../../shaders/fsr3_reduce.wgsl");
const REACTIVITY_SOURCE: &str = include_str!("../../../shaders/fsr3_prepare_reactivity.wgsl");
const INSTABILITY_SOURCE: &str = include_str!("../../../shaders/fsr3_luma_instability.wgsl");
const ACCUMULATE_SOURCE: &str = include_str!("../../../shaders/fsr3_accumulate.wgsl");

/// WGSL has no `#include`, so each pass is compiled as the shared half
/// followed by its own. Keeping them as separate files rather than one
/// module with five entry points matters for the bind groups: an entry
/// point may only declare bindings its own pass uses.
fn source(pass: &str) -> String {
    format!("{COMMON_SOURCE}\n{pass}")
}

/// The same, for the one pass that needs half precision.
///
/// 🔴 `enable` is a module-wide directive and must precede every
/// declaration, so putting it in the shared half made all five passes
/// demand `SHADER_F16` — including the four that never write an `f16`.
/// A device without the extension then failed to create a shader it had
/// no reason to care about, which is how two unrelated tests started
/// dying inside `fsr3_prepare_inputs`.
fn source_f16(pass: &str) -> String {
    format!("enable f16;\n{COMMON_SOURCE}\n{pass}")
}

/// FSR's own workgroup size for every pass in this schedule.
const GROUP: u32 = 8;

fn groups(size: (u32, u32)) -> (u32, u32) {
    (size.0.max(1).div_ceil(GROUP), size.1.max(1).div_ceil(GROUP))
}

/// Laid out by hand to 96 bytes so that the WGSL declaration and this
/// one cannot drift: every `vec2` is 8-aligned, the scalars fill the
/// tail, and the padding takes the size to a multiple of 16 because
/// that is what the uniform address space requires.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
struct Fsr3Ubo {
    render_size: [f32; 2],
    output_size: [f32; 2],
    render_size_rcp: [f32; 2],
    output_size_rcp: [f32; 2],
    jitter: [f32; 2],
    prev_jitter: [f32; 2],
    downscale: [f32; 2],
    near: f32,
    exposure: f32,
    reset: f32,
    frame_index: u32,
    delta_pre_exposure: f32,
    jitter_sequence_length: f32,
    debug: u32,
    _pad: [f32; 3],
}

/// Which half of each ping-pong holds the previous frame, and whether
/// there is anything in it. Behind a lock for the same reason the
/// resolve's is: the render chain is on `&self`.
struct History {
    index: usize,
    reset: bool,
    frame_index: u32,
    prev_jitter: glam::Vec2,
    prev_exposure: f32,
    /// The last debug stage this instance logged, so the line below
    /// appears once per change instead of once per frame.
    logged_stage: Option<u32>,
}

const COMPUTE: wgpu::ShaderStages = wgpu::ShaderStages::COMPUTE;

fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn sampled(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// Anything the engine reads through the linear sampler. `Rgba16Float`
/// is filterable; `R32Float` is not, which is why the two are separate
/// helpers rather than one with a flag nobody would read.
fn filterable(binding: u32) -> wgpu::BindGroupLayoutEntry {
    sampled(binding, wgpu::TextureSampleType::Float { filterable: true })
}

fn unfilterable(binding: u32) -> wgpu::BindGroupLayoutEntry {
    sampled(
        binding,
        wgpu::TextureSampleType::Float { filterable: false },
    )
}

fn storage(
    binding: u32,
    format: wgpu::TextureFormat,
    access: wgpu::StorageTextureAccess,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn write_hdr(binding: u32) -> wgpu::BindGroupLayoutEntry {
    storage(
        binding,
        HDR_COLOR_FORMAT,
        wgpu::StorageTextureAccess::WriteOnly,
    )
}

/// The single-channel intermediates. `R32Float` is four bytes where
/// `Rgba16Float` is eight, and it is filterable because the engine has
/// required `FLOAT32_FILTERABLE` since #370.
fn write_r32(binding: u32) -> wgpu::BindGroupLayoutEntry {
    storage(
        binding,
        wgpu::TextureFormat::R32Float,
        wgpu::StorageTextureAccess::WriteOnly,
    )
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn layout(
    device: &wgpu::Device,
    label: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries,
    })
}

fn pipeline(
    device: &wgpu::Device,
    label: &str,
    module: &wgpu::ShaderModule,
    entry: &str,
    bgl: &wgpu::BindGroupLayout,
) -> wgpu::ComputePipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(bgl)],
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        module,
        entry_point: Some(entry),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}

pub(super) struct Fsr3 {
    prepare_inputs: wgpu::ComputePipeline,
    clear_reconstructed: wgpu::ComputePipeline,
    prepare_inputs_bgl: wgpu::BindGroupLayout,

    farthest_mip1: wgpu::ComputePipeline,
    clear_new_locks: wgpu::ComputePipeline,
    reduce_bgl: wgpu::BindGroupLayout,

    reactivity: wgpu::ComputePipeline,
    reactivity_bgl: wgpu::BindGroupLayout,

    instability: wgpu::ComputePipeline,
    instability_bgl: wgpu::BindGroupLayout,

    accumulate: wgpu::ComputePipeline,
    accumulate_bgl: wgpu::BindGroupLayout,

    ubo: wgpu::Buffer,
    linear: wgpu::Sampler,
    targets: Targets,
    render_size: (u32, u32),
    output_size: (u32, u32),
    state: std::sync::Mutex<History>,
}

impl Fsr3 {
    pub(super) fn resize(&mut self, device: &wgpu::Device, render: (u32, u32), output: (u32, u32)) {
        if (render, output) == (self.render_size, self.output_size)
            || render.0 == 0
            || render.1 == 0
        {
            return;
        }
        self.targets = Targets::new(device, render, output);
        self.render_size = render;
        self.output_size = output;
        // Every history in here is at the old grid, so none of it means
        // anything at the new one.
        let mut state = self.state.lock().expect("fsr3 history lock");
        state.reset = true;
        state.frame_index = 0;
    }

    /// The image the tonemap reads. Valid only after [`Self::draw`] has
    /// run this frame; before the first one it is the cleared half of
    /// the pair, which is black rather than undefined.
    pub(super) fn resolved_texture(&self) -> &wgpu::Texture {
        &self.targets.output.texture
    }
}
