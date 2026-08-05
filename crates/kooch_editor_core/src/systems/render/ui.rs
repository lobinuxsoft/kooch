//! Egui UI pass for the editor — runs the dock area, menu bar, and
//! launch screen depending on whether a project is loaded.

use egui_dock::DockArea;

use kooch_render::meshlet::{
    MeshletDebugCaps, MeshletDebugMode, MeshletLodSettings, MeshletRenderStats,
};

use crate::actions::{EditorAction, PendingPrefabOverwrite};
use crate::editor_camera::EditorCameraController;
use crate::editor_camera::input::ViewportInputDelta;
use crate::icons;
use crate::launch_screen::{self, LaunchAction};
use crate::menu_bar::draw_menu_bar;
use crate::project_state::ProjectState;
use crate::remote_session::ConnectionState;
use crate::state::EditorOverlay;
use crate::systems::tab_viewer::EditorTabViewer;

use super::frame_display::FrameDisplayData;

/// Toolbar/menu-bar state gathered before the egui pass: what the undo
/// stack can offer, and the two modes the toolbar reports on.
pub(super) struct ToolbarInfo {
    pub(super) can_undo: bool,
    pub(super) can_redo: bool,
    pub(super) undo_desc: Option<String>,
    pub(super) redo_desc: Option<String>,
    pub(super) is_playing: bool,
    /// Where the remote session stands, or `None` in local mode.
    pub(super) remote: Option<ConnectionState>,
    /// Why the remote snapshot stopped tracking, when it has.
    pub(super) remote_stale: Option<String>,
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
    pub(super) handle_mode: kooch_gizmos_handles::HandleMode,
    /// The Game panel's texture + size request. A separate view of the
    /// same stage (#592), so it carries its own size — the two panels
    /// are docked independently and rarely agree on one.
    pub(super) game_texture_id: egui::TextureId,
    pub(super) game_request: &'a mut Option<(u32, u32)>,
    pub(super) game_has_camera: bool,
    pub(super) input_owner: &'a mut crate::input_focus::InputOwner,
}

