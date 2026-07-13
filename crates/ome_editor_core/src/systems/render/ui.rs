//! Egui UI pass for the editor — runs the dock area, menu bar, and
//! launch screen depending on whether a project is loaded.

use egui_dock::DockArea;

use ome_render::meshlet::{
    MeshletDebugCaps, MeshletDebugMode, MeshletLodSettings, MeshletRenderStats,
};

use crate::actions::EditorAction;
use crate::editor_camera::EditorCameraController;
use crate::editor_camera::input::ViewportInputDelta;
use crate::launch_screen::{self, LaunchAction};
use crate::menu_bar::draw_menu_bar;
use crate::project_state::ProjectState;
use crate::state::EditorOverlay;
use crate::systems::tab_viewer::EditorTabViewer;

use super::frame_display::FrameDisplayData;

pub(super) struct UndoInfo {
    pub(super) can_undo: bool,
    pub(super) can_redo: bool,
    pub(super) undo_desc: Option<String>,
    pub(super) redo_desc: Option<String>,
    pub(super) is_playing: bool,
}

/// Handles to the viewport resource consumed by the UI: a read-only
/// texture id for drawing, slots where the View panel writes the
/// desired backing texture size and the captured input delta for the
/// frame, and a read-only snapshot of the camera controller used for
/// sensitivity reads inside the egui closure.
pub(super) struct ViewportUi<'a> {
    pub(super) texture_id: egui::TextureId,
    pub(super) request: &'a mut Option<(u32, u32)>,
    pub(super) input: &'a mut Option<ViewportInputDelta>,
    pub(super) controller: &'a EditorCameraController,
    pub(super) handle_mode: ome_gizmos_handles::HandleMode,
}

/// Runs the egui UI for one frame. Produces the tessellation input and the
/// editor actions queued by widgets.
//
// Migrating off `Context::run` + `DockArea::show(ctx, ...)` requires
// adopting the eframe 0.34+ `App::ui(&mut self, ui: &mut Ui)` pattern,
// which is a structural change to the editor's render loop. Out of
// scope for the #299 cleanup.
#[allow(deprecated)]
#[allow(clippy::too_many_arguments)]
pub(super) fn run_editor_ui(
    overlay: &mut EditorOverlay,
    project_state: &mut Option<ProjectState>,
    raw_input: egui::RawInput,
    project_loaded: bool,
    data: &FrameDisplayData,
    undo: &UndoInfo,
    viewport: ViewportUi<'_>,
    power_profile: ome_core::power::PowerProfile,
    asset_catalog: &[crate::panels::inspector::AssetCatalogEntry],
    asset_detail: Option<&crate::panels::asset_browser::AssetDetail>,
    meshlet_debug_mode: &mut MeshletDebugMode,
    meshlet_debug_caps: MeshletDebugCaps,
    meshlet_lod_settings: &mut MeshletLodSettings,
    meshlet_stats: MeshletRenderStats,
    perf_stats: crate::perf::EditorPerfStats,
) -> (egui::FullOutput, Vec<EditorAction>) {
    let mut selected = std::mem::take(&mut overlay.selected_entities);
    let mut selected_asset = overlay.selected_asset;
    let mut last_clicked_index = overlay.last_clicked_index.take();
    let mut actions: Vec<EditorAction> = Vec::new();
    let ViewportUi {
        texture_id,
        request,
        input,
        controller,
        handle_mode,
    } = viewport;

    let full_output = overlay.ctx.run(raw_input, |ctx| {
        if project_loaded {
            draw_menu_bar(
                ctx,
                &mut overlay.dock_state,
                &mut actions,
                undo.is_playing,
                undo.can_undo,
                undo.can_redo,
                undo.undo_desc.as_deref(),
                undo.redo_desc.as_deref(),
                power_profile,
            );

            let selection_has_transform = data.entities.iter().any(|info| {
                selected.contains(&info.entity)
                    && info.components.iter().any(|c| c.short_name == "Transform")
            });
            let mut tab_viewer = EditorTabViewer {
                entities: &data.entities,
                archetypes: &data.archetypes,
                component_types: &data.component_types,
                selected: &mut selected,
                reflected_types: &data.reflected_types,
                actions: &mut actions,
                entity_count: data.entity_count,
                archetype_count: data.archetype_count,
                active_archetype_count: data.active_archetype_count,
                last_clicked_index: &mut last_clicked_index,
                viewport_texture_id: texture_id,
                viewport_request: request,
                viewport_input: input,
                editor_camera_controller: controller,
                rotation_euler_cache: &mut overlay.rotation_euler_cache,
                rotation_display_mode: &mut overlay.rotation_display_mode,
                snap_settings: &mut overlay.snap_settings,
                handle_mode,
                selection_has_transform,
                asset_catalog,
                selected_asset: &mut selected_asset,
                asset_detail,
                meshlet_debug_mode,
                meshlet_debug_caps,
                meshlet_lod_settings,
                meshlet_stats,
                perf_stats,
            };

            DockArea::new(&mut overlay.dock_state)
                .style(egui_dock::Style::from_egui(ctx.global_style().as_ref()))
                .show(ctx, &mut tab_viewer);
        } else if let Some(ps) = project_state.as_mut() {
            let launch_actions = launch_screen::draw_launch_screen(ctx, ps);
            forward_launch_actions(launch_actions, &mut actions);
        }
    });

    overlay.selected_entities = selected;
    overlay.selected_asset = selected_asset;
    overlay.last_clicked_index = last_clicked_index;
    (full_output, actions)
}

fn forward_launch_actions(launch_actions: Vec<LaunchAction>, actions: &mut Vec<EditorAction>) {
    for la in launch_actions {
        match la {
            LaunchAction::OpenProject(path) => actions.push(EditorAction::OpenProject(path)),
            LaunchAction::CreateProject { name, parent_path } => {
                actions.push(EditorAction::CreateProject { name, parent_path })
            }
            LaunchAction::RemoveRecent(path) => actions.push(EditorAction::RemoveRecent(path)),
            LaunchAction::LaunchProject(path) => actions.push(EditorAction::LaunchProject(path)),
            LaunchAction::CancelLaunch => actions.push(EditorAction::CancelLaunch),
        }
    }
}
