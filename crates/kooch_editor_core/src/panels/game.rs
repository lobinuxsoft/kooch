//! Game panel — the scene through the gameplay camera.
//!
//! Deliberately bare next to the View panel: no handle toolbar, no
//! debug-mode dropdown, no gizmo toggles. Those are authoring controls,
//! and this panel exists to answer "what does the player see". Anything
//! drawn here that the player would not see makes the answer wrong.

/// Draws the game image, or says why there is nothing to draw.
pub(crate) fn draw_game_content(
    ui: &mut egui::Ui,
    texture_id: egui::TextureId,
    size_request: &mut Option<(u32, u32)>,
    has_camera: bool,
) {
    let available = ui.available_size();
    // Physical pixels: the offscreen target is sized in them, and on a
    // fractional-scale desktop using points would render at the wrong
    // resolution and resample.
    let pixels_per_point = ui.ctx().pixels_per_point();
    let requested = (
        (available.x * pixels_per_point).round().max(1.0) as u32,
        (available.y * pixels_per_point).round().max(1.0) as u32,
    );
    *size_request = Some(requested);

    if !has_camera {
        // A black rectangle would read as "the game renders black".
        // Saying which component is missing turns a puzzle into a task.
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new(
                    "No active gameplay camera.\n\nAdd a PerspectiveCamera component to an \
                     entity to see the game here.",
                )
                .weak(),
            );
        });
        return;
    }

    ui.add(egui::Image::new((texture_id, available)));
}
