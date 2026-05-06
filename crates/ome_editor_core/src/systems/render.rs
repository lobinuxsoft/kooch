//! Editor render system — runs egui UI and presents overlay to the surface.

mod frame_display;
mod ui;

use ome_core::event::{AppExit, Events};
use ome_core::gpu::GpuContext;
use ome_core::resource::Resources;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_gizmos::{GizmoBatch, GizmoRenderer, MeshBatch, MeshGizmoRenderer};
use ome_render::meshlet::{
    MeshletBlit, MeshletDebugMode, MeshletLodSettings, MeshletRenderStage, MeshletRenderStats,
};
use ome_render::SkyRenderPass;

use crate::actions::{apply_actions, EditorAction};
use crate::editor_camera::EditorCameraController;
use crate::editor_camera::input::{
    ViewportInputDelta, apply_viewport_input, entity_world_position,
};
use crate::perf::record_cpu_frame_ms;
use crate::play_state::PlayState;
use crate::project_state::{LauncherStatus, ProjectState};
use crate::state::EditorOverlay;
use crate::systems::present::present_editor_frame;
use crate::undo::UndoStack;
use crate::viewport::render::MeshletPathInputs;
use crate::viewport::{render_viewport, ViewportTarget};

use self::frame_display::FrameDisplayData;
use self::ui::{run_editor_ui, UndoInfo, ViewportUi};

/// Polls launcher state. Returns `true` when the render system should exit early
/// because the project binary has been launched (triggering AppExit).
fn poll_launcher(resources: &mut Resources) -> bool {
    if let Some(ps) = resources.get_mut::<ProjectState>() {
        ps.poll_launcher();
    }

    let launched = resources
        .get::<ProjectState>()
        .and_then(|ps| ps.launcher_status())
        .is_some_and(|s| *s == LauncherStatus::Launched);

    if launched
        && let Some(events) = resources.get_mut::<Events<AppExit>>()
    {
        events.send(AppExit);
    }
    launched
}

fn apply_deferred_actions(
    resources: &mut Resources,
    actions: &[EditorAction],
    undo_stack: &mut UndoStack,
) {
    if actions.is_empty() {
        return;
    }
    let has_open_scene = actions
        .iter()
        .any(|a| matches!(a, EditorAction::OpenScene));

    apply_actions(resources, actions, undo_stack);

    if has_open_scene
        && let Some(overlay) = resources.get_mut::<EditorOverlay>()
    {
        overlay.selected_entities.clear();
        overlay.last_clicked_index = None;
    }

    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
        archetypes.gc_empty_archetypes();
    }
}

