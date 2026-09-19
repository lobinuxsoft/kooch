//! Atomic R64 visibility-buffer pipeline (#493).

mod clear;
mod compute_shade;
mod debug_resolve;
mod density_clear;
mod dlss;
mod forward;

pub(crate) use forward::ForwardList;
pub(crate) use shader_cache::ShaderPipelines;
mod fsr3;
mod jitter;
mod motion;
mod raster;
mod sgsr2;
mod shader_cache;
mod shading_pad;
mod shading_rate;
mod sharpen;
mod taa;
mod tile_bins;
mod tonemap;
mod two_pass;
mod upsample;

use bytemuck::{Pod, Zeroable};

use crate::meshlet::dispatcher::MeshletCull;
use crate::meshlet::render_stage::create_2d_attachment;
use crate::meshlet::scene::MeshletScene;

use crate::meshlet::deferred::DEFERRED_COLOR_FORMAT;
use clear::Vbuf64Clear;
use compute_shade::ComputeShading;
use debug_resolve::DebugResolve;
use density_clear::DensityClear;
use motion::MotionVectors;
use raster::Vbuf64Rasterizer;
use taa::Taa;
use tonemap::Tonemap;
use upsample::ShadingUpsample;

pub use jitter::{JITTER_BASE_PHASES, Jitter};
pub use shading_rate::ShadingRate;

pub(super) use compute_shade::enabled_by_environment as compute_shading_override;
pub(super) use shading_rate::rate_from_environment as shading_rate_override;

/// Storage texture format for the atomic visibility buffer.
pub(crate) const VBUF64_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R64Uint;

