//! Editor render system — runs egui UI and presents overlay to the surface.

mod edits;
mod focus;
mod frame_display;
mod gather;
mod lifted;
pub(crate) mod play_focus;
mod taken;
mod ui;
mod views;

use kooch_core::event::{AppExit, Events};
use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_gizmos::{GizmoBatch, GizmoRenderer, MeshBatch, MeshGizmoRenderer};
use kooch_render::SkyRenderPass;
use kooch_render::meshlet::{
    MeshletBlit, MeshletDebugCaps, MeshletRenderStage, MeshletRenderStats,
};

use crate::actions::{EditorAction, apply_actions};
use crate::editor_camera::EditorCameraController;
use crate::editor_camera::input::{
    ViewportInputDelta, apply_viewport_input, entity_world_position,
};
use crate::perf::record_cpu_frame_ms;
use crate::play_state::PlayState;
use crate::project_state::{LauncherStatus, ProjectState};
use crate::state::EditorOverlay;
use crate::systems::pacing::{editor_pace, shortest_repaint_delay};
use crate::systems::present::present_editor_frame;
use crate::undo::UndoStack;
use crate::viewport::render::MeshletPathInputs;
use crate::viewport::{GameView, ViewportTarget, render_game_view, render_viewport};

use self::edits::{apply_viewport_edits, drives_camera};
use self::frame_display::FrameDisplayData;
use self::gather::{build_panel, poll_play, publish_gpu_stats, toolbar_info};
use self::lifted::Lifted;
use self::taken::Taken;
use self::ui::{ToolbarInfo, ViewportUi, run_editor_ui};
use self::views::ViewRequests;

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

    if launched && let Some(events) = resources.get_mut::<Events<AppExit>>() {
        events.send(AppExit);
    }
    launched
}

fn apply_deferred_actions(
    resources: &mut Resources,
    actions: &[EditorAction],
    undo_stack: &mut UndoStack,
) {
    // Not just "did the user do something": a prefab saved last frame queued work that is drained
    // inside `apply_actions`, and an idle frame returning early is a frame that queue does not
    // drain.
    if actions.is_empty() && !crate::actions::prefab_propagate::anything_queued(resources) {
        return;
    }
    let has_open_scene = actions
        .iter()
        .any(|a| matches!(a, EditorAction::OpenScene { .. }));

    apply_actions(resources, actions, undo_stack);

    if has_open_scene && let Some(overlay) = resources.get_mut::<EditorOverlay>() {
        overlay.selected_entities.clear();
        // Pins name entities from the world that just went away. Entity ids are generational, so a
        // stale one cannot match a new entity — but keeping them would grow the set for the life of
        // the session with ids nothing will ever draw.
        overlay.pinned_gizmos.clear();
        overlay.last_clicked_index = None;
    }

    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
        archetypes.gc_empty_archetypes();
    }
}

