//! What the viewport responds to, in the corner (#1436).
//!
//! The editor had a dozen keyboard commands and nothing on screen said so: the only way to find
//! `Shift+E` or `Ctrl+Shift` was to read the source or be told. A tool whose gestures are
//! invisible is a tool nobody uses past the one they were shown.

use crate::commands::Command;

/// Distance from the viewport's bottom-left corner.
const OFFSET: egui::Vec2 = egui::vec2(8.0, 8.0);

/// Width of the chord column. Fixed, so the verbs line up instead of stepping with every row's
/// key — a ragged list is read one row at a time, an aligned one at a glance.
const CHORD_WIDTH: f32 = 96.0;

/// Draws the list over the bottom-left of `viewport`.
pub(super) fn draw(
    ui: &mut egui::Ui,
    viewport: egui::Rect,
    selection_has_transform: bool,
    editing_a_block: bool,
    editing_a_spline: bool,
) {
    let groups =
        crate::commands::for_context(selection_has_transform, editing_a_block, editing_a_spline);
    if groups.is_empty() {
        return;
    }

    // Built upwards from the bottom edge: the list grows as the selection gains tools, and
    // anchoring the top would make it crawl down the screen as it does.
    let anchor = egui::Rect::from_min_max(
        egui::pos2(viewport.min.x, viewport.min.y) + OFFSET,
        egui::pos2(viewport.max.x, viewport.max.y) - OFFSET,
    );
    let mut overlay = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(anchor)
            .layout(egui::Layout::bottom_up(egui::Align::LEFT)),
    );
    // 🔴 The overlay must not eat the viewport's input. It sits over the image that owns the
    // camera drag, and a panel that answers the pointer would swallow an orbit started on top of
    // it.
    overlay.disable();

    egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(20, 20, 24, 170))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::same(8))
        .show(&mut overlay, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            // Reversed, because the layout stacks bottom-up and the groups are ordered by how
            // close each is to what the author is holding.
            for (index, (title, commands)) in groups.iter().enumerate().rev() {
                for command in commands.iter().rev() {
                    row(ui, *command);
                }
                ui.label(egui::RichText::new(*title).small().strong());
                if index > 0 {
                    ui.add_space(4.0);
                }
            }
        });
}

fn row(ui: &mut egui::Ui, command: Command) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(CHORD_WIDTH, ui.spacing().interact_size.y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new(command.label())
                        .small()
                        .monospace()
                        .color(egui::Color32::from_rgb(230, 190, 120)),
                );
            },
        );
        ui.label(
            egui::RichText::new(command.what)
                .small()
                .color(egui::Color32::from_gray(205)),
        );
    });
}