/// Format of the dummy color attachment the R64 raster pipeline declares to satisfy wgpu's
/// "fragment stage requires ≥ 1 color target" rule. `R8Uint` keeps memory at 1 byte/pixel; the
/// pipeline's `write_mask` is empty so no fragment writes ever land here.
pub(crate) const DUMMY_COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Uint;

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub(super) struct CameraUbo {
    pub view_proj: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub(super) struct ScreenUbo {
    pub size: [u32; 2],
    pub material_id: u32,
    pub debug_mode: u32,
    /// Pixels per shaded sample, per axis (#825). Only the compute path
    /// reads it; the fragment paths write 1 and the field is inert.
    pub shading_rate: u32,
    /// What the uv derivatives are multiplied by before the mip is chosen — `exp2(mip_bias)`, and
    /// 1.0 for no bias (#881).
    pub mip_bias_scale: f32,
    /// Seconds since the engine started, for surfaces that move (#1159). In the padding the UBO
    /// already carried, so a shader graph gets time without a binding of its own.
    pub time: f32,
    /// To 32 bytes. The uniform is bound with a dynamic offset, and a
    /// size that is a multiple of 16 is the shape every backend agrees
    /// on without argument.
    pub _pad: [u32; 1],
}

/// End-to-end atomic R64 visibility-buffer pipeline (clear + raster + deferred). Owns its own
/// R64Uint texture and per-pass pipelines / bind-group layouts; reuses the meshlet pool BGL + cull
/// buffers + scene instance buffer + material pool BG from the surrounding render stage.
pub struct Vbuf64Stage {
    clear: Vbuf64Clear,
    /// Compute clear for the triangle-density accumulator (#454). Allocated unconditionally inside
    /// `Vbuf64Stage::new` because the vbuf64 feature bundle already implies `TEXTURE_ATOMIC`
    /// support (R32Uint atomic is a subset of the R64Uint atomic the rest of the stage needs).
    density_clear: DensityClear,
    rasterizer: Vbuf64Rasterizer,
    /// Two-pass material shading (#440) for normal-look modes.
    two_pass: two_pass::MaterialTwoPass,
    /// Transparent surfaces over the shaded scene (#452). Behind a lock because the frame shades
    /// through `&self` and the pass grows its list buffer.
    forward: std::sync::Mutex<forward::ForwardPass>,
    /// The compute alternative to `two_pass` (#824), which shades from a per-tile light list in
    /// workgroup memory. Both are built: the two exist to be captured against each other on the
    /// device, and [`compute_shade::enabled_by_environment`] picks per run.
    compute_shade: ComputeShading,
    compute_enabled: bool,
    /// Half-rate lighting and the pass that puts it back on screen (#825). Owns its own
    /// reduced-resolution targets; idle at [`ShadingRate::Full`], which is what every capture
    /// before this issue was taken against.
    upsample: ShadingUpsample,
    tonemap: Tonemap,
    motion: MotionVectors,
    /// The temporal resolve (#481) and whether it is switched on. Built unconditionally: its two
    /// history pairs are allocated for the life of the stage so turning it on is not the frame that
    /// stalls, the same rule the shading rate follows (#830).
    taa: Taa,
    /// SGSR 2, transliterated (#481 step 4). Built unconditionally alongside the resolve, for the
    /// reason the resolve itself is: switching technique must not be the frame that stalls, and the
    /// A/B between them in one session is how an upscaler is judged.
    sgsr2: sgsr2::Sgsr2,
    fsr3: fsr3::Fsr3,
    /// 🔴 Built alongside the other two even in a build that cannot run it. Without the `dlss`
    /// feature this is an empty shell that reports itself unready, which is what keeps `cfg` out of
    /// the frame and out of the settings.
    dlss: dlss::Dlss,
    /// Which one runs. See [`UpscaleTechnique`](crate::quality::UpscaleTechnique)
    /// for why this is an enum rather than a trait object.
    technique: crate::quality::UpscaleTechnique,
    /// RCAS, the pass that ends the frame (#481 step 5). Built
    /// unconditionally like the two above, and for the same reason.
    sharpen: sharpen::Sharpen,
    /// How much of it, 0..=100. Zero skips the pass entirely — off has
    /// to cost nothing, not cost a full-screen identity.
    sharpening: u32,
    /// This frame's sub-pixel offset in RENDER pixels, kept because
    /// SGSR 2 needs the value the projection was jittered by and the
    /// resolve does not.
    last_jitter: glam::Vec2,
    /// `tan(fov_vertical / 2) * aspect`, which SGSR 2's depth-clip
    /// threshold scales by. Set from the camera each frame.
    fov_k: f32,
    /// The camera's near plane; see [`Self::set_camera_lens`].
    near: f32,
    /// Which sub-pixel offset the next frame takes. Advances once per
    /// frame per view, which is why it lives here rather than beside the
    /// camera — two views of the same scene must not share a phase.
    jitter_index: u32,
    shading_rate: ShadingRate,
    /// Fullscreen fragment pass for the colorize debug modes.
    debug_resolve: DebugResolve,
    vbuf_texture: wgpu::Texture,
    vbuf_view: wgpu::TextureView,
    dummy_color_texture: wgpu::Texture,
    dummy_color_view: wgpu::TextureView,
    /// Pass-1 target of the two-pass material path: each covered pixel's `material_id` encoded as
    /// depth (`f32(id)/65535`). Pass-2 per-material shading depth-tests `Equal` against it.
    /// Allocated here so it tracks the stage's size alongside the vbuf / dummy targets.
    material_depth_texture: wgpu::Texture,
    material_depth_view: wgpu::TextureView,
    /// 🔴 The size everything up to the resolve is rendered at, which is NOT the size presented once
    /// a technique upscales (#481 step 4). Every target in this struct is this size except the
    /// resolve's output and the tonemap's.
    size: (u32, u32),
    /// What reaches the window. Equal to `size` unless the technique
    /// upscales and the project asked for a scale below 100.
    output_size: (u32, u32),
}

impl Vbuf64Stage {
    pub fn new(
        device: &wgpu::Device,
        meshlet_bgl: &wgpu::BindGroupLayout,
        depth_format: wgpu::TextureFormat,
        size: (u32, u32),
        output_size: (u32, u32),
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        let (vbuf_texture, vbuf_view) = create_vbuf64_texture(device, size);
        let (dummy_color_texture, dummy_color_view) = create_dummy_color_texture(device, size);
        let (material_depth_texture, material_depth_view) =
            create_material_depth_texture(device, size);
        let clear = Vbuf64Clear::new(device);
        let density_clear = DensityClear::new(device);
        let rasterizer = Vbuf64Rasterizer::new(device, meshlet_bgl, depth_format, pipeline_cache);
        let two_pass = two_pass::MaterialTwoPass::new(device, meshlet_bgl);
        let forward =
            forward::ForwardPass::new(device, depth_format, meshlet_bgl, two_pass.layouts());
        let compute_shade = ComputeShading::new(device, meshlet_bgl);
        let compute_enabled = compute_shade::enabled_by_environment().unwrap_or(false);
        let upsample = ShadingUpsample::new(device, size);
        let tonemap = Tonemap::new(device, size);
        let motion = MotionVectors::new(device, size, meshlet_bgl);
        let taa = Taa::new(device, size);
        // 🔴 Half rate is a property of the compute path and nothing else: the fragment path shades
        // inside its own raster, one invocation per covered pixel, and has no thread to remove.
        // Honouring the variable there would silently measure the wrong thing.
        let shading_rate = if compute_enabled {
            shading_rate::rate_from_environment().unwrap_or_default()
        } else {
            ShadingRate::Full
        };
        let debug_resolve = DebugResolve::new(device);
        Self {
            clear,
            density_clear,
            rasterizer,
            two_pass,
            forward: std::sync::Mutex::new(forward),
            compute_shade,
            compute_enabled,
            upsample,
            tonemap,
            motion,
            taa,
            // Off until an author or a settings asset asks for it. A temporal resolve changes every
            // pixel of the image, and that is not a default an engine should adopt on behalf of a
            // project that never mentioned it.
            sgsr2: sgsr2::Sgsr2::new(device, size, output_size),
            fsr3: fsr3::Fsr3::new(device, size, output_size),
            dlss: dlss::Dlss::new(device, size, output_size),
            technique: crate::quality::UpscaleTechnique::None,
            sharpen: sharpen::Sharpen::new(device, output_size),
            // Off until asked for, like the technique above it: this
            // rewrites every pixel of a finished image, and a project
            // that never mentioned sharpening did not ask for that.
            sharpening: 0,
            last_jitter: glam::Vec2::ZERO,
            // A 60-degree vertical lens at 16:9, replaced on the first
            // frame that has a camera.
            fov_k: (std::f32::consts::FRAC_PI_3 * 0.5).tan() * (16.0 / 9.0),
            near: 0.1,
            jitter_index: 0,
            shading_rate,
            output_size,
            debug_resolve,
            vbuf_texture,
            vbuf_view,
            dummy_color_texture,
            dummy_color_view,
            material_depth_texture,
            material_depth_view,
            size,
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32), output_size: (u32, u32)) {
        if (size, output_size) == (self.size, self.output_size) || size.0 == 0 || size.1 == 0 {
            return;
        }
        let (texture, view) = create_vbuf64_texture(device, size);
        self.vbuf_texture = texture;
        self.vbuf_view = view;
        let (dummy_tex, dummy_view) = create_dummy_color_texture(device, size);
        self.dummy_color_texture = dummy_tex;
        self.dummy_color_view = dummy_view;
        let (md_tex, md_view) = create_material_depth_texture(device, size);
        self.material_depth_texture = md_tex;
        self.material_depth_view = md_view;
        self.upsample.resize(device, size);
        self.tonemap.resize(device, size);
        self.motion.resize(device, size);
        self.taa.resize(device, size);
        self.sgsr2.resize(device, size, output_size);
        self.fsr3.resize(device, size, output_size);
        self.dlss.resize(device, size, output_size);
        self.sharpen.resize(device, output_size);
        self.size = size;
        self.output_size = output_size;
    }

    /// Which shading path this stage takes, overriding what `KOOCH_COMPUTE_SHADING` said at
    /// construction (#824).
    pub fn set_compute_shading(&mut self, on: bool) {
        self.compute_enabled = on;
        if !on {
            self.shading_rate = ShadingRate::Full;
        }
    }

    /// What the uv derivatives are multiplied by before a mip is chosen — `exp2(mip_bias)` (#881).
    pub(crate) fn mip_bias_scale(&self) -> f32 {
        if !self.technique.is_temporal() {
            return 1.0;
        }
        let render = self.size.0.max(1) as f32;
        let output = self.output_size.0.max(1) as f32;
        (render / output) * 0.5
    }

    /// The size it rasterises at.
    pub(crate) fn render_size(&self) -> (u32, u32) {
        self.size
    }

    /// Whether this view shades in compute.
    pub fn compute_shading(&self) -> bool {
        self.compute_enabled
    }

    /// How many pixels share one shaded sample (#825).
    pub fn set_shading_rate(&mut self, rate: ShadingRate) -> bool {
        if rate != ShadingRate::Full && !self.compute_enabled {
            return false;
        }
        self.shading_rate = rate;
        true
    }

    /// The motion-vector target (#481). `Rg16Float`, full resolution,
    /// one UV offset per pixel.
    pub fn motion_vector_texture(&self) -> &wgpu::Texture {
        self.motion.texture()
    }

    /// The most recent temporal resolve, for a test to read back.
    pub fn resolved_texture(&self) -> &wgpu::Texture {
        match self.technique {
            crate::quality::UpscaleTechnique::Sgsr2 => self.sgsr2.resolved_texture(),
            crate::quality::UpscaleTechnique::Fsr3 => self.fsr3.resolved_texture(),
            // `None` in a build without the feature, which is the same
            // answer the fallback gives: the TAA target is what was
            // actually written.
            crate::quality::UpscaleTechnique::Dlss => self
                .dlss
                .resolved_texture()
                .unwrap_or_else(|| self.taa.resolved_texture()),
            _ => self.taa.resolved_texture(),
        }
    }

    /// Switches the temporal resolve on or off (#481).
    pub fn set_temporal_aa(&mut self, on: bool) {
        self.technique = if on {
            crate::quality::UpscaleTechnique::Taa
        } else {
            crate::quality::UpscaleTechnique::None
        };
    }

    /// What DLSS insists `output` be rendered at, or `None` when it has
    /// not said — see `dlss::Dlss::wanted_render_size`.
    pub fn dlss_render_size(&self, output: (u32, u32)) -> Option<(u32, u32)> {
        self.dlss.wanted_render_size(output)
    }

    /// Whether DLSS cannot run, so a frame asking for it has to be
    /// rendered at the output's own size instead of an upscaler's.
    pub fn dlss_unusable(&self) -> bool {
        self.dlss.unusable()
    }

    /// Selects the technique (#536).
    pub fn set_upscale(&mut self, technique: crate::quality::UpscaleTechnique) {
        self.technique = technique;
    }

    pub fn technique(&self) -> crate::quality::UpscaleTechnique {
        self.technique
    }

    /// How hard RCAS sharpens the finished image, 0..=100 (#481 step 5).
    pub fn set_sharpening(&mut self, percent: u32) {
        self.sharpening = percent.min(100);
    }

    pub fn sharpening(&self) -> u32 {
        self.sharpening
    }

    /// The lens, for the techniques whose thresholds depend on it.
    pub fn set_camera_lens(&mut self, fov_y_rad: f32, aspect: f32, near: f32) {
        self.fov_k = sgsr2::fov_k(fov_y_rad, aspect);
        self.near = near.max(1.0e-4);
    }

    pub fn temporal_aa(&self) -> bool {
        self.technique.is_temporal()
    }

    /// Whether anything this frame will read the motion vectors.
    fn needs_motion(&self) -> bool {
        self.technique.is_temporal()
    }

    /// This frame's sub-pixel offset, and the pair of matrices that follow from it.
    pub fn next_jitter(&mut self, view_proj: glam::Mat4) -> Jitter {
        if !self.technique.is_temporal() {
            self.last_jitter = glam::Vec2::ZERO;
            return Jitter::none(view_proj);
        }
        let jitter = Jitter::at(
            self.jitter_index,
            view_proj,
            self.size,
            self.jitter_phases(),
        );
        self.jitter_index = self.jitter_index.wrapping_add(1);
        self.last_jitter = jitter.pixels;
        jitter
    }

    /// How many sub-pixel offsets this view cycles through.
    fn jitter_phases(&self) -> u32 {
        jitter::phase_count(self.size.0, self.output_size.0)
    }

    /// Everything a temporal upscaler consumes, gathered in one place.
    fn upscale_inputs<'a>(
        &'a self,
        depth: &'a wgpu::TextureView,
        exposure: f32,
        debug_stage: u32,
        scopes: Option<&'a kooch_core::gpu::GpuScopes>,
        parent: Option<&'a kooch_core::gpu::GpuQuery>,
    ) -> sgsr2::UpscaleInputs<'a> {
        sgsr2::UpscaleInputs {
            color: self.tonemap.hdr_view(),
            depth,
            motion: self.motion.view(),
            jitter: self.last_jitter,
            exposure,
            fov_k: self.fov_k,
            near: self.near,
            jitter_phases: self.jitter_phases() as f32,
            debug_stage,
            scopes,
            parent,
        }
    }

    pub fn shading_rate(&self) -> ShadingRate {
        self.shading_rate
    }

    pub fn material_depth_view(&self) -> &wgpu::TextureView {
        &self.material_depth_view
    }

    pub fn vbuf_view(&self) -> &wgpu::TextureView {
        &self.vbuf_view
    }

    pub fn vbuf_texture(&self) -> &wgpu::Texture {
        &self.vbuf_texture
    }

    /// Records clear → raster → deferred for the entire frame. `clear_depth` controls the depth
    /// attachment load op for pass A; the Hi-Z 2-pass orchestrator (#445 follow-up) will pass
    /// `false` for pass B once it ports onto this stage.
    #[allow(clippy::too_many_arguments)]
    /// The RASTER half: clears, then the R64 visibility buffer and the depth that comes with it.
    #[allow(clippy::too_many_arguments)]
    pub fn render_geometry(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        depth_view: &wgpu::TextureView,
        density_view: &wgpu::TextureView,
        density_mode: u32,
        meshlet_bg: &wgpu::BindGroup,
        cull: &MeshletCull,
        scene: &MeshletScene,
        view_proj: glam::Mat4,
        clear_depth: bool,
        masked: Option<crate::meshlet::MaskedDraw<'_>>,
    ) {
        self.clear.dispatch(
            device,
            queue,
            encoder,
            &self.vbuf_view,
            self.tonemap.hdr_view(),
            self.size,
        );
        // Clear the density accumulator before each frame's raster pass so the heatmap reflects
        // only the current frame's contribution count.
        self.density_clear
            .dispatch(device, queue, encoder, density_view, self.size);
        self.rasterizer.render_scene(
            device,
            queue,
            encoder,
            &self.vbuf_view,
            &self.dummy_color_view,
            depth_view,
            density_view,
            density_mode,
            meshlet_bg,
            cull,
            scene,
            view_proj,
            clear_depth,
            masked,
        );
    }
}

