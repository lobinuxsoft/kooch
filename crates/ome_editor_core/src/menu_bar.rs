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
) {
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
