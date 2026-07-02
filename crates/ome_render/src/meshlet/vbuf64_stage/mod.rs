//! Atomic R64 visibility-buffer pipeline (#493).
//!
//! Bevy-style winner-takes-all path that fixes the coplanar-meshlet
//! z-fighting the legacy R32Uint color-attachment path exhibits. Three
//! GPU passes per frame:
//!
//!   1. `Vbuf64Clear`   — compute clears the storage R64 vbuf to 0.
//!   2. `Vbuf64Rasterizer::render_scene`
//!                      — meshlet draw_indirect, fragment writes
//!                        `textureAtomicMax((depth<<32) | ids)`.
//!   3. Shading — two paths off the same vbuf, both all-fragment (#440):
//!      `MaterialTwoPass` (resolve material depth + per-material passes)
//!      for normal-look modes, `DebugResolve` for the colorize debug
//!      modes.
//!
//! Construction is gated on [`Vbuf64Support`](crate::vbuf64::Vbuf64Support);
//! the meshlet render stage carries an `Option<Vbuf64Stage>` and the
//! per-frame orchestrator switches paths atomically — the legacy R32Uint
//! resources stay live for adapters / backends that lack the atomic
//! features (Metal / MSL has no `atomic_uint64`).

mod clear;
mod debug_resolve;
mod density_clear;
mod raster;
mod two_pass;

use bytemuck::{Pod, Zeroable};

use crate::meshlet::deferred::DEFERRED_COLOR_FORMAT;
use crate::meshlet::dispatcher::MeshletCull;
use crate::meshlet::render_stage::create_2d_attachment;
use crate::meshlet::scene::MeshletScene;

use clear::Vbuf64Clear;
use debug_resolve::DebugResolve;
use density_clear::DensityClear;
use raster::Vbuf64Rasterizer;

/// Storage texture format for the atomic visibility buffer.
pub(super) const VBUF64_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R64Uint;

/// Format of the dummy color attachment the R64 raster pipeline declares
/// to satisfy wgpu's "fragment stage requires ≥ 1 color target" rule.
/// `R8Uint` keeps memory at 1 byte/pixel; the pipeline's `write_mask` is
/// empty so no fragment writes ever land here.
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
}

/// End-to-end atomic R64 visibility-buffer pipeline (clear + raster +
/// deferred). Owns its own R64Uint texture and per-pass pipelines /
/// bind-group layouts; reuses the meshlet pool BGL + cull buffers + scene
/// instance buffer + material pool BG from the surrounding render stage.
pub struct Vbuf64Stage {
    clear: Vbuf64Clear,
    /// Compute clear for the triangle-density accumulator (#454).
    /// Allocated unconditionally inside `Vbuf64Stage::new` because
    /// the vbuf64 feature bundle already implies `TEXTURE_ATOMIC`
    /// support (R32Uint atomic is a subset of the R64Uint atomic
    /// the rest of the stage needs).
    density_clear: DensityClear,
    rasterizer: Vbuf64Rasterizer,
    /// Two-pass material shading (#440) for normal-look modes.
    two_pass: two_pass::MaterialTwoPass,
    /// Fullscreen fragment pass for the colorize debug modes.
    debug_resolve: DebugResolve,
    vbuf_texture: wgpu::Texture,
    vbuf_view: wgpu::TextureView,
    dummy_color_texture: wgpu::Texture,
    dummy_color_view: wgpu::TextureView,
    /// Pass-1 target of the two-pass material path: each covered pixel's
    /// `material_id` encoded as depth (`f32(id)/65535`). Pass-2 per-material
    /// shading depth-tests `Equal` against it. Allocated here so it tracks
    /// the stage's size alongside the vbuf / dummy targets.
    material_depth_texture: wgpu::Texture,
    material_depth_view: wgpu::TextureView,
    size: (u32, u32),
}

impl Vbuf64Stage {
    pub fn new(
        device: &wgpu::Device,
        meshlet_bgl: &wgpu::BindGroupLayout,
        depth_format: wgpu::TextureFormat,
        size: (u32, u32),
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
        let debug_resolve = DebugResolve::new(device);
        Self {
            clear,
            density_clear,
            rasterizer,
            two_pass,
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

    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if size == self.size || size.0 == 0 || size.1 == 0 {
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
        self.size = size;
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

    /// Records clear → raster → deferred for the entire frame.
    /// `clear_depth` controls the depth attachment load op for pass A;
    /// the Hi-Z 2-pass orchestrator (#445 follow-up) will pass `false`
    /// for pass B once it ports onto this stage.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        depth_view: &wgpu::TextureView,
        color_view: &wgpu::TextureView,
        density_view: &wgpu::TextureView,
        density_mode: u32,
        meshlet_bg: &wgpu::BindGroup,
        material_pipeline: Option<&crate::material::MaterialPipeline>,
        cull: &MeshletCull,
        scene: &MeshletScene,
        view_proj: glam::Mat4,
        debug_mode: u32,
        clear_depth: bool,
    ) {
        self.clear
            .dispatch(device, queue, encoder, &self.vbuf_view, self.size);
        // Clear the density accumulator before each frame's raster
        // pass so the heatmap reflects only the current frame's
        // contribution count. Cost is negligible (one 8×8-tiled
        // compute dispatch per frame, masked off in production by the
        // density-enable uniform on the raster fragment).
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
        // Colorize debug modes (ids / heatmaps / cull passthrough) render
        // through the fullscreen debug fragment pass. Every other mode —
        // Off and the normal-look debug modes — shades through the
        // two-pass material path; the reject overlay is a separate
        // dispatch layered on top by the caller.
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
            self.two_pass.shade(
                device,
                queue,
                encoder,
                &self.vbuf_view,
                &self.material_depth_view,
                color_view,
                meshlet_bg,
                cull,
                scene,
                pipeline,
                view_proj,
                self.size,
            );
        }
    }
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
mod tests {
    const RASTER_SOURCE: &str = include_str!("../../../shaders/meshlet_vbuf64.wgsl");
    const CLEAR_SOURCE: &str = include_str!("../../../shaders/meshlet_clear_vbuf64.wgsl");

    fn validate(source: &str, label: &str) {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{label} should parse: {e:?}"));
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .unwrap_or_else(|e| panic!("{label} should validate: {e:?}"));
    }

    #[test]
    fn vbuf64_raster_shader_validates() {
        validate(RASTER_SOURCE, "meshlet_vbuf64.wgsl");
    }

    #[test]
    fn vbuf64_clear_shader_validates() {
        validate(CLEAR_SOURCE, "meshlet_clear_vbuf64.wgsl");
    }

    #[test]
    fn vbuf64_format_is_r64uint() {
        assert_eq!(super::VBUF64_FORMAT, wgpu::TextureFormat::R64Uint);
    }
}
