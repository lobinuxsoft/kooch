//! View panel — draws the viewport offscreen texture and reports the
//! desired backing texture size for the next frame.

/// Draws the viewport image filling the available panel area.
///
/// `texture_id` is the egui-side handle to the offscreen ray-march texture.
/// `request` is written with the desired backing texture size (in physical
/// pixels) so the render system can resize the offscreen texture next frame.
pub(crate) fn draw_view_content(
    ui: &mut egui::Ui,
    texture_id: egui::TextureId,
    request: &mut Option<(u32, u32)>,
) {
    let available = ui.available_size();
    let pixels_per_point = ui.ctx().pixels_per_point();

    let physical = (
        ((available.x * pixels_per_point).round() as i32).max(1) as u32,
        ((available.y * pixels_per_point).round() as i32).max(1) as u32,
    );
    *request = Some(physical);

    if available.x < 1.0 || available.y < 1.0 {
        return;
    }

    ui.image((texture_id, available));
}
