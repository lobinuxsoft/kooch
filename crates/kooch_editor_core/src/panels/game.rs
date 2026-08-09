//! Game panel — the scene through the gameplay camera.
//!
//! Deliberately bare next to the View panel: no handle toolbar, no
//! debug-mode dropdown, no gizmo toggles. Those are authoring controls,
//! and this panel exists to answer "what does the player see". Anything
//! drawn here that the player would not see makes the answer wrong.

/// Draws the game image with the perf sidebar over it, or says why
/// there is nothing to draw.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_game_content(
    ui: &mut egui::Ui,
    texture_id: egui::TextureId,
    size_request: &mut Option<(u32, u32)>,
    has_camera: bool,
    perf_stats: crate::perf::EditorPerfStats,
    meshlet_stats: kooch_render::meshlet::MeshletRenderStats,
    meshlet_debug_mode: &mut kooch_render::meshlet::MeshletDebugMode,
    meshlet_debug_caps: kooch_render::meshlet::MeshletDebugCaps,
    single_light_note: Option<&str>,
    meshlet_lod_settings: &mut kooch_render::meshlet::MeshletLodSettings,
    hud_visibility: &mut crate::perf::HudVisibility,
) {
    let available = ui.available_size();
    let panel_origin = ui.cursor().min;
    // Physical pixels: the offscreen target is sized in them, and on a
    // fractional-scale desktop using points would render at the wrong
    // resolution and resample.
    let pixels_per_point = ui.ctx().pixels_per_point();
    let requested = (
        (available.x * pixels_per_point).round().max(1.0) as u32,
        (available.y * pixels_per_point).round().max(1.0) as u32,
    );
    *size_request = Some(requested);

    // The sidebar draws over the image, so the image goes down first —
    // and it draws even without a camera, because "no camera" is exactly
    // when you want to see that the frame costs nothing.
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
    } else {
        ui.add(egui::Image::new((texture_id, available)));
    }

    if available.x >= 1.0 && available.y >= 1.0 {
        crate::panels::performance::draw_perf_sidebar(
            ui,
            panel_origin,
            available,
            perf_stats,
            meshlet_stats,
            meshlet_debug_mode,
            meshlet_debug_caps,
            meshlet_lod_settings,
            hud_visibility,
            single_light_note,
        );
    }
}
