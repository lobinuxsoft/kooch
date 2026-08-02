//! Editor menu bar drawing.

use egui_dock::DockState;
use kooch_core::power::PowerProfile;

use crate::actions::EditorAction;
use crate::icons;
use crate::remote_session::ConnectionState;
use crate::state::{ALL_TABS, EditorTab, dock_has_tab};

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_menu_bar(
    ui: &mut egui::Ui,
    dock_state: &mut DockState<EditorTab>,
    actions: &mut Vec<EditorAction>,
    is_playing: bool,
    remote: Option<ConnectionState>,
    remote_stale: Option<&str>,
    can_undo: bool,
    can_redo: bool,
    undo_desc: Option<&str>,
    redo_desc: Option<&str>,
    power_profile: PowerProfile,
    ide_command: Option<&str>,
) {
    // Keyboard shortcuts — check before any UI so they work regardless of focus.
    let ctrl_z = ui
        .ctx()
        .input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Z));
    let ctrl_y = ui
        .ctx()
        .input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Y));

    if ctrl_z && can_undo {
        actions.push(EditorAction::Undo);
    }
    if ctrl_y && can_redo {
        actions.push(EditorAction::Redo);
    }

    // `Panel::top` in egui 0.35: `SidePanel` and `TopBottomPanel` were
    // unified into one `Panel` type (egui #5659).
    egui::Panel::top("editor_menu").show(ui, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Save Scene...").clicked() {
                    actions.push(EditorAction::SaveScene);
                    ui.close();
                }
                if ui.button("Open Scene...").clicked() {
                    actions.push(EditorAction::OpenScene);
                    ui.close();
                }
                // Disabled while driving a project: the world here is a
                // mirror of the project's, and an additive load would
                // spawn entities that exist only on this side. They would
                // be invisible in the game and every edit to them would be
                // dropped for not being in the mirror. Loading a second
                // scene into the project needs the project to do it — see
                // the issue linked from the tooltip.
                let mirroring = remote == Some(ConnectionState::Connected);
                let additive = ui
                    .add_enabled(!mirroring, egui::Button::new("Open Scene Additive..."))
                    .on_hover_text(if mirroring {
                        "Unavailable while a project is open — the world \
                         shown here mirrors the project, and a scene loaded \
                         on this side would not exist in it"
                    } else {
                        "Load a scene beside the ones already open"
                    });
                if additive.clicked() {
                    actions.push(EditorAction::OpenSceneAdditive);
                    ui.close();
                }
                ui.separator();
                if ui
                    .button("Clean Project")
                    .on_hover_text(
                        "Run cargo clean: deletes the build output and nothing else. \
                         The project has to be rebuilt afterwards.",
                    )
                    .clicked()
                {
                    actions.push(EditorAction::CleanProject);
                    ui.close();
                }
                if ui.button("Close Project").clicked() {
                    actions.push(EditorAction::CloseProject);
                    ui.close();
                }
            });
            ui.menu_button("Edit", |ui| {
                let undo_label = match undo_desc {
                    Some(desc) => format!("Undo {desc}  Ctrl+Z"),
                    None => "Undo  Ctrl+Z".to_owned(),
                };
                if ui
                    .add_enabled(can_undo, egui::Button::new(undo_label))
                    .clicked()
                {
                    actions.push(EditorAction::Undo);
                    ui.close();
                }

                let redo_label = match redo_desc {
                    Some(desc) => format!("Redo {desc}  Ctrl+Y"),
                    None => "Redo  Ctrl+Y".to_owned(),
                };
                if ui
                    .add_enabled(can_redo, egui::Button::new(redo_label))
                    .clicked()
                {
                    actions.push(EditorAction::Redo);
                    ui.close();
                }
            });
            ui.menu_button("Engine", |ui| {
                ui.menu_button(format!("Power Profile: {}", power_profile.as_str()), |ui| {
                    for option in [
                        PowerProfile::Plugged,
                        PowerProfile::Balanced,
                        PowerProfile::Battery,
                        PowerProfile::Debug,
                    ] {
                        if ui
                            .selectable_label(power_profile == option, option.as_str())
                            .clicked()
                        {
                            actions.push(EditorAction::SetPowerProfile(option));
                            ui.close();
                        }
                    }
                });
            });
            ui.menu_button("Window", |ui| {
                for &tab in ALL_TABS {
                    let is_open = dock_has_tab(dock_state, &tab);
                    if ui.selectable_label(is_open, tab.label()).clicked() {
                        if is_open {
                            dock_state.retain_tabs(|t| *t != tab);
                        } else {
                            dock_state.add_window(vec![tab]);
                        }
                        ui.close();
                    }
                }
            });
            // A button, not a menu with a form in it. An egui menu closes
            // on any click it does not consume, so a text field inside one
            // shuts the menu the moment it is clicked — which made the IDE
            // path impossible to type. A menu picks an action; a form
            // belongs in a window.
            if ui.button("Settings").clicked() {
                ui.data_mut(|d| d.insert_temp(settings_open_id(ui.ctx()), true));
            }

            // Push Play/Stop buttons to the center of the remaining space.
            let available = ui.available_width();
            let button_width = 70.0;
            let total_buttons = button_width * 2.0 + ui.spacing().item_spacing.x;
            let offset = (available - total_buttons) / 2.0;
            if offset > 0.0 {
                ui.add_space(offset);
            }

            if ui
                .add_enabled(
                    !is_playing,
                    egui::Button::new(format!("{} Play", icons::PLAY))
                        .min_size(egui::vec2(button_width, 0.0)),
                )
                .clicked()
            {
                actions.push(EditorAction::Play);
            }
            if ui
                .add_enabled(
                    is_playing,
                    egui::Button::new(format!("{} Stop", icons::STOP))
                        .min_size(egui::vec2(button_width, 0.0)),
                )
                .clicked()
            {
                actions.push(EditorAction::Stop);
            }

            // Remote status, right-aligned: a project build takes long
            // enough that a silent editor reads as a hang.
            if let Some(remote) = remote {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    draw_remote_status(ui, remote, remote_stale, actions);
                });
            }
        });
    });
}

