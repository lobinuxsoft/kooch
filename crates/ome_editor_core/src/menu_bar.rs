//! Editor menu bar drawing.

use egui_dock::DockState;

use crate::actions::EditorAction;
use crate::icons;
use crate::state::{dock_has_tab, EditorTab, ALL_TABS};

pub(crate) fn draw_menu_bar(
    ctx: &egui::Context,
    dock_state: &mut DockState<EditorTab>,
    actions: &mut Vec<EditorAction>,
    is_playing: bool,
    can_undo: bool,
    can_redo: bool,
    undo_desc: Option<&str>,
    redo_desc: Option<&str>,
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
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Save Scene...").clicked() {
                    actions.push(EditorAction::SaveScene);
                    ui.close_menu();
                }
                if ui.button("Open Scene...").clicked() {
                    actions.push(EditorAction::OpenScene);
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Close Project").clicked() {
                    actions.push(EditorAction::CloseProject);
                    ui.close_menu();
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
                    ui.close_menu();
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
                    ui.close_menu();
                }
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
                        ui.close_menu();
                    }
                }
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
        });
    });
}
