//! The frame's view passes: Game, View and the Shader Graph preview, each only when its tab drew.

use super::*;

/// What the UI asked of the views this frame; each is `Some` iff its tab was drawn.
pub(super) struct ViewRequests {
    pub(super) viewport: Option<(u32, u32)>,
    pub(super) game: Option<(u32, u32)>,
    pub(super) preview: Option<crate::viewport::PreviewRequest>,
}

impl Taken {
    pub(super) fn render_views(
        &mut self,
        resources: &mut Resources,
        project_loaded: bool,
        requests: &ViewRequests,
    ) {
        // 🔴 Once, ahead of BOTH views: inside the View pass, a material edit reached the Game panel
        // only when View was drawn, and a frame late since Game renders first (#1171).
        if project_loaded && let Some(stage) = self.meshlet_stage.as_mut() {
            stage.sync_assets_to_gpu(self.gpu.device(), self.gpu.queue(), resources);
        }

        // Game first, so the two submits keep a fixed order and a frame capture reads the same way.
        if project_loaded
            && requests.game.is_some()
            && let (Some(game), Some(stage), Some(blit)) = (
                self.game_view.as_mut(),
                self.meshlet_stage.as_mut(),
                self.meshlet_blit.as_ref(),
            )
        {
            render_game_view(&self.gpu, &mut self.sky_pass, game, stage, blit, resources);
        } else if let Some(game) = self.game_view.as_mut() {
            game.has_camera = false;
        }

        if requests.viewport.is_some() {
            self.render_view(resources, project_loaded);
        }

        if let Some(request) = requests.preview
            && let Some(preview) = self.shader_preview.as_mut()
        {
            render_preview(&self.gpu, preview, request, resources);
        } else if requests.preview.is_none() {
            crate::panels::shader_graph::forget_opening(&self.overlay.ctx);
        }
    }

    fn render_view(&mut self, resources: &mut Resources, project_loaded: bool) {
        let gpu = &self.gpu;
        // Both live for the whole session; missing means another system removed them mid-frame.
        let mut placeholder_stage;
        let placeholder_blit;
        let meshlet = match (self.meshlet_stage.as_mut(), self.meshlet_blit.as_ref()) {
            (Some(stage), Some(blit)) => MeshletPathInputs { stage, blit },
            _ => {
                placeholder_stage = MeshletRenderStage::new(
                    gpu.device(),
                    kooch_render::meshlet::MeshletRenderStageConfig::default(),
                );
                placeholder_blit = MeshletBlit::new(
                    gpu.device(),
                    gpu.format(),
                    kooch_render::VIEWPORT_DEPTH_FORMAT,
                );
                MeshletPathInputs {
                    stage: &mut placeholder_stage,
                    blit: &placeholder_blit,
                }
            }
        };
        render_viewport(
            gpu,
            &mut self.sky_pass,
            &mut self.gizmo_renderer,
            &self.gizmo_batch,
            &mut self.mesh_gizmo_renderer,
            &self.mesh_gizmo_batch,
            &self.viewport,
            resources,
            project_loaded,
            meshlet,
        );
    }
}

fn render_preview(
    gpu: &GpuContext,
    preview: &mut crate::viewport::ShaderPreview,
    request: crate::viewport::PreviewRequest,
    resources: &mut Resources,
) {
    preview.show_primitive(gpu.device(), request.primitive);
    if let Some(side) = request.size {
        preview.request_size(side);
    }
    let dt = resources
        .get::<kooch_core::time::Time>()
        .map(|time| time.delta_secs())
        .unwrap_or(0.016);
    // Regenerated each frame: a few dozen nodes; the pipeline rebuilds only when the WGSL changes.
    // 🔴 A failure is shown, not dropped: a frozen preview makes every later edit look ignored.
    let generated = resources
        .get::<crate::state::OpenShaderGraph>()
        .map(|open| crate::shader_graph::generate(&open.graph));
    let shader = match generated {
        Some(Ok(source)) => kooch_render::material::Shader::parse(&source).ok(),
        Some(Err(why)) => {
            preview.refuse(why);
            None
        }
        None => None,
    };
    let images = preview_images(resources);
    for (_, guid) in &images {
        if let Some(image) = loaded_image(resources, *guid) {
            preview.show_image(gpu.device(), gpu.queue(), *guid, &image);
        }
    }
    if let Some(shader) = shader {
        preview.render(
            gpu,
            &shader.params_wgsl(),
            &shader.source,
            &shader.params,
            &images,
            dt,
        );
    }
}
