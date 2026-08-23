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
    lights_hot: &mut kooch_lighting::LightsHot,
    cluster_settings: &mut kooch_lighting::ClusterSettings,
    specular_floor: &mut kooch_lighting::SpecularFloor,
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
        // Godot's viewport grammar: a small View menu in the top-left
        // deciding which anchored overlays draw, each in its own fixed
        // corner where it does not cover the action.
        view_menu(ui, panel_origin, hud_visibility);
        if hud_visibility.frame_time_card {
            frame_time_card(ui, panel_origin, available, &perf_stats);
        }
        if hud_visibility.info_card {
            info_card(ui, panel_origin, available, &perf_stats, requested);
        }
        crate::panels::performance::draw_perf_sidebar(
            ui,
            panel_origin,
            available,
            perf_stats,
            meshlet_stats,
            meshlet_debug_mode,
            meshlet_debug_caps,
            meshlet_lod_settings,
            lights_hot,
            cluster_settings,
            specular_floor,
            // Physical pixels, like the offscreen target: the grid's
            // columns and rows come out of the viewport's aspect.
            available * pixels_per_point,
            hud_visibility,
            single_light_note,
        );
    }
}

/// The viewport's View menu — Godot's top-left button. Each entry is an
/// overlay the user opts into; the full readout lives in the
/// Performance dock tab and stays out of the picture.
fn view_menu(ui: &mut egui::Ui, origin: egui::Pos2, hud: &mut crate::perf::HudVisibility) {
    let rect = egui::Rect::from_min_size(origin + egui::vec2(8.0, 8.0), egui::vec2(90.0, 26.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(20, 20, 24, 200))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(&mut child, |ui| {
            ui.menu_button(
                egui::RichText::new(format!("{} View", crate::icons::EYE)).size(13.0),
                |ui| {
                    ui.set_min_width(210.0);
                    ui.checkbox(&mut hud.frame_time_card, "View Frame Time")
                        .on_hover_text("FPS, CPU and GPU time, anchored top-right.");
                    ui.checkbox(&mut hud.info_card, "View Information")
                        .on_hover_text("Resolution, draw calls and memory, anchored bottom-right.");
                    ui.separator();
                    ui.checkbox(&mut hud.shadow_pages_window, "Shadow pages window")
                        .on_hover_text("The virtual-shadow-map readout, as a movable window.");
                    ui.checkbox(&mut hud.sidebar, "Performance overlay")
                        .on_hover_text(
                            "The full metrics column over the viewport. The same content \
                             lives in the Performance tab, off the picture.",
                        );
                },
            );
        });
}

/// Godot's frame-time card: three numbers, top-right, out of the way.
fn frame_time_card(
    ui: &mut egui::Ui,
    origin: egui::Pos2,
    available: egui::Vec2,
    perf: &crate::perf::EditorPerfStats,
) {
    let green = egui::Color32::from_rgb(140, 220, 130);
    card(ui, origin + egui::vec2(available.x - 12.0, 12.0), |ui| {
        ui.label(
            egui::RichText::new(format!("CPU Time: {:.2} ms", perf.cpu_frame_ms))
                .color(green)
                .monospace()
                .size(12.0),
        );
        let gpu = perf
            .gpu_frame_ms
            .map(|ms| format!("GPU Time: {ms:.2} ms"))
            .unwrap_or_else(|| "GPU Time: n/a".to_owned());
        ui.label(egui::RichText::new(gpu).color(green).monospace().size(12.0));
        ui.label(
            egui::RichText::new(format!("FPS: {:.0}", perf.fps_avg))
                .color(green)
                .monospace()
                .size(12.0),
        );
    });
}

/// Godot's information card: what the frame is made of, bottom-right.
fn info_card(
    ui: &mut egui::Ui,
    origin: egui::Pos2,
    available: egui::Vec2,
    perf: &crate::perf::EditorPerfStats,
    requested: (u32, u32),
) {
    let text = egui::Color32::from_rgb(220, 220, 224);
    card(
        ui,
        origin + egui::vec2(available.x - 12.0, available.y - 12.0 - 84.0),
        |ui| {
            let mp = requested.0 as f64 * requested.1 as f64 / 1.0e6;
            for line in [
                format!("Size: {} × {} ({mp:.1}MP)", requested.0, requested.1),
                format!("Draw Calls: {}", perf.draw_calls),
                format!("VRAM: {} MB", perf.vram_tracked_bytes / (1024 * 1024)),
                format!("RAM: {} MB", perf.ram_rss_mb),
            ] {
                ui.label(egui::RichText::new(line).color(text).monospace().size(12.0));
            }
        },
    );
}

/// A small anchored card: dark, rounded, right-aligned at `top_right`.
fn card(ui: &mut egui::Ui, top_right: egui::Pos2, body: impl FnOnce(&mut egui::Ui)) {
    let rect =
        egui::Rect::from_min_size(top_right - egui::vec2(190.0, 0.0), egui::vec2(190.0, 200.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Max)),
    );
    egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(20, 20, 24, 210))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(8))
        .show(&mut child, body);
}