/// Work the caller must submit AFTER this frame's own encoder, in this order (#536).
pub struct Deferred {
    /// NVIDIA's own commands.
    pub dlss: wgpu::CommandBuffer,
    /// The tonemap and the sharpen, and whatever the caller adds.
    pub post: wgpu::CommandEncoder,
}

fn create_vbuf64_texture(
    device: &wgpu::Device,
    size: (u32, u32),
) -> (wgpu::Texture, wgpu::TextureView) {
    create_2d_attachment(
        device,
        "meshlet_vbuf64",
        size,
        VBUF64_FORMAT,
        wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
    )
}

fn create_dummy_color_texture(
    device: &wgpu::Device,
    size: (u32, u32),
) -> (wgpu::Texture, wgpu::TextureView) {
    create_2d_attachment(
        device,
        "meshlet_vbuf64_dummy_color",
        size,
        DUMMY_COLOR_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    )
}

fn create_material_depth_texture(
    device: &wgpu::Device,
    size: (u32, u32),
) -> (wgpu::Texture, wgpu::TextureView) {
    create_2d_attachment(
        device,
        "meshlet_material_depth",
        size,
        crate::meshlet::MATERIAL_DEPTH_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    )
}

mod debug_modes;
mod shading;

use debug_modes::*;

#[cfg(test)]
mod tests;
