//! Editor render system — runs egui UI and presents overlay to the surface.

mod focus;
mod frame_display;
mod ui;

use kooch_core::event::{AppExit, Events};
use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_gizmos::{GizmoBatch, GizmoRenderer, MeshBatch, MeshGizmoRenderer};
use kooch_render::SkyRenderPass;
use kooch_render::meshlet::{
    MeshletBlit, MeshletDebugCaps, MeshletDebugMode, MeshletLodSettings, MeshletRenderStage,
    MeshletRenderStats,
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

use self::frame_display::FrameDisplayData;
use self::ui::{ToolbarInfo, ViewportUi, run_editor_ui};

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
    // perf HUD can report `cpu_frame_ms` (excludes GPU + present). The matching
    // `record_cpu_frame_ms` call lives at the end of this function.
    let frame_cpu_start = std::time::Instant::now();

    // The buffer is cloned out first: it is an `Arc` handle, so this is
    // cheap, and holding a borrow of `Resources` across the poll below
    // would collide with the mutable one that draining needs.
    let log_buffer = resources.get::<kooch_core::LogBuffer>().cloned();

    let is_playing = if let Some(play_state) = resources.get_mut::<PlayState>() {
        play_state.poll();
        let lines = play_state.drain_output();
        if let Some(buffer) = log_buffer.as_ref() {
            for line in &lines {
                crate::project_log::record(buffer, line);
            }
        }
        play_state.is_playing()
    } else {
        false
    };

    // The host's own output, which was captured and then never read by anyone. Everything a
    // mirrored project says — including every physics event — happens over there, so without this
    // the Console shows the editor talking to itself.
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

    let gpu = resources
        .remove::<GpuContext>()
        .expect("GpuContext not found");
    let mut overlay = resources
        .remove::<EditorOverlay>()
        .expect("EditorOverlay not found");
    let mut game_view = resources.remove::<GameView>();
    let mut shader_preview = resources.remove::<crate::viewport::ShaderPreview>();
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
    let mut dlss = resources
        .remove::<crate::dlss_sdk::SdkInstall>()
        .unwrap_or_default();
    let mut undo_stack = resources
        .remove::<UndoStack>()
        .unwrap_or_else(UndoStack::new);
    // Debug-mode resource is owned by the UI thread for the egui pass
    // so the View dropdown can mutate it directly. Re-inserted before
    // the meshlet stage runs so render_with_assets sees the new value.
    let mut meshlet_debug_mode = resources.remove::<MeshletDebugMode>().unwrap_or_default();
    // Capability probe (#454) drives the dropdown filter. Default is
    // conservative — when the resource is missing the filter falls
    // back to the baseline-safe subset of modes.
    let meshlet_debug_caps = resources
        .get::<MeshletDebugCaps>()
        .copied()
        .unwrap_or_default();
    let mut meshlet_lod_settings = resources.remove::<MeshletLodSettings>().unwrap_or_default();
    // The lights-per-pixel view's top of scale (#817). Out of the map
    // and back like the LOD threshold, so the panel edits the same value
    // the shading pass will read.
    let mut lights_hot = resources
        .remove::<kooch_lighting::LightsHot>()
        .unwrap_or_default();
    // Out of the map and back like the rest: the panel edits the same
    // settings the clustering passes will read this frame.
    let mut cluster_settings = resources
        .remove::<kooch_lighting::ClusterSettings>()
        .unwrap_or_default();
    let mut specular_floor = resources
        .remove::<kooch_lighting::SpecularFloor>()
        .unwrap_or_default();
    // Stats are produced by last frame's viewport render and re-published
    // as a Resource. Read-only here — copied so we don't keep the borrow
    // through the egui pass.
    let meshlet_stats = resources
        .get::<MeshletRenderStats>()
        .copied()
        .unwrap_or_default();
    // 🔴 The GAME viewport's own, published under its own key. The resource above is written by the
    // View camera's render alone, so the Game tab's overlay used to describe a frustum nobody was
    // looking through — and a page count that never moved while the game camera did.
    let game_stats = resources
        .get::<crate::viewport::game::GameViewStats>()
        .map(|s| s.0)
        .unwrap_or_default();

    // TIMESTAMP_QUERY) propagates from the render stage into the perf HUD Resource so the View
    // toolbar reads a single source.
    let vram_bytes = resources
        .get::<std::sync::Arc<kooch_render::EngineVramTracker>>()
        .map(|t| t.bytes())
        .unwrap_or(0);
    // 🔴 EVERY scope, not the meshlet chain. `meshlet_stats.gpu_frame_ms` times cull → raster →
    // shade for the main view and nothing else, so the shadow page passes were never in it: on
    // `dense.scene` the HUD read 0.55 ms while the pages took about nine.
    // Each shader's share of the last finished GPU frame, for the graph's header and the profiler.
    let shader_costs: Vec<(String, f32)> = resources
        .get::<kooch_core::gpu::GpuScopes>()
        .map(|scopes| {
            scopes
                .totals()
                .filter(|(label, _)| label.starts_with("shader "))
                .map(|(label, ms)| (label.to_owned(), ms))
                .collect()
        })
        .unwrap_or_default();
    let gpu_ms = resources
        .get::<kooch_core::gpu::GpuScopes>()
        .and_then(|scopes| scopes.frame_ms())
        .or(meshlet_stats.gpu_frame_ms);
    // Read off the surface rather than off a setting: `.rendersettings`
    // describes the project's window, and this is the editor's.
    let vsync = gpu.vsync();
    if let Some(stats) = resources.get_mut::<crate::perf::EditorPerfStats>() {
        stats.gpu_frame_ms = gpu_ms;
        stats.vsync = vsync;
        stats.vram_tracked_bytes = vram_bytes;
        // scene contents: sky background, viewport blit, egui paint.
        const EDITOR_BASE_PASSES: u32 = 3;
        stats.draw_calls = meshlet_stats.draw_calls + EDITOR_BASE_PASSES;
    }

    // Apply the previous frame's size request before the UI runs so the
    // texture id stays stable through the entire egui pass.
    viewport.resize_if_needed(gpu.device(), &mut overlay.renderer);
    if let Some(game) = game_view.as_mut() {
        game.target
            .resize_if_needed(gpu.device(), &mut overlay.renderer);
    }
    if let Some(preview) = shader_preview.as_mut() {
        preview.resize_if_needed(gpu.device(), &mut overlay.renderer);
    }

    // 🔴 Which history the Edit menu describes follows which one a Ctrl+Z would reach. With a
    // project open that is the remote one — the local stack still holds commands, but they describe
    // the mirror and nothing will ever run them again.
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
        _ => world_history(resources, &undo_stack),
    };

    // Per frame, not once: it includes whether a scene is dirty, and the
    // whole point is that the button goes away the moment it would cost
    // somebody their work.
    let install_blocked = resources
        .get::<crate::preflight::Report>()
        .and_then(|report| crate::install::refusal(resources, report));
    let toolbar = ToolbarInfo {
        install_blocked,
        can_undo,
        can_redo,
        undo_desc,
        redo_desc,
        document,
        clipboard_has_entities: resources
            .get::<crate::clipboard::EntityClipboard>()
            .is_some_and(|clipboard| !clipboard.is_empty()),
        remote: resources
            .get::<crate::remote_session::RemoteState>()
            .and_then(|s| s.session.as_ref().map(|s| s.state())),
        remote_stale: resources
            .get::<crate::remote_session::RemoteState>()
            .and_then(|s| s.session.as_ref()?.stale_reason().map(String::from)),
        scripts_behind: resources
            .get::<crate::script_sync::ScriptSync>()
            .is_some_and(|sync| sync.state == crate::script_sync::SyncState::NeedsRebuild),
        // In remote mode the project runs gameplay in place, so Play
        // is a wire toggle rather than a launched process.
        is_playing: is_playing
            || resources
                .get::<crate::remote_session::RemoteState>()
                .is_some_and(|s| s.playing),
    };

    if let Some(extra) = resources.get_mut::<kooch_window::ExtraWindows>() {
        let EditorOverlay {
            dock_state,
            windows,
            ctx,
            ..
        } = &mut overlay;
        crate::os_windows::sync(dock_state, windows, extra, ctx, &gpu);
    }

    let raw_input = {
        let mut state = overlay.winit_state.lock().unwrap();
        state.take_egui_input(&window)
    };

    let mut viewport_request: Option<(u32, u32)> = None;
    let mut game_request: Option<(u32, u32)> = None;
    // Which shape the Shader Graph panel wants its preview on. `Some` only when that panel was
    // drawn this frame, so a closed tab renders nothing (#1159).
    let mut preview_request: Option<crate::viewport::PreviewRequest> = None;
    let mut input_owner = crate::input_focus::InputOwner::default();
    let mut viewport_input: Option<ViewportInputDelta> = None;
    let controller_snapshot = resources
        .get::<EditorCameraController>()
        .cloned()
        .unwrap_or_default();

    // Snapshot the AssetDatabase once per frame for the inspector's typed asset picker. Empty when
    // the database is missing — the picker dropdown will simply show "(no <Type> assets
    // registered)".
    let (engine_root_owned, project_root_owned) = match project_state.as_ref() {
        Some(ps) => (
            ps.engine_root.as_ref().map(|p| p.join("assets")),
            ps.active_project
                .as_ref()
                .map(|ap| ap.root_path.join("assets")),
        ),
        None => (None, None),
    };
    // The Asset Browser tree is rooted at the project *crate* root (not
    // `assets/`) so `src/`, `Cargo.toml`, `scenes/`, … are all browsable
    // and openable in an external IDE.
    let project_crate_root = project_state
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

    // Resolve the Asset Browser's selection into a data snapshot before the egui frame — the detail
    // pane needs the asset's contents, and resolving them requires mutable `Resources` (AssetServer
    // load).
    let open_input_map = resources.get::<crate::state::OpenInputMap>().cloned();
    // 🔴 Cloned rather than borrowed: `egui-snarl` edits the graph while it draws it, and the
    // closure below already holds `Resources`. What the panel changed is put back after the frame.
    let mut open_shader_graph = resources.get::<crate::state::OpenShaderGraph>().cloned();
    let asset_detail = overlay.selected_asset.and_then(|guid| {
        crate::systems::asset_detail::gather_asset_detail(guid, resources)
            .map(|detail| crate::panels::inspector::AssetSnapshot { guid, detail })
    });
    gather_stages.assets_ms = crate::perf::ms_since(assets_start);

    // Lifted out for the frame: the Gizmos dropdown mutates it, and the egui closure already holds
    // Resources immutably. Groups are resolved from the registry now rather than rebuilt inside the
    // menu, so the panel is a pure draw over data.
    let mut gizmo_visibility = resources
        .get::<crate::gizmos::GizmoVisibility>()
        .cloned()
        .unwrap_or_else(crate::gizmos::GizmoVisibility::new);
    let gizmo_groups = crate::gizmos::groups_from_resources(resources);
    // Same lift as the gizmo choices: the menu mutates it while the egui
    // closure holds Resources immutably. The overlay resource owns the
    // reusable line buffer, so only the switches travel.
    let mut physics_debug = resources
        .get::<crate::gizmos::PhysicsDebugOverlay>()
        .map(|overlay| overlay.categories)
        .unwrap_or_default();
    // Lifted for the same reason: the panel writes what it drew, and the
    // metric systems read it next frame to decide whether to pay (#703).
    let mut hud_visibility = resources
        .get::<crate::perf::HudVisibility>()
        .copied()
        .unwrap_or_default();
    let mut console = resources
        .remove::<crate::panels::console::ConsoleState>()
        .unwrap_or_default();
    // Cloned rather than borrowed: the egui closure holds `Resources`
    // immutably and the banner needs these lines inside it.
    let connect_output = resources
        .get::<crate::remote_session::RemoteState>()
        .map(|state| state.connect_output.clone())
        .unwrap_or_default();

    // Cloned out for the same reason as `connect_output`: the UI closure
    // borrows `resources` immutably and the prompt needs this inside it.
    let prefab_overwrite = resources
        .get::<crate::actions::PendingPrefabOverwrite>()
        .cloned();

    // The Build panel's view of things. Assembled here rather than in the panel because the panel
    // draws and does not read resources — and because the job has to be polled whether or not its
    // tab is even visible (#758).
    let build_panel = {
        // Two statements, not one: polling takes `resources` mutably and
        // so does loading the presets, and the first borrow has to end
        // before the second begins.
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
        let presets = crate::panels::build::presets_in(resources, &asset_catalog);
        crate::panels::build::BuildPanel {
            presets,
            status,
            log,
            project: project_loaded,
        }
    };

    // #691 — everything above was assembling what the UI is about to
    // read: the hierarchy, the inspector's view of it, the asset
    // catalog. It walks the world, so it grows with the scene.
    let mut stages = crate::perf::RenderStages {
        gather_ms: crate::perf::ms_since(frame_cpu_start),
        gather: gather_stages,
        ..Default::default()
    };

    // What the isolated light casts, in words (#743). Read before the UI runs because the panel has
    // no `Resources`, and computed only while the view is open — it is three component lookups, but
    // three that no other frame has any reason to pay for.
    let single_light_note = meshlet_debug_mode
        .needs_selected_light()
        .then(|| overlay.selected_entities.first().copied())
        .flatten()
        .and_then(|entity| kooch_lighting::shadow_note(resources, entity));

    // Read before the UI borrows nothing else from `resources`: the
    // report is inserted once at startup and never changes.
    let preflight = resources.get::<crate::preflight::Report>().cloned();
    let installing = resources
        .get::<crate::install::Installing>()
        .map(crate::install::Installing::progress);

    let ui_start = std::time::Instant::now();
    let (full_output, mut actions) = run_editor_ui(
        &mut overlay,
        &mut project_state,
        &mut dlss,
        preflight.as_ref(),
        installing.as_ref(),
        raw_input,
        project_loaded,
        &display_data,
        &toolbar,
        ViewportUi {
            texture_id: viewport.texture_id(),
            request: &mut viewport_request,
            game_texture_id: game_view
                .as_ref()
                .map(|g| g.target.texture_id())
                .unwrap_or(egui::TextureId::default()),
            game_request: &mut game_request,
            game_has_camera: game_view.as_ref().map(|g| g.has_camera).unwrap_or(false),
            preview_texture_id: shader_preview
                .as_ref()
                .map(|preview| preview.texture_id())
                .unwrap_or_default(),
            preview_primitive: shader_preview
                .as_ref()
                .map(|preview| preview.primitive())
                .unwrap_or_default(),
            preview_refusal: shader_preview
                .as_ref()
                .and_then(|preview| preview.refusal()),
            preview_request: &mut preview_request,
            input_owner: &mut input_owner,
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
        &mut meshlet_debug_mode,
        meshlet_debug_caps,
        single_light_note,
        &mut meshlet_lod_settings,
        &mut lights_hot,
        &mut cluster_settings,
        &mut specular_floor,
        meshlet_stats,
        game_stats,
        resources
            .get::<crate::perf::EditorPerfStats>()
            .copied()
            .unwrap_or_default(),
        &mut gizmo_visibility,
        &gizmo_groups,
        &mut physics_debug,
        &mut hud_visibility,
        log_buffer.as_ref(),
        &mut console,
        &connect_output,
        prefab_overwrite.as_ref(),
        &build_panel,
        crate::editor_camera::editor_camera_rotation(resources),
    );
    stages.ui_ms = crate::perf::ms_since(ui_start);
    let input_start = std::time::Instant::now();

    // #656 — egui's own answer to "does anything need redrawing", read
    // before `full_output` is handed to the presenter and consumed.
    let ui_repaint_delay = shortest_repaint_delay(&full_output);

    // The graph the node panel edited: the dock had a copy of it, and this puts it back — after
    // filing what it was, while the resource still holds it, so the edit can be undone (#1211).
    if let Some(open) = open_shader_graph {
        record_graph_edit(resources, &open);
        resources.insert(open);
    }

    // Put the choices back so the batch system and the save system see
    // whatever the dropdown just changed.
    resources.insert(gizmo_visibility);
    resources.insert(console);
    // What the panel actually drew. Without this the UI writes into a
    // copy and every metric system keeps paying for a section nobody has
    // open — the whole point of lifting it.
    resources.insert(hud_visibility);

    // The overlay resource is created on first use rather than at startup:
    // a host with no physics never grows one.
    match resources.get_mut::<crate::gizmos::PhysicsDebugOverlay>() {
        Some(overlay) => overlay.categories = physics_debug,
        None => {
            if physics_debug.any() {
                resources.insert(crate::gizmos::PhysicsDebugOverlay::new(physics_debug));
            }
        }
    }

    // Hand the (possibly toggled) debug mode + LOD threshold back to
    // the resource map before the viewport render pass picks them up.
    resources.insert(meshlet_debug_mode);
    resources.insert(meshlet_lod_settings);
    resources.insert(lights_hot);
    resources.insert(cluster_settings);
    resources.insert(specular_floor);

    // Which light the single-light view isolates (#743): the selection, because "one light at a
    // time" is what selecting a light already means and a second list to pick from is a second
    // thing to keep in step with the scene.
    resources.insert(kooch_lighting::DebugLight(
        meshlet_debug_mode
            .needs_selected_light()
            .then(|| overlay.selected_entities.first().copied())
            .flatten(),
    ));

    if let Some(size) = viewport_request {
        viewport.request_size(size);
    }
    if let (Some(size), Some(game)) = (game_request, game_view.as_mut()) {
        game.target.request_size(size);
    }
    // Published for the consumers that run in other stages — the remote
    // input sender reads it in PreUpdate next frame.
    if let Some(focus) = resources.get_mut::<crate::input_focus::InputFocus>() {
        focus.set_owner(input_owner);
    }

    // Apply viewport input to the editor camera before the same frame's render pass so the new pose
    // is visible immediately. Focus-on- selection uses the first selected entity's world position,
    // if any.
    let driving_camera = viewport_input.is_some_and(|delta| {
        delta.fly_active
            || delta.fly_keys.any()
            || delta.orbit_yaw != 0.0
            || delta.orbit_pitch != 0.0
            || delta.pan_dx != 0.0
            || delta.pan_dy != 0.0
            || delta.zoom_lines != 0.0
    });

    if let Some(delta) = viewport_input
        && let Some(mode) = delta.element_request
    {
        overlay.element_mode = mode;
    }

    // Before anything can drag: an element selection outlives neither a
    // switch to Object nor Play, and leaving it painted while the handle
    // is gated is a gizmo that looks grabbable and records nothing.
    crate::block_edit::drop_selection_unless_editing(
        resources,
        overlay.element_mode,
        resources
            .get::<crate::remote_session::RemoteState>()
            .is_some_and(|state| state.playing),
    );

    // A shape parameter edited in the Inspector reshapes its block this frame.
    if !resources
        .get::<crate::remote_session::RemoteState>()
        .is_some_and(|state| state.playing)
    {
        crate::block_edit::shape_sync::sync_block_shapes(resources);
    }

    // E with faces selected: pull them out. Before the handle, because
    // the same key asks for the rotate mode and only one of the two can
    // be what was meant.
    if let Some(delta) = viewport_input
        && delta.extrude_pressed
        && overlay.element_mode == crate::block_edit::ElementMode::Face
        && let [entity] = overlay.selected_entities.as_slice()
    {
        let entity = *entity;
        let step = overlay.snap_settings.translate;
        if let Some((before, after)) = crate::block_edit::extrude_selection(resources, entity, step)
            && let Some(source) = crate::block_edit::source_of(resources, entity)
        {
            actions.push(EditorAction::BlockEdit {
                entity,
                source,
                before: Box::new(before),
                after: Box::new(after),
            });
            if let Some(bake) = crate::block_edit::shape_sync::bake(resources, entity) {
                actions.push(bake);
            }
        }
    }

    if let Some(delta) = viewport_input {
        let selected_snapshot: Vec<_> = overlay.selected_entities.iter().copied().collect();
        let rotation_mode = overlay.rotation_display_mode;
        let snap = overlay.snap_settings;
        // Before the transform handles: a shape handle sits on the block, inside the reach of the
        // move arrows, and the more specific handle takes the click.
        let shape_active = crate::gizmos::shape_handles::apply_shape_handles(
            delta,
            resources,
            &selected_snapshot,
            snap,
            &mut actions,
        );
        let handle_active = shape_active
            || crate::gizmos::apply_handle_input(
                delta,
                resources,
                &selected_snapshot,
                rotation_mode,
                snap,
                &mut overlay.gizmo_drag_start,
                &mut overlay.shape_drag_start,
                &mut actions,
                overlay.element_mode,
            );
        if !handle_active {
            // Clicking picks only when a gizmo did not take the click:
            // a handle sits *over* the thing it moves, so picking first
            // would select whatever is behind the arrow the user grabbed.
            apply_viewport_click(delta, resources, &mut overlay);

            let focus_target = overlay
                .selected_entities
                .first()
                .copied()
                .and_then(|entity| focus_target(resources, entity));
            apply_viewport_input(delta, resources, focus_target);
        }
    }

    stages.input_ms = crate::perf::ms_since(input_start);

    let viewport_start = std::time::Instant::now();

    // 🔴 Once per frame, ahead of BOTH views. This lived inside the View panel's pass, so a material
    // edit reached the Game panel only when the View panel happened to be drawn — and a frame late
    // when it was, since Game renders first (#1171). The Game panel is a second view of the SAME
    // stage: everything they share is brought up to date here, by the frame, or by nobody.
    if project_loaded && let Some(stage) = meshlet_stage.as_mut() {
        stage.sync_assets_to_gpu(gpu.device(), gpu.queue(), resources);
    }

    // The Game panel renders first: a second view of the same stage, through the gameplay camera.
    // Before the View panel's pass rather than after, so the two submits stay in a fixed order and
    // a frame capture always reads the same way.
    if project_loaded
        && game_request.is_some()
        && let (Some(game), Some(stage), Some(blit)) = (
            game_view.as_mut(),
            meshlet_stage.as_mut(),
            meshlet_blit.as_ref(),
        )
    {
        render_game_view(&gpu, &mut sky_pass, game, stage, blit, resources);
    } else if let Some(game) = game_view.as_mut() {
        game.has_camera = false;
    }

    // The View panel, gated the way the Game panel above already is: `viewport_request` is `Some`
    // this frame iff the tab was actually drawn.
    if viewport_request.is_some() {
        // The meshlet stage + blit are constructed at startup and live for the whole editor
        // session; if either is missing, another system removed them mid-frame.
        let mut placeholder_stage;
        let placeholder_blit;
        let meshlet = match (meshlet_stage.as_mut(), meshlet_blit.as_ref()) {
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

    // The Shader Graph's preview, gated the way the panels above are: `preview_request` is `Some`
    // this frame iff that tab was drawn.
    if let Some(request) = preview_request
        && let Some(preview) = shader_preview.as_mut()
    {
        preview.show_primitive(gpu.device(), request.primitive);
        if let Some(side) = request.size {
            preview.request_size(side);
        }
        let dt = resources
            .get::<kooch_core::time::Time>()
            .map(|time| time.delta_secs())
            .unwrap_or(0.016);
        // Generated fresh, and cheap: a graph is a few dozen nodes, and the pipeline behind it is
        // rebuilt only when the WGSL it produces actually changes.
        // 🔴 A graph that does not generate says so in the column. Dropped silently, the preview
        // froze on the last image and every edit after that looked like it did nothing.
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
                &gpu,
                &shader.params_wgsl(),
                &shader.source,
                &shader.params,
                &images,
                dt,
            );
        }
    }

    if preview_request.is_none() {
        crate::panels::shader_graph::forget_opening(&overlay.ctx);
    }

    stages.viewport_ms = crate::perf::ms_since(viewport_start);

    let present_start = std::time::Instant::now();
    // Taken out and put back the way `gpu` is: the frame's resolve and
    // its boundary need `&mut`, and the viewport passes above only
    // needed `&`.
    let mut scopes = resources.remove::<kooch_core::gpu::GpuScopes>();
    let presented = present_editor_frame(&gpu, &mut overlay, &window, full_output, scopes.as_mut());
    if let Some(scopes) = scopes {
        resources.insert(scopes);
    }
    stages.present_ms = crate::perf::ms_since(present_start);

    resources.insert(gpu);
    // Read before the overlay goes back, applied after this frame's edits
    // — see `seal_histories`.
    let ended = overlay.ctx.input(|i| i.pointer.any_released());
    resources.insert(dlss);
    resources.insert(overlay);
    resources.insert(viewport);
    if let Some(game) = game_view {
        resources.insert(game);
    }
    if let Some(preview) = shader_preview {
        resources.insert(preview);
    }
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

    let actions_start = std::time::Instant::now();
    apply_deferred_actions(resources, &actions, &mut undo_stack);
    if ended {
        seal_histories(resources);
    }
    stages.actions_ms = crate::perf::ms_since(actions_start);

    resources.insert(undo_stack);

    // needs nothing. Actions are applied first: one of them may have opened a project or started
    // Play, and the frame that does so must not go to sleep before the effect is visible.
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

    // HUD Resource. Last call so the elapsed measurement covers every CPU branch above (early
    // returns excepted; those are wall-clock-trivial).
    record_cpu_frame_ms(resources, frame_cpu_start);
    // #691 — published after the total, so the residual the HUD derives
    // from the two is read from the same frame.
    crate::perf::record_render_stages(resources, stages);
}