/// Draws the remote session indicator and its rebuild control.
///
/// `stale` is why the snapshot stopped tracking the project. It overrides
/// the connected look: the socket being up says nothing about whether what
/// is on screen still matches the world, and a mirror that quietly stopped
/// updating is indistinguishable from a world where nothing is moving.
fn draw_remote_status(
    ui: &mut egui::Ui,
    remote: ConnectionState,
    stale: Option<&str>,
    actions: &mut Vec<EditorAction>,
) {
    let stale_hover;
    let (icon, text, color, hover) = match (remote, stale) {
        (ConnectionState::Connected, Some(reason)) => {
            stale_hover = format!(
                "The project stopped answering with a readable world, so this is the \
                 last one the editor could read — edits from here may not land.\n\n{reason}",
            );
            (
                // The same glyph the Inspector uses for a warning; the
                // icon font has no triangle of its own.
                "\u{26a0}",
                "Stale",
                egui::Color32::from_rgb(210, 150, 60),
                stale_hover.as_str(),
            )
        }
        _ => remote_status_look(remote),
    };
    // Right-to-left layout: this lands to the left of the status text.
    if ui
        .add_enabled(
            remote != ConnectionState::Connecting,
            egui::Button::new(icons::ARROWS_CLOCKWISE),
        )
        .on_hover_text(
            "Rebuild & Relaunch — recompiles the project and reconnects. \
             Needed to pick up code added since it started, and the way \
             back from a project that exited.",
        )
        .clicked()
    {
        actions.push(EditorAction::RebuildRemote);
    }
    ui.label(egui::RichText::new(format!("{icon} {text}")).color(color))
        .on_hover_text(hover);
}

/// How each handshake state reads, before staleness is taken into account.
fn remote_status_look(
    remote: ConnectionState,
) -> (&'static str, &'static str, egui::Color32, &'static str) {
    match remote {
        ConnectionState::Connecting => (
            icons::GEAR,
            "Connecting",
            egui::Color32::from_rgb(210, 180, 90),
            "Building and starting the project. The world appears once it answers.",
        ),
        ConnectionState::Connected => (
            icons::ROCKET,
            "Remote",
            egui::Color32::from_rgb(100, 200, 100),
            "Editing the project's live world. Edits are applied by the project, not here.",
        ),
        ConnectionState::Failed => (
            icons::X,
            "Disconnected",
            egui::Color32::from_rgb(200, 80, 80),
            "The project exited before answering. Check its build output in the terminal.",
        ),
    }
}

/// Where the Settings window's open flag lives.
fn settings_open_id(ctx: &egui::Context) -> egui::Id {
    let _ = ctx;
    egui::Id::new("kooch_settings_window_open")
}

/// The Settings window: editor preferences that are not per-project.
///
/// A window rather than a menu, because a menu closes on any click it
/// does not consume and a text field inside one cannot be typed into.
/// It is also movable and resizable, which a path worth pasting needs.
pub(crate) fn draw_settings_window(
    ctx: &egui::Context,
    actions: &mut Vec<EditorAction>,
    ide_command: Option<&str>,
) {
    let id = settings_open_id(ctx);
    let mut open = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    if !open {
        return;
    }

    let buffer_id = id.with("ide_cmd_buffer");
    let mut buf: String = ctx
        .data(|d| d.get_temp::<String>(buffer_id))
        .unwrap_or_else(|| ide_command.unwrap_or_default().to_owned());

    egui::Window::new("Settings")
        .open(&mut open)
        .resizable(true)
        .default_width(460.0)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.label("IDE command — opens the project folder, and the file if it can:");
            if ui.text_edit_singleline(&mut buf).changed() {
                ctx.data_mut(|d| d.insert_temp(buffer_id, buf.clone()));
            }
            ui.weak(
                "A full path is safest: an IDE installed by Flatpak, Homebrew or an \
                 AppImage is usually not on this process's PATH. Arguments are fine, \
                 e.g. 'flatpak run com.vscodium.codium'.",
            );

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Apply").clicked() {
                    let trimmed = buf.trim();
                    let command = (!trimmed.is_empty()).then(|| trimmed.to_owned());
                    actions.push(EditorAction::SetIdeCommand { command });
                    ctx.data_mut(|d| d.remove::<String>(buffer_id));
                }
                // Fills the box rather than applying, so what was found can
                // be read and edited before it is committed.
                if ui.button("Detect").clicked()
                    && let Some(found) = crate::actions::detected_ide_command()
                {
                    buf = found;
                    ctx.data_mut(|d| d.insert_temp(buffer_id, buf.clone()));
                }
                if ui.button("Clear").clicked() {
                    actions.push(EditorAction::SetIdeCommand { command: None });
                    buf.clear();
                    ctx.data_mut(|d| d.remove::<String>(buffer_id));
                }
            });

            ui.add_space(4.0);
            match ide_command {
                Some(current) => ui.weak(format!("In use: {current}")),
                None => ui.weak("In use: whatever the desktop says opens a source file."),
            };
        });

    ctx.data_mut(|d| d.insert_temp(id, open));
}
