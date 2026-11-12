//! A `kind: ui` shader in the graph's preview (#1805): one node filling the target, drawn by the
//! pass that draws it in the game, over a dome so Parallax and Image Normal have a height to read.

use std::sync::Arc;

use kooch_core::Guid;
use kooch_core::gpu::GpuContext;
use kooch_ecs::{AtlasUpload, QUAD_IMAGE, UiDrawList, UiMaterial, UiQuad, UiShader};
use kooch_render::material::Shader;
use kooch_render::ui::{UiPass, validate_ui};

const SIDE: u32 = 128;

pub(super) struct UiPreview {
    pass: UiPass,
    list: UiDrawList,
    /// The text last validated, so an unchanged graph is not checked once per frame.
    checked: Arc<str>,
    refusal: Option<String>,
}

impl UiPreview {
    pub(super) fn new(gpu: &GpuContext) -> Self {
        let list = UiDrawList {
            quads: vec![UiQuad {
                color: [0.3, 0.5, 0.9, 1.0],
                border_color: [0.1, 0.1, 0.14, 1.0],
                uv_size: [SIDE, SIDE],
                flags: QUAD_IMAGE,
                ..UiQuad::default()
            }],
            materials: vec![UiMaterial::default()],
            atlas_side: SIDE,
            atlas_uploads: vec![AtlasUpload {
                rect: [0, 0, SIDE, SIDE],
                texels: dome(),
            }],
            ..UiDrawList::default()
        };
        Self {
            pass: UiPass::new(gpu.device(), gpu.format()),
            list,
            checked: "".into(),
            refusal: None,
        }
    }

    /// Draws `source` over `target`, or returns why it cannot be.
    pub(super) fn render(
        &mut self,
        gpu: &GpuContext,
        target: &wgpu::TextureView,
        size: (u32, u32),
        source: &str,
    ) -> Option<String> {
        if &*self.checked != source {
            self.checked = source.into();
            self.refusal = Shader::parse(source)
                .map_err(|e| e.to_string())
                .and_then(|shader| validate_ui(&shader))
                .err();
            self.list.shaders = vec![UiShader {
                guid: Guid::new_v4(),
                source: self.checked.clone(),
            }];
        }
        if self.refusal.is_some() {
            return self.refusal.clone();
        }
        let (width, height) = (size.0 as f32, size.1 as f32);
        self.list.quads[0].size = [width, height];
        self.list.quads[0].clip = [0.0, 0.0, width, height];
        let mut encoder = gpu
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shader_preview_ui"),
            });
        self.pass.draw(
            gpu.device(),
            gpu.queue(),
            &mut encoder,
            target,
            size,
            &mut self.list,
            true,
        );
        gpu.queue().submit(Some(encoder.finish()));
        None
    }
}

/// White at the middle, falling to black at the edge.
fn dome() -> Vec<u8> {
    let mut texels = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let at = |v: u32| (v as f32 + 0.5) / SIDE as f32 * 2.0 - 1.0;
            let reach = (1.0 - at(x).hypot(at(y))).clamp(0.0, 1.0);
            let shade = (reach.sqrt() * 255.0) as u8;
            texels.extend_from_slice(&[shade, shade, shade, 255]);
        }
    }
    texels
}