/// Render system: runs egui UI and renders the overlay to the surface.
pub(crate) fn editor_render_system(resources: &mut Resources) {
    // #463.2 — capture the start of the CPU-side render work so the
    // perf HUD can report `cpu_frame_ms` (excludes GPU + present).
    // The matching `record_cpu_frame_ms` call lives at the end of
    // this function.
    let frame_cpu_start = std::time::Instant::now();

    let is_playing = if let Some(play_state) = resources.get_mut::<PlayState>() {
        play_state.poll();
        for line in play_state.drain_output() {
            tracing::info!("[game] {line}");
        }
        play_state.is_playing()
    } else {
        false
    };

    if poll_launcher(resources) {
        return;
    }

    let project_loaded = resources
        .get::<ProjectState>()
        .is_some_and(|ps| ps.is_project_loaded());

    let display_data = if project_loaded {
        FrameDisplayData::gather(resources)
    } else {
        FrameDisplayData::empty()
    };

    let window = resources
        .get::<ome_window::WindowHandle>()
        .expect("WindowHandle not found")
        .window()
        .clone();

    let gpu = resources
        .remove::<GpuContext>()
        .expect("GpuContext not found");
    let mut overlay = resources
        .remove::<EditorOverlay>()
        .expect("EditorOverlay not found");
    let mut viewport = resources
        .remove::<ViewportTarget>()
        .expect("ViewportTarget not found");
    let mut sky_pass = resources
        .remove::<SkyRenderPass>()
        .expect("SkyRenderPass not found");
    let mut meshlet_stage = resources.remove::<MeshletRenderStage>();
    let meshlet_blit = resources.remove::<MeshletBlit>();
    let mut gizmo_renderer = resources
        .remove::<GizmoRenderer>()
        .expect("GizmoRenderer not found");
    let gizmo_batch = resources.remove::<GizmoBatch>().unwrap_or_default();
    let mut mesh_gizmo_renderer = resources
        .remove::<MeshGizmoRenderer>()
        .expect("MeshGizmoRenderer not found");
    let mesh_gizmo_batch = resources.remove::<MeshBatch>().unwrap_or_default();
    let mut project_state = resources.remove::<ProjectState>();
    let mut undo_stack = resources
        .remove::<UndoStack>()
        .unwrap_or_else(UndoStack::new);
    let mut streaming_config = resources
        .remove::<ome_world::lod::LodRingConfig>()
        .unwrap_or_default();
    // Debug-mode resource is owned by the UI thread for the egui pass
    // so the View dropdown can mutate it directly. Re-inserted before
    // the meshlet stage runs so render_with_assets sees the new value.
    let mut meshlet_debug_mode = resources.remove::<MeshletDebugMode>().unwrap_or_default();
    let mut meshlet_lod_settings =
        resources.remove::<MeshletLodSettings>().unwrap_or_default();
    // Stats are produced by last frame's viewport render and re-published
    // as a Resource. Read-only here — copied so we don't keep the borrow
    // through the egui pass.
    let meshlet_stats = resources
        .get::<MeshletRenderStats>()
        .copied()
        .unwrap_or_default();

    // #463.4 — last frame's GPU timing (when adapter exposes
    // TIMESTAMP_QUERY) propagates from the render stage into the
    // perf HUD Resource so the View toolbar reads a single source.
    if let Some(stats) = resources.get_mut::<crate::perf::EditorPerfStats>() {
        stats.gpu_frame_ms = meshlet_stats.gpu_frame_ms;
    }

    // Apply the previous frame's size request before the UI runs so the
    // texture id stays stable through the entire egui pass.
    viewport.resize_if_needed(gpu.device(), &mut overlay.renderer);

    let undo = UndoInfo {
        can_undo: undo_stack.can_undo(),
        can_redo: undo_stack.can_redo(),
        undo_desc: undo_stack.undo_description().map(String::from),
        redo_desc: undo_stack.redo_description().map(String::from),
        is_playing,
    };

    let raw_input = {
        let mut state = overlay.winit_state.lock().unwrap();
        state.take_egui_input(&window)
    };

    let mut viewport_request: Option<(u32, u32)> = None;
    let mut viewport_input: Option<ViewportInputDelta> = None;
    let controller_snapshot = resources
        .get::<EditorCameraController>()
        .cloned()
        .unwrap_or_default();
    let power_profile = resources
        .get::<ome_core::power::PowerProfile>()
        .copied()
        .unwrap_or_default();

    // Snapshot the AssetDatabase once per frame for the inspector's
    // typed asset picker. Empty when the database is missing — the
    // picker dropdown will simply show "(no <Type> assets registered)".
    // The two roots drive the `[engine]` / `[project]` source tag
    // shown next to each entry — read from the already-extracted
    // `project_state` local because `ProjectState` was removed from
    // resources earlier in this function.
    let (engine_root_owned, project_root_owned) = match project_state.as_ref() {
        Some(ps) => (
            ps.engine_root.as_ref().map(|p| p.join("assets")),
            ps.active_project
                .as_ref()
                .map(|ap| ap.root_path.join("assets")),
        ),
        None => (None, None),
    };
    let asset_catalog = resources
        .get::<ome_core::asset_database::AssetDatabase>()
        .map(|db| {
            crate::panels::inspector::AssetCatalogEntry::collect_from_database(
                db,
                engine_root_owned.as_deref(),
                project_root_owned.as_deref(),
            )
        })
        .unwrap_or_default();

    let (full_output, mut actions) = run_editor_ui(
        &mut overlay,
        &mut project_state,
        raw_input,
        project_loaded,
        &display_data,
        &undo,
        ViewportUi {
            texture_id: viewport.texture_id(),
            request: &mut viewport_request,
            input: &mut viewport_input,
            controller: &controller_snapshot,
            handle_mode: resources
                .get::<ome_gizmos_handles::HandleSet>()
                .map(|h| h.mode())
                .unwrap_or_default(),
        },
        power_profile,
        &mut streaming_config,
        &asset_catalog,
        &mut meshlet_debug_mode,
        &mut meshlet_lod_settings,
        meshlet_stats,
    );

    // Hand the (possibly toggled) debug mode + LOD threshold back to
    // the resource map before the viewport render pass picks them up.
    resources.insert(meshlet_debug_mode);
    resources.insert(meshlet_lod_settings);

    if let Some(size) = viewport_request {
        viewport.request_size(size);
    }

    // Apply viewport input to the editor camera before the same frame's
    // render pass so the new pose is visible immediately. Focus-on-
    // selection uses the first selected entity's world position, if any.
    //
    // First give the gizmo handle system a chance to absorb input. If a
    // handle is hovered or being dragged, suppress camera input so the
    // user doesn't inadvertently orbit while moving an entity.
    if let Some(delta) = viewport_input {
        let selected_snapshot: Vec<_> = overlay.selected_entities.iter().copied().collect();
        let rotation_mode = overlay.rotation_display_mode;
        let snap = overlay.snap_settings;
        let handle_active = crate::gizmos::apply_handle_input(
            delta,
            resources,
            &selected_snapshot,
            rotation_mode,
            snap,
            &mut overlay.gizmo_drag_start,
            &mut actions,
        );
        if !handle_active {
            let selection_world = overlay
                .selected_entities
                .first()
                .copied()
                .and_then(|e| entity_world_position(resources, e));
            apply_viewport_input(delta, resources, selection_world);
        }
    }

    {
        // The meshlet stage + blit are constructed at startup and live
        // for the whole editor session; if either is missing, another
        // system removed them mid-frame. Reconstruct minimal
        // placeholders so the call still type-checks — they'll be
        // re-inserted at the end of this system anyway.
        let mut placeholder_stage;
        let placeholder_blit;
        let meshlet = match (meshlet_stage.as_mut(), meshlet_blit.as_ref()) {
            (Some(stage), Some(blit)) => MeshletPathInputs { stage, blit },
            _ => {
                placeholder_stage = MeshletRenderStage::new(
                    gpu.device(),
                    ome_render::meshlet::MeshletRenderStageConfig::default(),
                );
                placeholder_blit = MeshletBlit::new(gpu.device(), gpu.format());
                MeshletPathInputs {
                    stage: &mut placeholder_stage,
                    blit: &placeholder_blit,
                }
            }
        };

        render_viewport(
            &gpu,
            &mut sky_pass,
            &mut gizmo_renderer,
            &gizmo_batch,
            &mut mesh_gizmo_renderer,
            &mesh_gizmo_batch,
            &viewport,
            resources,
            project_loaded,
            meshlet,
        );
    }

    let _presented = present_editor_frame(&gpu, &mut overlay, &window, full_output);

    resources.insert(gpu);
    resources.insert(overlay);
    resources.insert(viewport);
    resources.insert(sky_pass);
    resources.insert(gizmo_renderer);
    resources.insert(gizmo_batch);
    resources.insert(mesh_gizmo_renderer);
    resources.insert(mesh_gizmo_batch);
    if let Some(stage) = meshlet_stage {
        resources.insert(stage);
    }
    if let Some(blit) = meshlet_blit {
        resources.insert(blit);
    }
    if let Some(ps) = project_state {
        resources.insert(ps);
    }
    resources.insert(streaming_config);

    apply_deferred_actions(resources, &actions, &mut undo_stack);

    resources.insert(undo_stack);

    // #463.2 — write the CPU side of the frame budget into the perf
    // HUD Resource. Last call so the elapsed measurement covers
    // every CPU branch above (early returns excepted; those are
    // wall-clock-trivial).
    record_cpu_frame_ms(resources, frame_cpu_start);
}