/// Moves the mirrored project's stdout into the editor's log.
fn forward_remote_output(resources: &mut Resources) {
    let Some(state) = resources.get::<crate::remote_session::RemoteState>() else {
        return;
    };
    let Some(session) = state.session.as_ref() else {
        return;
    };
    // Kept as well as forwarded, while the handshake is still in flight: the log is where these
    // belong, but the connecting banner needs something to show and draining is destructive (#672).
    // Once the project answers, the Console is the place to read it and the copy stops growing.
    let keep = session.state() == crate::remote_session::ConnectionState::Connecting;
    let Some(buffer) = resources.get::<kooch_core::LogBuffer>() else {
        return;
    };
    let buffer = buffer.clone();
    let lines = session.drain_output();
    for line in &lines {
        crate::project_log::record(&buffer, line);
    }
    if keep
        && !lines.is_empty()
        && let Some(state) = resources.get_mut::<crate::remote_session::RemoteState>()
    {
        state.connect_output.extend(lines);
    }
}

/// Selects the entity under the cursor, if the viewport was clicked.
fn apply_viewport_click(
    delta: ViewportInputDelta,
    resources: &mut Resources,
    overlay: &mut EditorOverlay,
) {
    if !delta.lmb_clicked {
        return;
    }
    let playing = resources
        .get::<crate::remote_session::RemoteState>()
        .is_some_and(|state| state.playing);
    if playing {
        return;
    }
    let Some(cursor) = delta.cursor_local else {
        return;
    };

    // Element mode first. A miss falls through to entity picking only when it lands on another
    // entity, so clicking past the edited block onto empty space keeps it in the inspector.
    if overlay.element_mode.edits_elements()
        && let [entity] = overlay.selected_entities.as_slice()
    {
        let entity = *entity;
        let element = crate::block_edit::element_under(
            resources,
            entity,
            cursor,
            delta.viewport_size,
            overlay.element_mode,
        );
        let hit = crate::picking::entity_hit_at(resources, cursor, delta.viewport_size);
        let block = match element {
            Some(_) => {
                crate::block_edit::block_distance(resources, entity, cursor, delta.viewport_size)
            }
            None => None,
        };
        if let crate::block_edit::ElementClick::Switch(other) =
            crate::block_edit::resolve_click(entity, element, hit, block)
        {
            if let Some(selection) = resources.get_mut::<crate::block_edit::BlockSelection>() {
                selection.clear();
            }
            overlay.selected_entities.clear();
            overlay.selected_entities.push(other);
            return;
        }
        if let Some(mut selection) = resources.remove::<crate::block_edit::BlockSelection>() {
            crate::block_edit::apply_click(&mut selection, entity, element, delta.ctrl_held);
            resources.insert(selection);
        }
        return;
    }

    let hit = crate::picking::entity_at(resources, cursor, delta.viewport_size);
    match (hit, delta.ctrl_held) {
        // Ctrl adds and removes, the same chord the World panel uses, so
        // building a multi-selection does not depend on which panel it was
        // started in.
        (Some(entity), true) => match overlay.selected_entities.iter().position(|e| *e == entity) {
            Some(index) => {
                overlay.selected_entities.remove(index);
            }
            None => overlay.selected_entities.push(entity),
        },
        (Some(entity), false) => {
            overlay.selected_entities.clear();
            overlay.selected_entities.push(entity);
        }
        (None, false) => overlay.selected_entities.clear(),
        // Ctrl+click on nothing is a miss, not "deselect everything".
        (None, true) => {}
    }
}

