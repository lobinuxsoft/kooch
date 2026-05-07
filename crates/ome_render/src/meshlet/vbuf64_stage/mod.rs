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
//!   3. `Vbuf64Deferred::shade_scene`
//!                      — compute reads the u64, unpacks `(slot, tri)`,
//!                        runs the same materialed normal-debug shader
//!                        as the R32 path.
//!
//! Construction is gated on [`Vbuf64Support`](crate::vbuf64::Vbuf64Support);
//! the meshlet render stage carries an `Option<Vbuf64Stage>` and the
//! per-frame orchestrator switches paths atomically — the legacy R32Uint
//! resources stay live for adapters / backends that lack the atomic
//! features (Metal / MSL has no `atomic_uint64`).

mod clear;
mod deferred;
mod raster;

use bytemuck::{Pod, Zeroable};

use crate::material::MaterialPool;
use crate::meshlet::deferred::DEFERRED_COLOR_FORMAT;
use crate::meshlet::dispatcher::MeshletCull;
use crate::meshlet::render_stage::create_2d_attachment;
use crate::meshlet::scene::MeshletScene;

use clear::Vbuf64Clear;
use deferred::Vbuf64Deferred;
use raster::Vbuf64Rasterizer;

/// Storage texture format for the atomic visibility buffer.
pub(super) const VBUF64_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R64Uint;

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
    rasterizer: Vbuf64Rasterizer,
    deferred: Vbuf64Deferred,
    vbuf_texture: wgpu::Texture,
    vbuf_view: wgpu::TextureView,
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
        let clear = Vbuf64Clear::new(device);
        let rasterizer = Vbuf64Rasterizer::new(device, meshlet_bgl, depth_format, pipeline_cache);
        let deferred = Vbuf64Deferred::new(device, meshlet_bgl);
        Self {
            clear,
            rasterizer,
            deferred,
            vbuf_texture,
            vbuf_view,
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
        self.size = size;
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
        meshlet_bg: &wgpu::BindGroup,
        material_bg: &wgpu::BindGroup,
        cull: &MeshletCull,
        scene: &MeshletScene,
        view_proj: glam::Mat4,
        debug_mode: u32,
        clear_depth: bool,
    ) {
        self.clear
            .dispatch(device, queue, encoder, &self.vbuf_view, self.size);
        self.rasterizer.render_scene(
            device,
            queue,
            encoder,
            &self.vbuf_view,
            depth_view,
            meshlet_bg,
            cull,
            scene,
            view_proj,
            clear_depth,
        );
        self.deferred.shade_scene(
            device,
            queue,
            encoder,
            &self.vbuf_view,
            color_view,
            meshlet_bg,
            material_bg,
            cull,
            scene,
            view_proj,
            self.size,
            debug_mode,
        );
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

// Re-export the material BGL helper so the deferred submodule can build
// its pipeline layout without re-importing `MaterialPool` directly. Kept
// crate-private so external callers are not tempted to consume it.
#[allow(dead_code)]
fn material_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    MaterialPool::bind_group_layout(device)
}

#[cfg(test)]
mod tests {
    const RASTER_SOURCE: &str = include_str!("../../../shaders/meshlet_vbuf64.wgsl");
    const CLEAR_SOURCE: &str = include_str!("../../../shaders/meshlet_clear_vbuf64.wgsl");
    const DEFERRED_SOURCE: &str = include_str!("../../../shaders/meshlet_deferred_r64.wgsl");

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
    fn vbuf64_deferred_shader_validates() {
        validate(DEFERRED_SOURCE, "meshlet_deferred_r64.wgsl");
    }

    #[test]
    fn vbuf64_format_is_r64uint() {
        assert_eq!(super::VBUF64_FORMAT, wgpu::TextureFormat::R64Uint);
    }
}
