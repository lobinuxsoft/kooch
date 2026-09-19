//! The resources the frame holds by value: the UI and view passes need `&mut` to them while
//! `Resources` is borrowed elsewhere.

use super::*;

pub(super) struct Taken {
    pub(super) gpu: GpuContext,
    pub(super) overlay: EditorOverlay,
    pub(super) game_view: Option<GameView>,
    pub(super) shader_preview: Option<crate::viewport::ShaderPreview>,
    pub(super) viewport: ViewportTarget,
    pub(super) sky_pass: SkyRenderPass,
    pub(super) meshlet_stage: Option<MeshletRenderStage>,
    pub(super) meshlet_blit: Option<MeshletBlit>,
    pub(super) gizmo_renderer: GizmoRenderer,
    pub(super) gizmo_batch: GizmoBatch,
    pub(super) mesh_gizmo_renderer: MeshGizmoRenderer,
    pub(super) mesh_gizmo_batch: MeshBatch,
    pub(super) project_state: Option<ProjectState>,
    pub(super) dlss: crate::dlss_sdk::SdkInstall,
}

impl Taken {
    pub(super) fn take(resources: &mut Resources) -> Self {
        Self {
            gpu: resources
                .remove::<GpuContext>()
                .expect("GpuContext not found"),
            overlay: resources
                .remove::<EditorOverlay>()
                .expect("EditorOverlay not found"),
            game_view: resources.remove::<GameView>(),
            shader_preview: resources.remove::<crate::viewport::ShaderPreview>(),
            viewport: resources
                .remove::<ViewportTarget>()
                .expect("ViewportTarget not found"),
            sky_pass: resources
                .remove::<SkyRenderPass>()
                .expect("SkyRenderPass not found"),
            meshlet_stage: resources.remove::<MeshletRenderStage>(),
            meshlet_blit: resources.remove::<MeshletBlit>(),
            gizmo_renderer: resources
                .remove::<GizmoRenderer>()
                .expect("GizmoRenderer not found"),
            gizmo_batch: resources.remove::<GizmoBatch>().unwrap_or_default(),
            mesh_gizmo_renderer: resources
                .remove::<MeshGizmoRenderer>()
                .expect("MeshGizmoRenderer not found"),
            mesh_gizmo_batch: resources.remove::<MeshBatch>().unwrap_or_default(),
            project_state: resources.remove::<ProjectState>(),
            dlss: resources
                .remove::<crate::dlss_sdk::SdkInstall>()
                .unwrap_or_default(),
        }
    }

    /// Last frame's size requests, applied before the UI so texture ids stay stable through it.
    pub(super) fn resize_targets(&mut self) {
        let (device, renderer) = (self.gpu.device(), &mut self.overlay.renderer);
        self.viewport.resize_if_needed(device, renderer);
        if let Some(game) = self.game_view.as_mut() {
            game.target.resize_if_needed(device, renderer);
        }
        if let Some(preview) = self.shader_preview.as_mut() {
            preview.resize_if_needed(device, renderer);
        }
    }

    pub(super) fn restore(self, resources: &mut Resources) {
        resources.insert(self.gpu);
        // Once the frame is submitted: a view resized this frame lets go of its old targets here.
        if let Some(pool) = resources.get_mut::<kooch_core::gpu::TargetPool>() {
            pool.end_frame();
        }
        resources.insert(self.dlss);
        resources.insert(self.overlay);
        resources.insert(self.viewport);
        if let Some(game) = self.game_view {
            resources.insert(game);
        }
        if let Some(preview) = self.shader_preview {
            resources.insert(preview);
        }
        resources.insert(self.sky_pass);
        resources.insert(self.gizmo_renderer);
        resources.insert(self.gizmo_batch);
        resources.insert(self.mesh_gizmo_renderer);
        resources.insert(self.mesh_gizmo_batch);
        if let Some(stage) = self.meshlet_stage {
            resources.insert(stage);
        }
        if let Some(blit) = self.meshlet_blit {
            resources.insert(blit);
        }
        if let Some(ps) = self.project_state {
            resources.insert(ps);
        }
    }
}