/// What the scene's history can offer, from whichever one is driving it.
fn world_history(
    resources: &kooch_core::resource::Resources,
    undo_stack: &UndoStack,
) -> (bool, bool, Option<String>, Option<String>) {
    let remote = resources
        .get::<crate::remote_session::RemoteState>()
        .is_some_and(|state| state.is_connected())
        .then(|| resources.get::<crate::actions::remote_undo::RemoteHistory>())
        .flatten();
    match remote {
        Some(history) => (
            history.can_undo(),
            history.can_redo(),
            history.undo_description().map(String::from),
            history.redo_description().map(String::from),
        ),
        None => (
            undo_stack.can_undo(),
            undo_stack.can_redo(),
            undo_stack.undo_description().map(String::from),
            undo_stack.redo_description().map(String::from),
        ),
    }
}

/// Whether a guid names a prefab or an ordinary asset.
fn asset_kind(
    resources: &kooch_core::resource::Resources,
    guid: kooch_core::Guid,
) -> crate::history::AssetKind {
    let prefab = resources
        .get::<kooch_core::asset_database::AssetDatabase>()
        .and_then(|db| db.entry(guid)?.type_name.clone())
        .is_some_and(|name| name == std::any::type_name::<kooch_ecs::scene::SceneDocument>());
    match prefab {
        true => crate::history::AssetKind::Prefab,
        false => crate::history::AssetKind::Asset,
    }
}