/// Runs the egui UI for one frame. Produces the tessellation input and the
/// editor actions queued by widgets.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_editor_ui(
    overlay: &mut EditorOverlay,
    project_state: &mut Option<ProjectState>,
    raw_input: egui::RawInput,
    project_loaded: bool,
    data: &FrameDisplayData,
    toolbar: &ToolbarInfo,
    viewport: ViewportUi<'_>,
    power_profile: kooch_core::power::PowerProfile,
    asset_catalog: &[crate::panels::inspector::AssetCatalogEntry],
    asset_detail: Option<&crate::panels::inspector::AssetDetail>,
    open_input_map: Option<&crate::state::OpenInputMap>,
    engine_assets_root: Option<&std::path::Path>,
    project_assets_root: Option<&std::path::Path>,
    meshlet_debug_mode: &mut MeshletDebugMode,
    meshlet_debug_caps: MeshletDebugCaps,
    meshlet_lod_settings: &mut MeshletLodSettings,
    meshlet_stats: MeshletRenderStats,
    perf_stats: crate::perf::EditorPerfStats,
    gizmo_visibility: &mut crate::gizmos::GizmoVisibility,
    gizmo_groups: &[crate::gizmos::GizmoGroup],
    physics_debug: &mut kooch_physics::backend::DebugCategories,
    hud_visibility: &mut crate::perf::HudVisibility,
    log_buffer: Option<&kooch_core::LogBuffer>,
    console: &mut crate::panels::console::ConsoleState,
    connect_output: &[String],
    prefab_overwrite: Option<&PendingPrefabOverwrite>,
) -> (egui::FullOutput, Vec<EditorAction>) {
    let mut selected = std::mem::take(&mut overlay.selected_entities);
    let mut pinned_gizmos = std::mem::take(&mut overlay.pinned_gizmos);
    let mut selected_asset = overlay.selected_asset;
    let mut current_folder = overlay.current_folder.take();
    let selected_before = selected.clone();
    let asset_before = selected_asset;
    let mut last_clicked_index = overlay.last_clicked_index.take();
    let mut actions: Vec<EditorAction> = Vec::new();
    let ViewportUi {
        texture_id,
        request,
        input,
        controller,
        handle_mode,
        game_texture_id,
        game_request,
        game_has_camera,
        input_owner,
    } = viewport;

    // `run_ui` rather than `run`: egui 0.35 hands the closure a root `Ui`
    // instead of the `Context`, and panels are placed inside a `Ui` now
    // rather than onto a context (egui #5659, #7781, #7783).
    let full_output = overlay.ctx.run_ui(raw_input, |ui| {
        if project_loaded {
            draw_menu_bar(
                ui,
                &mut overlay.dock_state,
                &mut actions,
                toolbar.is_playing,
                toolbar.remote,
                toolbar.remote_stale.as_deref(),
                toolbar.can_undo,
                toolbar.can_redo,
                toolbar.undo_desc.as_deref(),
                toolbar.redo_desc.as_deref(),
                power_profile,
                project_state
                    .as_ref()
                    .and_then(|ps| ps.editor_config.ide_command.as_deref()),
            );

            // Drawn on the context rather than inside a panel: a window is
            // free-floating, and nesting it in the menu bar's `Ui` would
            // clip it to that strip.
            crate::menu_bar::draw_settings_window(
                ui.ctx(),
                &mut actions,
                project_state
                    .as_ref()
                    .and_then(|ps| ps.editor_config.ide_command.as_deref()),
            );

            // A build is running and the dock has nothing to show yet.
            // The toolbar chip alone read as "dead" — a static gear, easy
            // to miss, and silent about what it was doing (#672).
            if toolbar.remote == Some(ConnectionState::Connecting) {
                draw_connecting_banner(ui, connect_output);
            }

            let selection_has_transform = data.entities.iter().any(|info| {
                selected.contains(&info.entity)
                    && info.components.iter().any(|c| c.short_name == "Transform")
            });
            // Opening an asset has to show it. A panel that loaded the
            // map behind a tab nobody switched to is indistinguishable
            // from one that did nothing.
            if open_input_map.is_some_and(|open| open.focus_requested) {
                if !crate::state::dock_has_tab(
                    &overlay.dock_state,
                    &crate::state::EditorTab::InputMap,
                ) {
                    overlay
                        .dock_state
                        .add_window(vec![crate::state::EditorTab::InputMap]);
                }
                actions.push(EditorAction::InputMapFocused);
            }

            let mut tab_viewer = EditorTabViewer {
                pinned: &mut pinned_gizmos,
                focused_tab: &mut overlay.focused_tab,
                accent: ui.visuals().selection.bg_fill,
                asset_nav: &mut overlay.asset_nav,
                inspector_nav: &mut overlay.inspector_nav,
                entities: &data.entities,
                scenes: &data.scenes,
                archetypes: &data.archetypes,
                component_types: &data.component_types,
                selected: &mut selected,
                reflected_types: &data.reflected_types,
                actions: &mut actions,
                gizmo_visibility,
                physics_debug,
                hud_visibility,
                log_buffer,
                console,
                gizmo_groups,
                entity_count: data.entity_count,
                archetype_count: data.archetype_count,
                active_archetype_count: data.active_archetype_count,
                last_clicked_index: &mut last_clicked_index,
                viewport_texture_id: texture_id,
                viewport_request: request,
                game_texture_id,
                game_request,
                game_has_camera,
                input_owner,
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
                open_input_map,
                current_folder: &mut current_folder,
                engine_assets_root,
                project_assets_root,
                meshlet_debug_mode,
                meshlet_debug_caps,
                meshlet_lod_settings,
                meshlet_stats,
                perf_stats,
            };

            // While the project is still building, the world these panels
            // edit has not arrived. `apply_actions` already refuses those
            // edits, but a dock that looks live and silently swallows
            // clicks reads as a broken editor rather than a busy one.
            let editable = toolbar.remote != Some(ConnectionState::Connecting);
            let dock_rect = ui.available_rect_before_wrap();

            DockArea::new(&mut overlay.dock_state)
                .style(egui_dock::Style::from_egui(ui.style().as_ref()))
                .show_inside(ui, &mut tab_viewer);

            if !editable {
                shade_out(ui, dock_rect);
            }
        } else if let Some(ps) = project_state.as_mut() {
            let launch_actions = launch_screen::draw_launch_screen(ui, ps);
            forward_launch_actions(launch_actions, &mut actions);
        }

        if let Some(pending) = prefab_overwrite {
            draw_prefab_overwrite_prompt(ui, pending, &mut actions);
        }
    });

    // Selection arbitration — the Inspector renders one thing. Picking
    // an asset this frame drops the entity selection; picking an entity
    // drops the asset selection. Keeps entity + asset selection mutually
    // exclusive without threading either into the other's panel.
    if selected_asset != asset_before && selected_asset.is_some() {
        selected.clear();
    }
    if selected != selected_before && !selected.is_empty() {
        selected_asset = None;
    }

    overlay.selected_entities = selected;
    overlay.pinned_gizmos = pinned_gizmos;
    overlay.selected_asset = selected_asset;
    overlay.current_folder = current_folder;
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

/// Says that a project is being built, and what it is building.
///
/// Opening a project compiles it — twenty-two seconds on this session's
/// own log — and until it answers the dock is furniture. A spinner is the
/// part that matters: motion is what distinguishes "working" from "hung",
/// which a static icon cannot do however well labelled.
fn draw_connecting_banner(ui: &mut egui::Ui, output: &[String]) {
    let frame = egui::Frame::group(ui.style()).fill(egui::Color32::from_rgb(48, 40, 16));
    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(14.0));
            ui.label(
                egui::RichText::new("Building the project")
                    .strong()
                    .color(egui::Color32::from_rgb(230, 205, 130)),
            );

            // Cargo's own words, not a percentage we would have to invent:
            // "Compiling kooch_render" is true, and a bar would not be.
            if let Some(last) = output.iter().rev().find(|line| !line.trim().is_empty()) {
                ui.label(
                    egui::RichText::new(last.trim())
                        .monospace()
                        .size(11.0)
                        .weak(),
                );
            }

            // The output is here rather than only in the Console, because
            // this is the surface a user is already looking at while they
            // wait — and if the build fails, what they need to send on.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !output.is_empty(),
                        egui::Button::new(format!("{} Copy", crate::icons::COPY)),
                    )
                    .on_hover_text("Copy the build output so far")
                    .clicked()
                {
                    ui.ctx().copy_text(output.join("\n"));
                }
            });
        });
    });
    // Without this the spinner freezes between input events: an idle
    // editor sleeps now (#656), and a spinner that does not move is worse
    // than no spinner at all.
    ui.ctx().request_repaint();
}

