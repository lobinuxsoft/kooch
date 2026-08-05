//! Editor render system — runs egui UI and presents overlay to the surface.

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
    // Not just "did the user do something": a prefab saved last frame
    // queued work that is drained inside `apply_actions`, and an idle
    // frame returning early is a frame that queue does not drain. It then
    // waits for the next unrelated action and lands in *that* batch, which
    // is how propagating a prefab appeared to only happen when the user
    // reverted an instance.
    if actions.is_empty() && !crate::actions::prefab_propagate::anything_queued(resources) {
        return;
    }
    let has_open_scene = actions.iter().any(|a| matches!(a, EditorAction::OpenScene));

    apply_actions(resources, actions, undo_stack);

    if has_open_scene && let Some(overlay) = resources.get_mut::<EditorOverlay>() {
        overlay.selected_entities.clear();
        // Pins name entities from the world that just went away. Entity
        // ids are generational, so a stale one cannot match a new
        // entity — but keeping them would grow the set for the life of
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
    // #463.2 — capture the start of the CPU-side render work so the
    // perf HUD can report `cpu_frame_ms` (excludes GPU + present).
    // The matching `record_cpu_frame_ms` call lives at the end of
    // this function.
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

    // The host's own output, which was captured and then never read by
    // anyone. Everything a mirrored project says — including every physics
    // event — happens over there, so without this the Console shows the
    // editor talking to itself.
    forward_remote_output(resources);

    if poll_launcher(resources) {
        return;
    }

    let project_loaded = resources
        .get::<ProjectState>()
        .is_some_and(|ps| ps.is_project_loaded());

    let (display_data, mut gather_stages) = if project_loaded {
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
    // #463.5 — same single-write site for the engine VRAM counter:
    // read the shared Arc<EngineVramTracker> and stamp the byte
    // total. Both end up in EditorPerfStats so the toolbar reads
    // exactly one Resource.
    let vram_bytes = resources
        .get::<std::sync::Arc<kooch_render::EngineVramTracker>>()
        .map(|t| t.bytes())
        .unwrap_or(0);
    if let Some(stats) = resources.get_mut::<crate::perf::EditorPerfStats>() {
        stats.gpu_frame_ms = meshlet_stats.gpu_frame_ms;
        stats.vram_tracked_bytes = vram_bytes;
        // #463.6 — three editor passes always run regardless of
        // scene contents: sky background, viewport blit, egui
        // paint. Gizmo line / mesh batches only emit when the
        // selection actually has something to visualize, so they
        // are NOT included in the fixed budget — counting them
        // here would inflate the number when there's nothing to
        // draw and confuse the artist who just despawned every
        // MeshRenderer. The meshlet stage already returns 0 when
        // there are no instances; combined with EDITOR_BASE_PASSES
        // the HUD shows a clean 3-draws floor in an empty scene.
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

    let toolbar = ToolbarInfo {
        can_undo: undo_stack.can_undo(),
        can_redo: undo_stack.can_redo(),
        undo_desc: undo_stack.undo_description().map(String::from),
        redo_desc: undo_stack.redo_description().map(String::from),
        remote: resources
            .get::<crate::remote_session::RemoteState>()
            .and_then(|s| s.session.as_ref().map(|s| s.state())),
        remote_stale: resources
            .get::<crate::remote_session::RemoteState>()
            .and_then(|s| s.session.as_ref()?.stale_reason().map(String::from)),
        // In remote mode the project runs gameplay in place, so Play
        // is a wire toggle rather than a launched process.
        is_playing: is_playing
            || resources
                .get::<crate::remote_session::RemoteState>()
                .is_some_and(|s| s.playing),
    };

    let raw_input = {
        let mut state = overlay.winit_state.lock().unwrap();
        state.take_egui_input(&window)
    };

    let mut viewport_request: Option<(u32, u32)> = None;
    let mut game_request: Option<(u32, u32)> = None;
    let mut game_focused = false;
    let mut viewport_input: Option<ViewportInputDelta> = None;
    let controller_snapshot = resources
        .get::<EditorCameraController>()
        .cloned()
        .unwrap_or_default();
    let power_profile = resources
        .get::<kooch_core::power::PowerProfile>()
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

    // Resolve the Asset Browser's selection into a data snapshot before
    // the egui frame — the detail pane needs the asset's contents, and
    // resolving them requires mutable `Resources` (AssetServer load).
    // Cloned out: the UI takes `&mut Resources` for other things, and
    // holding a borrow of one resource rules out asking for another.
    let open_input_map = resources.get::<crate::state::OpenInputMap>().cloned();
    let asset_detail = overlay
        .selected_asset
        .and_then(|guid| crate::systems::asset_detail::gather_asset_detail(guid, resources));
    gather_stages.assets_ms = crate::perf::ms_since(assets_start);

    // Lifted out for the frame: the Gizmos dropdown mutates it, and the
    // egui closure already holds Resources immutably. Groups are resolved
    // from the registry now rather than rebuilt inside the menu, so the
    // panel is a pure draw over data.
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

    // #691 — everything above was assembling what the UI is about to
    // read: the hierarchy, the inspector's view of it, the asset
    // catalog. It walks the world, so it grows with the scene.
    let mut stages = crate::perf::RenderStages {
        gather_ms: crate::perf::ms_since(frame_cpu_start),
        gather: gather_stages,
        ..Default::default()
    };

    let ui_start = std::time::Instant::now();
    let (full_output, mut actions) = run_editor_ui(
        &mut overlay,
        &mut project_state,
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
            game_focused: &mut game_focused,
            input: &mut viewport_input,
            controller: &controller_snapshot,
            handle_mode: resources
                .get::<kooch_gizmos_handles::HandleSet>()
                .map(|h| h.mode())
                .unwrap_or_default(),
        },
        power_profile,
        &asset_catalog,
        asset_detail.as_ref(),
        open_input_map.as_ref(),
        engine_root_owned.as_deref(),
        project_crate_root.as_deref(),
        &mut meshlet_debug_mode,
        meshlet_debug_caps,
        &mut meshlet_lod_settings,
        meshlet_stats,
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
    );
    stages.ui_ms = crate::perf::ms_since(ui_start);
    let input_start = std::time::Instant::now();

    // #656 — egui's own answer to "does anything need redrawing", read
    // before `full_output` is handed to the presenter and consumed.
    let ui_repaint_delay = shortest_repaint_delay(&full_output);

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

    if let Some(size) = viewport_request {
        viewport.request_size(size);
    }
    if let (Some(size), Some(game)) = (game_request, game_view.as_mut()) {
        game.target.request_size(size);
    }
    if let Some(game) = game_view.as_mut() {
        game.focused = game_focused;
    }

    // Apply viewport input to the editor camera before the same frame's
    // render pass so the new pose is visible immediately. Focus-on-
    // selection uses the first selected entity's world position, if any.
    //
    // First give the gizmo handle system a chance to absorb input. If a
    // handle is hovered or being dragged, suppress camera input so the
    // user doesn't inadvertently orbit while moving an entity.
    // Any camera motion at all, not only the fly keys: an orbit or pan
    // drag has the same problem if the pointer stops moving for a frame
    // while a button is still down.
    let driving_camera = viewport_input.is_some_and(|delta| {
        delta.fly_active
            || delta.fly_keys.any()
            || delta.orbit_yaw != 0.0
            || delta.orbit_pitch != 0.0
            || delta.pan_dx != 0.0
            || delta.pan_dy != 0.0
            || delta.zoom_lines != 0.0
    });

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
            // Clicking picks only when a gizmo did not take the click:
            // a handle sits *over* the thing it moves, so picking first
            // would select whatever is behind the arrow the user grabbed.
            apply_viewport_click(delta, resources, &mut overlay);

            let selection_world = overlay
                .selected_entities
                .first()
                .copied()
                .and_then(|e| entity_world_position(resources, e));
            apply_viewport_input(delta, resources, selection_world);
        }
    }

    stages.input_ms = crate::perf::ms_since(input_start);

    let viewport_start = std::time::Instant::now();

    // The Game panel renders first: a second view of the same stage,
    // through the gameplay camera. Before the View panel's pass rather
    // than after, so the two submits stay in a fixed order and a frame
    // capture always reads the same way.
    //
    // Skipped entirely when no project is loaded — there is no scene to
    // look at, and the panel says so rather than showing black.
    //
    // Also skipped when the panel is not on screen. `game_request` is
    // set by `draw_game_content`, so it is `Some` this frame iff the tab
    // was actually drawn: Game ships as a sibling tab of View, so the
    // common case is that only one of them is visible, and rendering
    // both would pay two culls a frame for a panel nobody is looking at.
    // The UI runs before this point in the frame, so the flag is already
    // current.
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
                    kooch_render::meshlet::MeshletRenderStageConfig::default(),
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

    stages.viewport_ms = crate::perf::ms_since(viewport_start);

    let present_start = std::time::Instant::now();
    let presented = present_editor_frame(&gpu, &mut overlay, &window, full_output);
    stages.present_ms = crate::perf::ms_since(present_start);

    resources.insert(gpu);
    resources.insert(overlay);
    resources.insert(viewport);
    if let Some(game) = game_view {
        resources.insert(game);
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
    stages.actions_ms = crate::perf::ms_since(actions_start);

    resources.insert(undo_stack);

    // #656 — say what the next frame needs, so the loop can stop when it
    // needs nothing. Actions are applied first: one of them may have
    // opened a project or started Play, and the frame that does so must
    // not go to sleep before the effect is visible. A frame that failed
    // to present asks for another unconditionally — the image on screen
    // is not the one this frame drew.
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

    // #463.2 — write the CPU side of the frame budget into the perf
    // HUD Resource. Last call so the elapsed measurement covers
    // every CPU branch above (early returns excepted; those are
    // wall-clock-trivial).
    record_cpu_frame_ms(resources, frame_cpu_start);
    // #691 — published after the total, so the residual the HUD derives
    // from the two is read from the same frame.
    crate::perf::record_render_stages(resources, stages);
}

/// Moves the mirrored project's stdout into the editor's log.
///
/// Tagged `[game]` like the spawned-process path, so one prefix means "not
/// the editor" however the project was started, and the Console's project
/// filter works for both.
fn forward_remote_output(resources: &mut Resources) {
    let Some(state) = resources.get::<crate::remote_session::RemoteState>() else {
        return;
    };
    let Some(session) = state.session.as_ref() else {
        return;
    };
    // Kept as well as forwarded, while the handshake is still in flight:
    // the log is where these belong, but the connecting banner needs
    // something to show and draining is destructive (#672). Once the
    // project answers, the Console is the place to read it and the copy
    // stops growing.
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
///
/// # Why not while playing
///
/// Play runs the project's gameplay in this same viewport, and a running
/// game wants its own clicks — aiming, shooting, pressing whatever is on
/// screen. Selecting an entity out from under that would fight the game for
/// the mouse, and the selection would be stale the moment Stop restores the
/// pre-play world anyway.
///
/// # Clicking nothing
///
/// Clears the selection, the way clicking empty space in the World panel
/// does. Leaving it alone would make an intentional deselect impossible
/// without finding a blank row in another panel.
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