/// Files the open graph's previous state when the panel changed it this frame.
fn record_graph_edit(resources: &mut Resources, edited: &crate::state::OpenShaderGraph) {
    let Some(step) = resources
        .get::<crate::state::OpenShaderGraph>()
        .filter(|before| before.path == edited.path)
        .and_then(|before| {
            crate::shader_graph::change(&before.graph, &edited.graph).or_else(|| {
                (before.annotations != edited.annotations)
                    .then_some(crate::shader_graph::GraphStep::Annotate)
            })
        })
    else {
        return;
    };
    crate::history::documents::record(
        resources,
        &crate::history::Document::ShaderGraph(edited.path.clone()),
        step.label(),
        step.merge_key(&edited.path),
    );
}

/// Closes the current run of edits in every history.
fn seal_histories(resources: &mut Resources) {
    if let Some(history) = resources.get_mut::<crate::actions::remote_undo::RemoteHistory>() {
        history.seal();
    }
    if let Some(histories) = resources.get_mut::<crate::history::documents::DocumentHistories>() {
        histories.seal();
    }
}

/// What F should frame for this entity.
fn focus_target(
    resources: &mut Resources,
    entity: kooch_ecs::entity::Entity,
) -> Option<crate::editor_camera::framing::FocusTarget> {
    use crate::editor_camera::framing::{FocusTarget, radius_around};

    if let Some((min, max)) = crate::block_edit::selection_bounds(resources, entity) {
        let point = (min + max) * 0.5;
        return Some(FocusTarget {
            point,
            radius: Some(radius_around(point, min, max)),
        });
    }

    let point = entity_world_position(resources, entity)?;
    let radius = crate::picking::entity_bounds(resources, entity)
        .map(|(min, max)| radius_around(point, min, max));
    Some(FocusTarget { point, radius })
}

/// The image each texture node of the open graph is previewed with, by the parameter's name.
fn preview_images(resources: &Resources) -> Vec<(String, kooch_core::Guid)> {
    resources
        .get::<crate::state::OpenShaderGraph>()
        .map(|open| {
            open.graph
                .nodes()
                .filter_map(|node| match node {
                    crate::shader_graph::Node::Texture {
                        name,
                        preview: Some(guid),
                        ..
                    } => Some((name.clone(), *guid)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Loads an image asset through the asset server, as the Inspector's asset detail does.
fn loaded_image(
    resources: &mut Resources,
    guid: kooch_core::Guid,
) -> Option<kooch_render::texture::Image> {
    use kooch_core::asset_loader::AssetServer;
    use kooch_render::texture::Image;

    let mut server = resources.remove::<AssetServer>()?;
    let handle = server.load_by_guid::<Image>(guid, resources).ok();
    resources.insert(server);
    resources
        .get::<kooch_core::assets::Assets<Image>>()?
        .get(handle?)
        .cloned()
}