/// Covers `rect` with a layer that dims it and swallows the pointer.
///
/// `add_enabled_ui` around the dock was the obvious try and does nothing:
/// `egui_dock` renders each tab body into a `UiBuilder` carrying its own
/// `layer_id` (`leaf.rs:316`), and a parent `Ui`'s disabled flag does not
/// reach a new layer. So the panels stayed live and clickable while the
/// banner above them said the project was still building.
///
/// A foreground layer needs no cooperation from anything underneath: it is
/// above every dock layer, and a response sensing clicks and drags over
/// the whole rect means nothing below ever sees them.
///
/// Deliberately only the dock's rect. The menu bar keeps Cancel, Close and
/// Clean Project, and the banner keeps its Copy — those are how a user
/// gets out of a build that never finishes.
/// Asks before replacing an existing prefab file.
///
/// # Why this is asked rather than avoided
///
/// Saving a prefab again after editing the entity is how a prefab is
/// iterated on, so the file has to be replaced. Suffixing instead — the
/// previous behaviour — never destroyed anything and made that impossible,
/// leaving `Enemy_1`, `Enemy_2`, `Enemy_3` behind and no updated `Enemy`.
///
/// So the destructive thing is the correct thing, and a prompt is what
/// makes it safe. A modal rather than an inline confirmation because it is
/// answering for a file the user cannot see from here.
fn draw_prefab_overwrite_prompt(
    ui: &egui::Ui,
    pending: &PendingPrefabOverwrite,
    actions: &mut Vec<EditorAction>,
) {
    let name = pending
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| pending.path.display().to_string());

    let mut answered = None;
    egui::Modal::new(egui::Id::new("prefab_overwrite_prompt")).show(ui.ctx(), |ui| {
        ui.set_width(360.0);
        ui.heading(format!("{} Replace prefab?", icons::PACKAGE));
        ui.add_space(6.0);
        // The file name, not "a prefab": the user is answering about
        // something they can recognise.
        ui.label(format!("{name} already exists."));
        ui.add_space(2.0);
        ui.weak("Its contents are replaced. Anything already referencing it keeps working — the file keeps its id.");
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                answered = Some(false);
            }
            if ui.button("Replace").clicked() {
                answered = Some(true);
            }
        });
    });

    // Both answers are actions: this function draws, and the action layer
    // owns the pending state. Cancelling still has to be said out loud, or
    // the prompt would come back next frame.
    match answered {
        Some(true) => actions.push(EditorAction::SavePrefab {
            entity: pending.entity,
            dest: pending.dest.clone(),
            overwrite: true,
        }),
        Some(false) => actions.push(EditorAction::CancelPrefabOverwrite),
        None => {}
    }
}

fn shade_out(ui: &egui::Ui, rect: egui::Rect) {
    let area = egui::Area::new(ui.id().with("dock_shade"))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min)
        .interactable(true);

    area.show(ui.ctx(), |ui| {
        // The response is the functional half: allocated over the whole
        // rect so the pointer lands here and not on a panel.
        ui.allocate_response(rect.size(), egui::Sense::click_and_drag());
        ui.painter().rect_filled(
            rect,
            0.0,
            // Dark enough to read as "not now", light enough that the
            // panels stay legible — a user waiting on a build still wants
            // to see the Console scroll past.
            egui::Color32::from_black_alpha(110),
        );
    });
}