/// Render system: runs egui UI and renders the overlay to the surface.
pub(crate) fn editor_render_system(resources: &mut Resources) {
    // `cpu_frame_ms` for the perf HUD, recorded at the end; excludes GPU and present.
    let frame_cpu_start = std::time::Instant::now();

    // Cloned out (an `Arc`): a borrow of `Resources` would collide with the poll's mutable one.
    let log_buffer = resources.get::<kooch_core::LogBuffer>().cloned();
    let is_playing = poll_play(resources, log_buffer.as_ref());
    // The host's own output: everything a mirrored project says, physics events included.
    forward_remote_output(resources);
    if poll_launcher(resources) {
        return;
    }

    let project_loaded = resources
        .get::<ProjectState>()
        .is_some_and(|ps| ps.is_project_loaded());
    let (display_data, mut gather_stages) = if project_loaded {
        profiling::scope!("editor: gather frame data");
        FrameDisplayData::gather(resources)
    } else {
        (FrameDisplayData::empty(), Default::default())
    };
    let window = resources
        .get::<kooch_window::WindowHandle>()
        .expect("WindowHandle not found")
        .window()
        .clone();

    let mut taken = Taken::take(resources);
    let mut undo_stack = resources
        .remove::<UndoStack>()
        .unwrap_or_else(UndoStack::new);
    let mut lifted = Lifted::take(resources);
    // #454; the default is the baseline-safe subset of modes.
    let meshlet_debug_caps = resources
        .get::<MeshletDebugCaps>()
        .copied()
        .unwrap_or_default();
    // Last frame's, from the View camera's render.
    let meshlet_stats = resources
        .get::<MeshletRenderStats>()
        .copied()
        .unwrap_or_default();
    // 🔴 The Game viewport's own: the View camera's stats described a frustum nobody looked through.
    let game_stats = resources
        .get::<crate::viewport::game::GameViewStats>()
        .map(|s| s.0)
        .unwrap_or_default();
    let shader_costs = publish_gpu_stats(resources, &taken.gpu, &meshlet_stats);

    taken.resize_targets();
    let toolbar = toolbar_info(resources, &taken.overlay, &undo_stack, is_playing);

    if let Some(modes) = resources.get::<kooch_core::window_mode::DisplayModes>() {
        taken.overlay.game_resolution.sizes = crate::panels::game::display_sizes(modes);
    }
    if let Some(extra) = resources.get_mut::<kooch_window::ExtraWindows>() {
        let EditorOverlay {
            dock_state,
            windows,
            ctx,
            ..
        } = &mut taken.overlay;
        crate::os_windows::sync(dock_state, windows, extra, ctx, &taken.gpu);
    }

    let raw_input = {
        let mut state = taken.overlay.winit_state.lock().unwrap();
        state.take_egui_input(&window)
    };

    let mut requests = ViewRequests {
        viewport: None,
        game: None,
        preview: None,
    };
    let mut input_owner = crate::input_focus::InputOwner::default();
    let mut game_clicked = false;
    let mut viewport_input: Option<ViewportInputDelta> = None;
    let controller_snapshot = resources
        .get::<EditorCameraController>()
        .cloned()
        .unwrap_or_default();

    // Roots for the inspector's typed asset picker; empty without a database.
    let (engine_root_owned, project_root_owned) = match taken.project_state.as_ref() {
        Some(ps) => (
            ps.engine_root.as_ref().map(|p| p.join("assets")),
            ps.active_project
                .as_ref()
                .map(|ap| ap.root_path.join("assets")),
        ),
        None => (None, None),
    };
    // The Asset Browser is rooted at the crate, not `assets/`, so `src/` and `Cargo.toml` show too.
    let project_crate_root = taken
        .project_state
        .as_ref()
        .and_then(|ps| ps.active_project.as_ref().map(|ap| ap.root_path.clone()));
    let assets_start = std::time::Instant::now();
    let asset_catalog = resources
        .get::<kooch_core::asset_database::AssetDatabase>()
        .map(|db| {
            crate::panels::inspector::AssetCatalogEntry::collect_from_database(
                db,
                engine_root_owned.as_deref(),
                project_root_owned.as_deref(),
            )
        })
        .unwrap_or_default();

    let open_input_map = resources.get::<crate::state::OpenInputMap>().cloned();
    // 🔴 Cloned: `egui-snarl` edits the graph while drawing it; put back after the frame.
    let mut open_shader_graph = resources.get::<crate::state::OpenShaderGraph>().cloned();
    // Resolved before the frame: loading the asset's contents needs mutable `Resources`.
    let asset_detail = taken.overlay.selected_asset.and_then(|guid| {
        crate::systems::asset_detail::gather_asset_detail(guid, resources)
            .map(|detail| crate::panels::inspector::AssetSnapshot { guid, detail })
    });
    gather_stages.assets_ms = crate::perf::ms_since(assets_start);

    let gizmo_groups = crate::gizmos::groups_from_resources(resources);
    let connect_output = resources
        .get::<crate::remote_session::RemoteState>()
        .map(|state| state.connect_output.clone())
        .unwrap_or_default();
    let prefab_overwrite = resources
        .get::<crate::actions::PendingPrefabOverwrite>()
        .cloned();
    let build_panel = build_panel(resources, &asset_catalog, project_loaded);

    // #691: everything above walks the world, so it grows with the scene.
    let mut stages = crate::perf::RenderStages {
        gather_ms: crate::perf::ms_since(frame_cpu_start),
        gather: gather_stages,
        ..Default::default()
    };

    // What the isolated light casts (#743), paid for only while that view is open.
    let single_light_note = lifted
        .isolated_light(&taken.overlay)
        .and_then(|entity| kooch_lighting::shadow_note(resources, entity));

    // Inserted once at startup and never changed.
    let preflight = resources.get::<crate::preflight::Report>().cloned();
    let installing = resources
        .get::<crate::install::Installing>()
        .map(crate::install::Installing::progress);

    let game_framing = selected_framing(resources, &taken.overlay.selected_entities);
    let ui_start = std::time::Instant::now();
    let (full_output, mut actions) = run_editor_ui(
        &mut taken.overlay,
        &mut taken.project_state,
        &mut taken.dlss,
        preflight.as_ref(),
        installing.as_ref(),
        raw_input,
        project_loaded,
        &display_data,
        &toolbar,
        ViewportUi {
            texture_id: taken.viewport.texture_id(),
            request: &mut requests.viewport,
            game_texture_id: taken
                .game_view
                .as_ref()
                .map(|g| g.target.texture_id())
                .unwrap_or_default(),
            game_request: &mut requests.game,
            game_has_camera: taken.game_view.as_ref().is_some_and(|g| g.has_camera),
            game_framing,
            preview_texture_id: taken
                .shader_preview
                .as_ref()
                .map(|preview| preview.texture_id())
                .unwrap_or_default(),
            preview_primitive: taken
                .shader_preview
                .as_ref()
                .map(|preview| preview.primitive())
                .unwrap_or_default(),
            preview_refusal: taken
                .shader_preview
                .as_ref()
                .and_then(|preview| preview.refusal()),
            preview_request: &mut requests.preview,
            input_owner: &mut input_owner,
            game_clicked: &mut game_clicked,
            shader_costs: &shader_costs,
            input: &mut viewport_input,
            controller: &controller_snapshot,
            handle_mode: resources
                .get::<kooch_gizmos_handles::HandleSet>()
                .map(|h| h.mode())
                .unwrap_or_default(),
        },
        &asset_catalog,
        asset_detail.as_ref(),
        open_input_map.as_ref(),
        open_shader_graph.as_mut(),
        engine_root_owned.as_deref(),
        project_crate_root.as_deref(),
        &mut lifted,
        meshlet_debug_caps,
        single_light_note,
        meshlet_stats,
        game_stats,
        resources
            .get::<crate::perf::EditorPerfStats>()
            .copied()
            .unwrap_or_default(),
        &gizmo_groups,
        log_buffer.as_ref(),
        &connect_output,
        prefab_overwrite.as_ref(),
        &build_panel,
        crate::editor_camera::editor_camera_rotation(resources),
    );
    stages.ui_ms = crate::perf::ms_since(ui_start);
    let input_start = std::time::Instant::now();

    // #656: read before the presenter consumes `full_output`.
    let ui_repaint_delay = shortest_repaint_delay(&full_output);

    // Back to the resource after filing what it was, so the edit can be undone (#1211).
    if let Some(open) = open_shader_graph {
        record_graph_edit(resources, &open);
        resources.insert(open);
    }
    let light = lifted.isolated_light(&taken.overlay);
    lifted.put_back(resources, light);

    if let Some(size) = requests.viewport {
        taken.viewport.request_size(size);
    }
    if let (Some(size), Some(game)) = (requests.game, taken.game_view.as_mut()) {
        game.target.request_size(size);
    }
    // Read by the remote input sender in PreUpdate next frame.
    if let Some(focus) = resources.get_mut::<crate::input_focus::InputFocus>() {
        focus.set_owner(input_owner);
    }
    // The editor's own window is the one the mouse is on, so the capture is the editor's to apply,
    // whatever process is running the game.
    let cursor = crate::input_focus::cursor_while_playing(
        resources
            .get::<kooch_input::CursorMode>()
            .copied()
            .unwrap_or_default(),
        resources
            .get::<PlayState>()
            .is_some_and(PlayState::is_playing),
        input_owner == crate::input_focus::InputOwner::Game,
        game_clicked,
        resources
            .get::<Box<dyn kooch_input::InputBackend>>()
            .is_some_and(|input| input.just_pressed(kooch_input::ids::KeyCode::Escape)),
    );
    resources.insert(cursor);

    let driving_camera = drives_camera(viewport_input);
    apply_viewport_edits(resources, &mut taken.overlay, viewport_input, &mut actions);
    stages.input_ms = crate::perf::ms_since(input_start);

    let viewport_start = std::time::Instant::now();
    taken.render_views(resources, project_loaded, &requests);
    stages.viewport_ms = crate::perf::ms_since(viewport_start);

    let present_start = std::time::Instant::now();
    // Out and back like `gpu`: the frame's resolve and boundary need `&mut`.
    let mut scopes = resources.remove::<kooch_core::gpu::GpuScopes>();
    let presented = present_editor_frame(
        &taken.gpu,
        &mut taken.overlay,
        &window,
        full_output,
        scopes.as_mut(),
    );
    if let Some(scopes) = scopes {
        resources.insert(scopes);
    }
    stages.present_ms = crate::perf::ms_since(present_start);

    // Read before the overlay goes back, applied after this frame's edits; see `seal_histories`.
    let ended = taken.overlay.ctx.input(|i| i.pointer.any_released());
    taken.restore(resources);

    let actions_start = std::time::Instant::now();
    apply_deferred_actions(resources, &actions, &mut undo_stack);
    if ended {
        seal_histories(resources);
    }
    stages.actions_ms = crate::perf::ms_since(actions_start);
    resources.insert(undo_stack);

    // After the actions: one may have opened a project or started Play, and the frame that did must
    // not sleep before the effect shows.
    let pace = if presented {
        editor_pace(
            ui_repaint_delay,
            toolbar.is_playing,
            toolbar.remote,
            driving_camera,
        )
    } else {
        kooch_core::frame_pacing::FramePace::Continuous
    };
    kooch_core::frame_pacing::FrameRequest::raise(resources, pace);

    // Last, so the measurement covers every branch above.
    record_cpu_frame_ms(resources, frame_cpu_start);
    // #691: after the total, so the HUD's residual reads one frame.
    crate::perf::record_render_stages(resources, stages);
}

mod helpers;

use helpers::*;

/// The framing of the one selected vcam, while it is on: the zones are drawn only while authored.
fn selected_framing(
    resources: &Resources,
    selected: &[kooch_ecs::entity::Entity],
) -> Option<kooch_camera::CameraFraming> {
    let [entity] = selected else {
        return None;
    };
    resources
        .get::<kooch_ecs::component::ComponentRegistry>()?
        .get_cpu::<kooch_camera::CameraFraming>()?
        .get(*entity)
        .copied()
        .filter(|framing| framing.enabled)
}
