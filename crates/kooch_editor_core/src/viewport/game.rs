//! The Game view — the scene through the *gameplay* camera, rendered beside the View panel rather
//! than instead of it (#592).

use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;
use kooch_core::time::Time;
use kooch_ecs::query::filter::Without;
use kooch_render::SkyRenderPass;
use kooch_render::camera_stack::{CameraStack, StackViews};
use kooch_render::meshlet::{MeshletBlit, MeshletRenderStage, ViewId};

/// The Game viewport's own render stats.
#[derive(Copy, Clone, Debug, Default)]
pub struct GameViewStats(pub kooch_render::meshlet::MeshletRenderStats);

use crate::editor_camera::markers::EditorCamera;
use crate::viewport::target::ViewportTarget;

/// The Game panel's offscreen target and its handle into the stage.
pub(crate) struct GameView {
    pub target: ViewportTarget,
    /// This view's slot in the [`MeshletRenderStage`]. Not the primary —
    /// that one belongs to the View panel.
    pub view_id: ViewId,
    /// Whether the last frame found a gameplay camera. Drives the
    /// panel's placeholder text, so an empty Game panel says *why* it is
    /// empty instead of showing black.
    pub has_camera: bool,
    /// One view per overlay camera (#1221), kept across frames so each keeps its own history.
    pub stack: StackViews,
}

impl GameView {
    pub fn new(
        device: &wgpu::Device,
        egui_renderer: &mut egui_wgpu::Renderer,
        format: wgpu::TextureFormat,
        size: (u32, u32),
        stage: &mut MeshletRenderStage,
    ) -> Self {
        Self {
            target: ViewportTarget::new(device, egui_renderer, format, size),
            view_id: stage.create_view(device, size),
            has_camera: false,
            stack: StackViews::default(),
        }
    }
}

/// Renders the gameplay camera into `game.target`.
pub(crate) fn render_game_view(
    gpu: &GpuContext,
    sky_pass: &mut SkyRenderPass,
    game: &mut GameView,
    stage: &mut MeshletRenderStage,
    blit: &MeshletBlit,
    resources: &mut Resources,
) -> bool {
    let stack = gameplay_stack(resources);
    let Some(camera) = stack.base else {
        game.has_camera = false;
        return false;
    };
    game.has_camera = true;
    let aspect = game.target.aspect();

    // Per view: dragging this panel's divider must not reallocate the
    // View panel's attachments.
    stage.resize_view(game.view_id, gpu.device(), game.target.size());

    // The stage submits its own command buffer (cull + raster +
    // deferred) before the encoder below reads its colour view.
    let stats = stage.render_with_assets(
        game.view_id,
        gpu.device(),
        gpu.queue(),
        resources,
        &camera,
        aspect,
    );
    // 🔴 Published under its OWN key.
    resources.insert(GameViewStats(stats));

    // Each overlay into its own view, at the panel's size: the Game panel is the preview of the
    // game, and a stack it did not compose would be the wrong preview (#1221).
    game.stack.retain(&stack, stage);
    for (entity, overlay) in &stack.overlays {
        let view = game
            .stack
            .view_for(*entity, stage, gpu.device(), game.target.size());
        let drawn =
            stage.render_with_assets(view, gpu.device(), gpu.queue(), resources, overlay, aspect);
        game.stack.mark(*entity, drawn.instances_uploaded > 0);
    }

    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("game_view_encoder"),
        });

    let sky_drawn = if let Some(active_sky) = SkyRenderPass::active_sky(resources) {
        let time_secs = resources
            .get::<Time>()
            .map(|t| t.elapsed_secs())
            .unwrap_or(0.0);
        sky_pass.render(
            gpu.queue(),
            &mut encoder,
            game.target.view(),
            game.target.depth_view(),
            resources,
            &camera,
            aspect,
            active_sky,
            time_secs,
        )
    } else {
        false
    };
    if !sky_drawn {
        super::render::clear_to_black(&mut encoder, game.target.view(), game.target.depth_view());
    }

    // Same per-frame truth the View panel uses: `instances_uploaded > 0` iff the pipeline ran a
    // real dispatch this frame. Gating on the pool's registered count instead would keep blitting a
    // colour view the stage does not clear when it skips, leaving last frame's ghost over the sky.
    if stats.instances_uploaded > 0
        && let Some(color) = stage.view_color_view(game.view_id)
        && let Some(depth) = stage.view_depth_sample(game.view_id)
    {
        blit.blit(
            gpu.device(),
            &mut encoder,
            color,
            depth,
            game.target.view(),
            game.target.depth_view(),
        );
    }

    // The overlays over the base, lowest priority first.
    for view in game.stack.drawn(&stack) {
        if let (Some(color), Some(depth)) =
            (stage.view_color_view(view), stage.view_depth_sample(view))
        {
            blit.blit(
                gpu.device(),
                &mut encoder,
                color,
                depth,
                game.target.view(),
                game.target.depth_view(),
            );
        }
    }

    // The same post-process the View panel runs: a game view without it would be the wrong preview
    // of the game (#1201).
    super::post::apply(gpu, &mut encoder, &game.target, resources);

    gpu.queue().submit(Some(encoder.finish()));
    true
}

/// The game's cameras, the editor's own left out: the View panel is where that one belongs.
fn gameplay_stack(resources: &Resources) -> CameraStack {
    CameraStack::read::<Without<EditorCamera>>(resources)
}

#[cfg(test)]
mod tests;
