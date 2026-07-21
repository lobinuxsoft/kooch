//! Editor menu bar drawing.

use egui_dock::DockState;
use ome_core::power::PowerProfile;

use crate::actions::EditorAction;
use crate::icons;
use crate::remote_session::ConnectionState;
use crate::state::{ALL_TABS, EditorTab, dock_has_tab};

// Migrating off `TopBottomPanel::top(...).show(ctx, ...)` requires
// adopting the eframe 0.34+ `App::ui(&mut self, ui: &mut Ui)` pattern,
// which is a structural change to the editor's render loop. Out of
// scope for the #299 cleanup.
#[allow(deprecated)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_menu_bar(
    ctx: &egui::Context,
    dock_state: &mut DockState<EditorTab>,
    actions: &mut Vec<EditorAction>,
    is_playing: bool,
    remote: Option<ConnectionState>,
    can_undo: bool,
    can_redo: bool,
    undo_desc: Option<&str>,
    redo_desc: Option<&str>,
    power_profile: PowerProfile,
    ide_command: Option<&str>,
) {
    // Keyboard shortcuts — check before any UI so they work regardless of focus.
    let ctrl_z = ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Z));
    let ctrl_y = ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Y));

    if ctrl_z && can_undo {
        actions.push(EditorAction::Undo);
    }
    if ctrl_y && can_redo {
        actions.push(EditorAction::Redo);
    }

    egui::TopBottomPanel::top("editor_menu").show(ctx, |ui| {
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
                ui.separator();
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
            ui.menu_button("Settings", |ui| {
                ui.label("IDE command (opens project + file):");
                let id = ui.id().with("ide_cmd_buffer");
                let mut buf: String = ui
                    .data(|d| d.get_temp::<String>(id))
                    .unwrap_or_else(|| ide_command.unwrap_or_default().to_owned());
                if ui.text_edit_singleline(&mut buf).changed() {
                    ui.data_mut(|d| d.insert_temp(id, buf.clone()));
                }
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        let trimmed = buf.trim();
                        let command = (!trimmed.is_empty()).then(|| trimmed.to_owned());
                        actions.push(EditorAction::SetIdeCommand { command });
                        ui.data_mut(|d| d.remove::<String>(id));
                        ui.close();
                    }
                    if ui.button("Auto-detect").clicked() {
                        actions.push(EditorAction::SetIdeCommand { command: None });
                        ui.data_mut(|d| d.remove::<String>(id));
                        ui.close();
                    }
                });
                ui.weak("Blank = codium / code. Args OK: 'flatpak run com.vscodium.codium'.");
            });

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
                    draw_remote_status(ui, remote, actions);
                });
            }
        });
    });
}

/// Draws the remote session indicator and its rebuild control.
fn draw_remote_status(ui: &mut egui::Ui, remote: ConnectionState, actions: &mut Vec<EditorAction>) {
    let (icon, text, color, hover) = match remote {
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
