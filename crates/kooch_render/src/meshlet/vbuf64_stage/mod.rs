//! Atomic R64 visibility-buffer pipeline (#493).

mod clear;
mod compute_shade;
mod debug_resolve;
mod density_clear;
mod dlss;
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
pub(super) const VBUF64_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R64Uint;

/// Format of the dummy color attachment the R64 raster pipeline declares to satisfy wgpu's
/// "fragment stage requires ≥ 1 color target" rule. `R8Uint` keeps memory at 1 byte/pixel; the
/// pipeline's `write_mask` is empty so no fragment writes ever land here.
pub(super) const DUMMY_COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Uint;

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
    fn mip_bias_scale(&self) -> f32 {
        if !self.technique.is_temporal() {
            return 1.0;
        }
        let render = self.size.0.max(1) as f32;
        let output = self.output_size.0.max(1) as f32;
        (render / output) * 0.5
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
        );
    }

    /// The SHADING half. See [`Self::render_geometry`] for why the
    /// two are apart.
    #[allow(clippy::too_many_arguments)]
    pub fn render_shading(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        depth_view: &wgpu::TextureView,
        depth_sample_view: &wgpu::TextureView,
        color_view: &wgpu::TextureView,
        density_view: &wgpu::TextureView,
        meshlet_bg: &wgpu::BindGroup,
        material_pipeline: Option<&crate::material::MaterialPipeline>,
        lights_bg: &wgpu::BindGroup,
        cull: &MeshletCull,
        scene: &MeshletScene,
        // #481 — the jittered matrix the raster and every reconstruction
        // off its visibility buffer use.
        view_proj: glam::Mat4,
        // …and the camera's own, which only the motion vectors read. Equal to `view_proj` whenever
        // TAA is off. See [`Self::next_jitter`] for why they are handed down as a pair rather than
        // one being derived here.
        unjittered_view_proj: glam::Mat4,
        contact: &crate::contact_shadow::ContactShadowUbo,
        debug_mode: u32,
        // #1159 — seconds since the engine started, for a surface that moves. It rides in the
        // padding the screen UBO already carried, so no binding changed to carry it.
        time: f32,
        // #732 — the tonemap moved out of the shading shader, so the
        // scalar it used to read from the Inti uniform has to reach
        // the pass that applies it now.
        exposure: f32,
        // the caller's `raster + shade`.
        scopes: Option<&kooch_core::gpu::GpuScopes>,
        parent: Option<&kooch_core::gpu::GpuQuery>,
        // systems have already removed `GpuContext` from `Resources` to get at. `None` in every
        // build and on every adapter that has no DLSS, which is most of them.
        dlss_runtime: Option<&kooch_core::gpu::DlssRuntime>,
    ) -> Option<Deferred> {
        // 🔴 DLSS hands back a command buffer of its own that has to be
        // submitted immediately after this frame's encoder, so it
        // travels all the way out of here rather than being recorded.
        let mut dlss_commands: Option<wgpu::CommandBuffer> = None;
        // Motion vectors, right after the raster that fills the vbuf they read and before anything
        // that shades (#481).
        if self.needs_motion() {
            let query = match (scopes, parent) {
                (Some(s), Some(p)) => Some(s.begin_child("motion vectors", encoder, p)),
                (Some(s), None) => Some(s.begin("motion vectors", encoder)),
                _ => None,
            };
            self.motion.dispatch(
                device,
                queue,
                encoder,
                &self.vbuf_view,
                meshlet_bg,
                cull.visible_meshlets_buffer(),
                scene.instance_buffer(),
                scene.previous_transform_buffer(),
                view_proj,
                unjittered_view_proj,
                self.size,
            );
            if let (Some(scopes), Some(query)) = (scopes, query) {
                scopes.end(encoder, query);
            }
        }
        // Colorize debug modes (ids / heatmaps / cull passthrough) render through the fullscreen
        // debug fragment pass.
        if debug_resolve::is_colorize_mode(debug_mode) {
            self.debug_resolve.draw(
                device,
                queue,
                encoder,
                &self.vbuf_view,
                color_view,
                density_view,
                cull,
                self.size,
                debug_mode,
            );
        } else if let Some(pipeline) = material_pipeline {
            // The label names the path, so the capture answers "which
            // one ran" without anybody having to trust a log line.
            let label = if self.compute_enabled {
                self.shading_rate.scope_label()
            } else {
                "shade: fragment"
            };
            let query = match (scopes, parent) {
                (Some(s), Some(p)) => Some(s.begin_child(label, encoder, p)),
                (Some(s), None) => Some(s.begin(label, encoder)),
                _ => None,
            };
            if self.compute_enabled {
                // At a reduced rate the shading writes the upsample's
                // own targets; at full rate it writes the screen and the
                // id target is bound but never stored to.
                let half = self.shading_rate.needs_upsample();
                if half {
                    self.upsample.clear_ids(encoder);
                }
                // 🔴 Neither branch writes `color_view` any more: the
                // compute path shades into HDR and the tonemap pass
                // below puts it on screen (#732).
                let shade_target = if half {
                    self.upsample.color_view()
                } else {
                    self.tonemap.hdr_view()
                };
                self.compute_shade.shade(
                    device,
                    queue,
                    encoder,
                    &self.vbuf_view,
                    depth_sample_view,
                    shade_target,
                    self.upsample.id_view(),
                    meshlet_bg,
                    cull,
                    scene,
                    pipeline,
                    lights_bg,
                    view_proj,
                    contact,
                    self.size,
                    self.shading_rate,
                    self.mip_bias_scale(),
                    time,
                    debug_mode,
                );
            } else if self.size != self.output_size {
                // 🔴 The fragment path shades a render-sized material depth buffer onto the
                // window-sized image, and wgpu refuses a pass whose attachments disagree — it
                // discards the whole thing, so the frame is not soft, it is absent.
                warn_once_about_transitional_frame(self.size, self.output_size);
            } else {
                self.two_pass.shade(
                    device,
                    queue,
                    encoder,
                    &self.vbuf_view,
                    &self.material_depth_view,
                    depth_sample_view,
                    color_view,
                    meshlet_bg,
                    cull,
                    scene,
                    pipeline,
                    lights_bg,
                    view_proj,
                    contact,
                    self.size,
                    time,
                    debug_mode,
                );
            }
            if let (Some(scopes), Some(query)) = (scopes, query) {
                scopes.end(encoder, query);
            }
            // Its own scope, and a sibling of the shading rather than a child of it: the whole
            // question this issue asks is whether what the reduced rate saves survives what putting
            // it back on screen costs. Two numbers a capture can subtract.
            if self.compute_enabled && self.shading_rate.needs_upsample() {
                let query = match (scopes, parent) {
                    (Some(s), Some(p)) => Some(s.begin_child("shade: upsample", encoder, p)),
                    (Some(s), None) => Some(s.begin("shade: upsample", encoder)),
                    _ => None,
                };
                self.upsample.draw(
                    device,
                    queue,
                    encoder,
                    &self.vbuf_view,
                    self.tonemap.hdr_view(),
                    self.size,
                );
                if let (Some(scopes), Some(query)) = (scopes, query) {
                    scopes.end(encoder, query);
                }
            }
            if self.compute_enabled {
                // The temporal resolve, between the radiance and the curve (#481). Skipped on the
                // debug views: they hand back display-referred false colour, and averaging a
                // cluster index with last frame's produces a number that indexes nothing.
                let mut source = self.tonemap.hdr_view();
                // While `source` is the render-resolution HDR texture, the tonemap must stretch its
                // written region to the target — a debug view that skips the upscaler used to show
                // the scene at half size in a corner.
                let mut source_scale = [
                    self.size.0 as f32 / self.output_size.0.max(1) as f32,
                    self.size.1 as f32 / self.output_size.1.max(1) as f32,
                ];
                if self.technique.is_temporal() && !replaces_shading(debug_mode) {
                    // 🔴 The scope carries the technique's name rather than a shared "temporal". A
                    // capture has to say WHICH one cost what, or the A/B that decides between them
                    // is two numbers under one label.
                    let label = match self.technique {
                        crate::quality::UpscaleTechnique::Sgsr2 => "sgsr2",
                        crate::quality::UpscaleTechnique::Fsr3 => "fsr3",
                        crate::quality::UpscaleTechnique::Dlss => "dlss",
                        _ => "taa",
                    };
                    let query = match (scopes, parent) {
                        (Some(s), Some(p)) => Some(s.begin_child(label, encoder, p)),
                        (Some(s), None) => Some(s.begin(label, encoder)),
                        _ => None,
                    };
                    // Strategy, dispatched by value: one match per
                    // frame, no vtable, and the compiler checks that a
                    // new technique is handled here.
                    source = match self.technique {
                        crate::quality::UpscaleTechnique::Sgsr2 => self.sgsr2.draw(
                            device,
                            queue,
                            encoder,
                            self.upscale_inputs(depth_sample_view, exposure, 0, None, None),
                        ),
                        crate::quality::UpscaleTechnique::Fsr3 => self.fsr3.draw(
                            device,
                            queue,
                            encoder,
                            self.upscale_inputs(
                                depth_sample_view,
                                exposure,
                                fsr3_debug_stage(debug_mode),
                                scopes,
                                query.as_ref(),
                            ),
                        ),
                        // 🔴 The one technique that can decline. A build without the feature, an AMD
                        // card, or an SDK call that failed all land in the same place: the engine's
                        // own resolve, so the frame is still antialiased and still arrives.
                        crate::quality::UpscaleTechnique::Dlss => {
                            let inputs =
                                self.upscale_inputs(depth_sample_view, exposure, 0, None, None);
                            match self
                                .dlss
                                .draw(device, queue, encoder, dlss_runtime, &inputs)
                            {
                                Some((view, commands)) => {
                                    dlss_commands = Some(commands);
                                    view
                                }
                                None => self.taa.draw(
                                    device,
                                    queue,
                                    encoder,
                                    self.tonemap.hdr_view(),
                                    self.motion.view(),
                                    depth_sample_view,
                                    exposure,
                                ),
                            }
                        }
                        _ => self.taa.draw(
                            device,
                            queue,
                            encoder,
                            self.tonemap.hdr_view(),
                            self.motion.view(),
                            depth_sample_view,
                            exposure,
                        ),
                    };
                    if let (Some(scopes), Some(query)) = (scopes, query) {
                        scopes.end(encoder, query);
                    }
                    source_scale = [1.0, 1.0];
                }
                // 🔴 Everything below reads the image the resolve just produced — and when that
                // resolve was DLSS, it has not happened yet.
                let mut post = dlss_commands.as_ref().map(|_| {
                    device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("kooch_post_upscale"),
                    })
                });
                let encoder: &mut wgpu::CommandEncoder = post.as_mut().unwrap_or(&mut *encoder);

                // 🔴 Sharpening reads a FINISHED image (#481 step 5), so when it runs the tonemap
                // resolves into its texture instead of into the window and it is RCAS that writes
                // what is presented.
                let sharpening = self.sharpening;
                let sharpening_runs = sharpening > 0 && !is_debug_view(debug_mode);
                let tonemap_target = if sharpening_runs {
                    self.sharpen.input_view()
                } else {
                    color_view
                };
                // HDR to the image. Its own scope: it is a full-screen pass that did not exist
                // before, and "the tonemap moved" has to be answerable with a number rather than an
                // argument.
                let query = match (scopes, parent) {
                    (Some(s), Some(p)) => Some(s.begin_child("tonemap", encoder, p)),
                    (Some(s), None) => Some(s.begin("tonemap", encoder)),
                    _ => None,
                };
                self.tonemap.draw(
                    queue,
                    device,
                    encoder,
                    source,
                    tonemap_target,
                    exposure,
                    // The Inti debug views hand back display-ready colour.
                    !is_display_referred(debug_mode),
                    source_scale,
                );
                if let (Some(scopes), Some(query)) = (scopes, query) {
                    scopes.end(encoder, query);
                }
                // Its own scope, because the whole argument for this pass is that ~0.2 ms buys back
                // what reconstruction takes away, and an argument of that shape is settled by two
                // numbers rather than by an opinion.
                if sharpening_runs {
                    let query = match (scopes, parent) {
                        (Some(s), Some(p)) => Some(s.begin_child("rcas", encoder, p)),
                        (Some(s), None) => Some(s.begin("rcas", encoder)),
                        _ => None,
                    };
                    self.sharpen
                        .draw(device, queue, encoder, color_view, sharpening);
                    if let (Some(scopes), Some(query)) = (scopes, query) {
                        scopes.end(encoder, query);
                    }
                }
                return dlss_commands
                    .zip(post)
                    .map(|(dlss, post)| Deferred { dlss, post });
            }
        }
        None
    }
}

