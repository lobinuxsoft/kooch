//! View panel — placeholder viewport.

/// Content of the "View" tab — placeholder viewport.
pub(crate) fn draw_view_content(ui: &mut egui::Ui) {
    ui.centered_and_justified(|ui| {
        ui.weak("Viewport — scene rendering will go here");
    });
}
