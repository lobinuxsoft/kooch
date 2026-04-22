//! View panel — draws the viewport offscreen texture and reports the
//! desired backing texture size for the next frame.

use crate::editor_camera::input::{ViewportInputDelta, collect_viewport_input};
use crate::editor_camera::EditorCameraController;

/// Draws the viewport image filling the available panel area.
///
/// `texture_id` is the egui-side handle to the offscreen ray-march texture.
/// `request` is written with the desired backing texture size (in physical
/// pixels) so the render system can resize the offscreen texture next frame.
/// `input` is written with the viewport input delta this frame, consumed
/// after egui closes by the editor camera controller.
pub(crate) fn draw_view_content(
    ui: &mut egui::Ui,
    texture_id: egui::TextureId,
    request: &mut Option<(u32, u32)>,
    input: &mut Option<ViewportInputDelta>,
    controller: &EditorCameraController,
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

    // Allocate an interactive image so MMB/RMB drags, scroll and the F
    // key can be captured by the editor camera input layer. The same
    // widget paints the offscreen texture and acts as the input target.
    let response = ui.add(
        egui::Image::new((texture_id, available)).sense(egui::Sense::click_and_drag()),
    );

    *input = Some(collect_viewport_input(&response, ui, controller));
}
