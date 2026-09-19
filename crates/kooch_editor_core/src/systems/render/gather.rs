//! What the frame reads before the egui pass: the panels draw and do not read `Resources`.

use super::*;

/// Polls the launched project and forwards its output to the log. Whether it is still playing.
pub(super) fn poll_play(
    resources: &mut Resources,
    log_buffer: Option<&kooch_core::LogBuffer>,
) -> bool {
    let Some(play_state) = resources.get_mut::<PlayState>() else {
        return false;
    };
    play_state.poll();
    let lines = play_state.drain_output();
    if let Some(buffer) = log_buffer {
        for line in &lines {
            crate::project_log::record(buffer, line);
        }
    }
    play_state.is_playing()
}

/// Publishes the GPU side of the perf HUD. Returns each shader's share of the last finished GPU
/// frame, for the graph's header and the profiler.
pub(super) fn publish_gpu_stats(
    resources: &mut Resources,
    gpu: &GpuContext,
    meshlet_stats: &MeshletRenderStats,
) -> Vec<(String, f32)> {
    let vram_bytes = resources
        .get::<std::sync::Arc<kooch_render::EngineVramTracker>>()
        .map(|t| t.bytes())
        .unwrap_or(0);
    let shader_costs = resources
        .get::<kooch_core::gpu::GpuScopes>()
        .map(|scopes| {
            scopes
                .totals()
                .filter(|(label, _)| label.starts_with("shader "))
                .map(|(label, ms)| (label.to_owned(), ms))
                .collect()
        })
        .unwrap_or_default();
    // 🔴 Every scope, not the meshlet chain's `gpu_frame_ms`: that one misses the shadow page passes
    // (HUD 0.55 ms vs ~9 ms of pages on `dense.scene`).
    let gpu_ms = resources
        .get::<kooch_core::gpu::GpuScopes>()
        .and_then(|scopes| scopes.frame_ms())
        .or(meshlet_stats.gpu_frame_ms);
    // Off the surface: `.rendersettings` describes the project's window, not the editor's.
    let vsync = gpu.vsync();
    if let Some(stats) = resources.get_mut::<crate::perf::EditorPerfStats>() {
        stats.gpu_frame_ms = gpu_ms;
        stats.vsync = vsync;
        stats.vram_tracked_bytes = vram_bytes;
        // Sky background, viewport blit, egui paint.
        const EDITOR_BASE_PASSES: u32 = 3;
        stats.draw_calls = meshlet_stats.draw_calls + EDITOR_BASE_PASSES;
    }
    shader_costs
}

pub(super) fn toolbar_info(
    resources: &Resources,
    overlay: &EditorOverlay,
    undo_stack: &UndoStack,
    is_playing: bool,
) -> ToolbarInfo {
    // 🔴 The Edit menu describes the history a Ctrl+Z would reach. With a project open that is the
    // remote one: the local stack describes the mirror and nothing will run it again.
    let document = crate::history::resolve(
        overlay.focused_tab,
        overlay
            .selected_asset
            .map(|guid| (guid, asset_kind(resources, guid))),
        resources
            .get::<crate::state::OpenInputMap>()
            .map(|open| open.path.clone())
            .as_deref(),
        resources
            .get::<crate::state::OpenShaderGraph>()
            .map(|open| open.path.clone())
            .as_deref(),
    );
    let (can_undo, can_redo, undo_desc, redo_desc) = match document.as_ref() {
        // A document of its own, with a history of its own.
        Some(document) if !document.is_world() => {
            let histories = resources.get::<crate::history::documents::DocumentHistories>();
            (
                histories.is_some_and(|h| h.can_undo(document)),
                histories.is_some_and(|h| h.can_redo(document)),
                histories.and_then(|h| h.undo_description(document).map(String::from)),
                histories.and_then(|h| h.redo_description(document).map(String::from)),
            )
        }
        _ => world_history(resources, undo_stack),
    };
    let remote = resources.get::<crate::remote_session::RemoteState>();
    ToolbarInfo {
        // Per frame: it includes whether a scene is dirty, so the button goes the moment it would
        // cost somebody their work.
        install_blocked: resources
            .get::<crate::preflight::Report>()
            .and_then(|report| crate::install::refusal(resources, report)),
        can_undo,
        can_redo,
        undo_desc,
        redo_desc,
        document,
        clipboard_has_entities: resources
            .get::<crate::clipboard::EntityClipboard>()
            .is_some_and(|clipboard| !clipboard.is_empty()),
        remote: remote.and_then(|s| s.session.as_ref().map(|s| s.state())),
        remote_stale: remote.and_then(|s| s.session.as_ref()?.stale_reason().map(String::from)),
        scripts_behind: resources
            .get::<crate::script_sync::ScriptSync>()
            .is_some_and(|sync| sync.state == crate::script_sync::SyncState::NeedsRebuild),
        // In remote mode the project runs gameplay in place, so Play is a wire toggle.
        is_playing: is_playing || remote.is_some_and(|s| s.playing),
    }
}

/// The Build panel's view. The job is polled whether or not its tab is visible (#758).
pub(super) fn build_panel(
    resources: &mut Resources,
    asset_catalog: &[crate::panels::inspector::AssetCatalogEntry],
    project_loaded: bool,
) -> crate::panels::build::BuildPanel {
    // Two statements: polling and loading the presets both borrow `resources` mutably.
    let (status, log) = match resources.get_mut::<crate::build::BuildState>() {
        Some(state) => {
            state.poll();
            (
                state
                    .job
                    .as_ref()
                    .map(crate::build::BuildJob::status)
                    .cloned(),
                state.log.clone(),
            )
        }
        None => (None, Vec::new()),
    };
    let presets = crate::panels::build::presets_in(resources, asset_catalog);
    crate::panels::build::BuildPanel {
        presets,
        status,
        log,
        project: project_loaded,
    }
}