/// Work the caller must submit AFTER this frame's own encoder, in this order (#536).
pub struct Deferred {
    /// NVIDIA's own commands.
    pub dlss: wgpu::CommandBuffer,
    /// The tonemap and the sharpen, and whatever the caller adds.
    pub post: wgpu::CommandEncoder,
}

/// Reports the mismatched frame once, and never again.
fn warn_once_about_transitional_frame(render: (u32, u32), output: (u32, u32)) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        tracing::warn!(
            target: "kooch_render::meshlet",
            render_size = ?render,
            output_size = ?output,
            "the fragment shading path was handed a reduced render size and skipped a              frame; the scale belongs to the compute path and the two settings landed              one frame apart",
        );
    });
}

/// True for every debug mode, which is the question the TONEMAP asks: all of them produce colour
/// that is already display-referred, and a false-colour legend through a filmic curve is a legend
/// nobody can read off.
fn is_debug_view(debug_mode: u32) -> bool {
    debug_mode >= 11
}

/// True only for the modes Inti resolves INSIDE the shading shader, so there is no radiance for a
/// temporal technique to resolve.
fn replaces_shading(debug_mode: u32) -> bool {
    crate::meshlet::debug::MeshletDebugMode::all_implemented()
        .iter()
        .find(|m| m.as_u32() == debug_mode)
        .is_some_and(|m| m.replaces_shading())
}

/// True when the tonemap must pass the colour through untouched.
fn is_display_referred(debug_mode: u32) -> bool {
    crate::meshlet::debug::MeshletDebugMode::all_implemented()
        .iter()
        .find(|m| m.as_u32() == debug_mode)
        .is_some_and(|m| m.is_display_referred())
}

/// Which FSR 3.1 intermediate the debug dropdown is asking for, or 0.
fn fsr3_debug_stage(debug_mode: u32) -> u32 {
    crate::meshlet::debug::MeshletDebugMode::all_implemented()
        .iter()
        .find(|m| m.as_u32() == debug_mode)
        .map_or(0, |m| m.fsr3_stage())
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

#[cfg(test)]
mod tests;
