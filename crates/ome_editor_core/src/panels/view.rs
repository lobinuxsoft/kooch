//! View panel — viewport image + handle-mode toolbar overlay.
//!
//! The toolbar at the top-left of the panel hosts:
//!
//! 1. **Move / Rotate / Scale** mode buttons (always visible). Tinted
//!    when the corresponding `HandleMode` is active. Tooltip shows the
//!    keyboard shortcut.
//! 2. **Local / World** rotation-display toggle (only when at least
//!    one selected entity has a `Transform` component). Affects the
//!    inspector rotation display AND the gizmo handles' basis.
//!
//! Toolbar clicks write to `viewport_input.mode_request` so the
//! existing W / E / R keyboard pipeline applies the change with no
//! extra wiring.

use ome_gizmos_handles::{HandleMode, SnapSettings};
use ome_render::meshlet::{MeshletDebugCaps, MeshletDebugMode, MeshletLodSettings, MeshletRenderStats};

use crate::editor_camera::EditorCameraController;
use crate::editor_camera::input::{HandleModeRequest, ViewportInputDelta, collect_viewport_input};
use crate::icons;
use crate::panels::performance::draw_performance_content;
use crate::perf::EditorPerfStats;
use crate::state::RotationDisplayMode;

const TOOLBAR_BUTTON_SIZE: f32 = 28.0;
const TOOLBAR_PADDING: f32 = 6.0;
const TOOLBAR_OFFSET: egui::Vec2 = egui::vec2(8.0, 8.0);

/// Draws the viewport image + the mode + Local/World toolbar.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_view_content(
    ui: &mut egui::Ui,
    texture_id: egui::TextureId,
    request: &mut Option<(u32, u32)>,
    input: &mut Option<ViewportInputDelta>,
    controller: &EditorCameraController,
    current_mode: HandleMode,
    rotation_mode: &mut RotationDisplayMode,
    snap_settings: &mut SnapSettings,
    selection_has_transform: bool,
    meshlet_debug_mode: &mut MeshletDebugMode,
    meshlet_debug_caps: MeshletDebugCaps,
    meshlet_lod_settings: &mut MeshletLodSettings,
    meshlet_stats: MeshletRenderStats,
    perf_stats: EditorPerfStats,
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

    let panel_origin = ui.cursor().min;

    // Allocate the interactive viewport image first (it captures camera
    // input). The toolbar is drawn on top using a child UI placed at
    // the top-left corner of the panel rect so its clicks do not fall
    // through to the viewport drag layer.
    let response = ui.add(
        egui::Image::new((texture_id, available)).sense(egui::Sense::click_and_drag()),
    );
    let mut delta = collect_viewport_input(&response, ui, controller);

    // Horizontal toolbar at the top edge of the viewport. Hosts only
    // gizmo controls (mode + basis + snap), shown when a Transform is
    // actually selected. Without a selection there is nothing to put
    // here — debug + perf knobs all live in the right sidebar — so we
    // skip the entire Frame to avoid leaving an empty padded
    // rectangle floating in the viewport's top-left.
    if selection_has_transform {
        let toolbar_rect = egui::Rect::from_min_size(
            panel_origin + TOOLBAR_OFFSET,
            egui::vec2(560.0, TOOLBAR_BUTTON_SIZE + TOOLBAR_PADDING * 2.0),
        );
        let mut toolbar_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(toolbar_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );

        egui::Frame::new()
            .fill(egui::Color32::from_rgba_unmultiplied(20, 20, 24, 200))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::same(TOOLBAR_PADDING as i8))
            .show(&mut toolbar_ui, |ui| {
                ui.spacing_mut().item_spacing.x = TOOLBAR_PADDING * 0.5;
                if mode_button(
                    ui,
                    icons::ARROWS_OUT_CARDINAL,
                    "Move (W)",
                    current_mode == HandleMode::Translate,
                ) {
                    delta.mode_request = Some(HandleModeRequest::Translate);
                }
                if mode_button(
                    ui,
                    icons::ARROWS_CLOCKWISE,
                    "Rotate (E)",
                    current_mode == HandleMode::Rotate,
                ) {
                    delta.mode_request = Some(HandleModeRequest::Rotate);
                }
                if mode_button(
                    ui,
                    icons::ARROWS_OUT_SIMPLE,
                    "Scale (R)",
                    current_mode == HandleMode::Scale,
                ) {
                    delta.mode_request = Some(HandleModeRequest::Scale);
                }

                ui.separator();

                if mode_button(
                    ui,
                    icons::MAP_PIN_SIMPLE_AREA,
                    "Local — handles follow entity rotation",
                    *rotation_mode == RotationDisplayMode::Local,
                ) {
                    *rotation_mode = RotationDisplayMode::Local;
                }
                if mode_button(
                    ui,
                    icons::GLOBE_SIMPLE,
                    "World — handles aligned to world axes",
                    *rotation_mode == RotationDisplayMode::World,
                ) {
                    *rotation_mode = RotationDisplayMode::World;
                }

                ui.separator();

                // Snap step values. Reuse the move / rotate glyphs as
                // prefixes so users associate each spinner with the
                // matching gizmo mode without spending toolbar width on
                // text labels.
                ui.add(
                    egui::DragValue::new(&mut snap_settings.translate)
                        .speed(0.01)
                        .range(0.001..=10.0)
                        .max_decimals(3)
                        .prefix(format!("{} ", icons::ARROWS_OUT_CARDINAL)),
                )
                .on_hover_text("Translate snap step (world units, hold Ctrl while dragging)");

                ui.add(
                    egui::DragValue::new(&mut snap_settings.rotate_deg)
                        .speed(0.1)
                        .range(0.1..=180.0)
                        .suffix("°")
                        .max_decimals(1)
                        .prefix(format!("{} ", icons::ARROWS_CLOCKWISE)),
                )
                .on_hover_text("Rotate snap step (degrees, hold Ctrl while dragging)");
            });
    }

    // Vertical perf sidebar anchored to the right edge of the
    // viewport. The toggle chevron sits at the very top-right
    // corner (always visible); the panel itself only renders when
    // toggled on. State is stored in egui memory so it survives
    // across frames without an extra Resource.
    let sidebar_visible_id = egui::Id::new("perf_sidebar_visible");
    let mut sidebar_visible = ui
        .ctx()
        .memory(|m| m.data.get_temp::<bool>(sidebar_visible_id))
        .unwrap_or(true);

    let panel_top_right =
        panel_origin + egui::vec2(available.x - TOOLBAR_OFFSET.x, TOOLBAR_OFFSET.y);

    // Toggle chevron — left-pointing when expanded (click to
    // collapse to the right), right-pointing when collapsed (click
    // to expand back). Always rendered so the user has a way back
    // even after hiding the panel.
    let toggle_size = egui::vec2(TOOLBAR_BUTTON_SIZE, TOOLBAR_BUTTON_SIZE);
    let toggle_pos = panel_top_right - egui::vec2(toggle_size.x, 0.0);
    let toggle_rect = egui::Rect::from_min_size(toggle_pos, toggle_size);
    let mut toggle_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(toggle_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(20, 20, 24, 200))
        .corner_radius(egui::CornerRadius::same(6))
        .show(&mut toggle_ui, |ui| {
            let glyph = if sidebar_visible { "\u{27e9}" } else { "\u{27e8}" };
            let button = egui::Button::new(egui::RichText::new(glyph).size(16.0))
                .min_size(toggle_size)
                .fill(egui::Color32::TRANSPARENT)
                .stroke(egui::Stroke::NONE);
            let resp = ui.add(button).on_hover_text(if sidebar_visible {
                "Hide performance sidebar"
            } else {
                "Show performance sidebar"
            });
            if resp.clicked() {
                sidebar_visible = !sidebar_visible;
                ui.ctx()
                    .memory_mut(|m| m.data.insert_temp(sidebar_visible_id, sidebar_visible));
            }
        });

    if sidebar_visible {
        // Panel sits below the toggle chevron, anchored to the
        // right edge. max_rect height is bounded by the viewport
        // so the inner ScrollArea can clip when sections overflow;
        // auto_shrink in `draw_performance_content` keeps the
        // Frame tight around the actually-visible content so
        // collapsing every section doesn't leave a giant black
        // box on the viewport.
        let panel_top = toggle_pos.y + toggle_size.y + 4.0;
        let panel_max_height = (available.y - 2.0 * TOOLBAR_OFFSET.y - toggle_size.y - 4.0)
            .max(0.0);
        let sidebar_max_rect = egui::Rect::from_min_size(
            egui::pos2(
                panel_top_right.x - PERF_SIDEBAR_WIDTH,
                panel_top,
            ),
            egui::vec2(PERF_SIDEBAR_WIDTH, panel_max_height),
        );
        let mut sidebar_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(sidebar_max_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        egui::Frame::new()
            .fill(egui::Color32::from_rgba_unmultiplied(20, 20, 24, 200))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::same(TOOLBAR_PADDING as i8))
            .show(&mut sidebar_ui, |ui| {
                ui.set_max_width(PERF_SIDEBAR_WIDTH - TOOLBAR_PADDING * 2.0);
                draw_performance_content(
                    ui,
                    perf_stats,
                    meshlet_stats,
                    meshlet_debug_mode,
                    meshlet_debug_caps,
                    meshlet_lod_settings,
                );
            });
    }

    *input = Some(delta);
}

/// Width of the perf sidebar overlay anchored to the right edge of
/// the viewport. 260 px fits the widest "n/a (TIMESTAMP_QUERY
/// unavailable)" GPU-frame-time row without wrapping while leaving
/// room to read the actual viewport.
const PERF_SIDEBAR_WIDTH: f32 = 260.0;

/// Renders one toolbar button. Highlights when `active`. Returns
/// `true` the frame the button is clicked.
fn mode_button(ui: &mut egui::Ui, icon: &str, tooltip: &str, active: bool) -> bool {
    let visuals = ui.style().visuals.clone();
    let fill = if active {
        visuals.selection.bg_fill
    } else {
        visuals.widgets.inactive.bg_fill
    };
    let stroke = if active {
        visuals.selection.stroke
    } else {
        visuals.widgets.inactive.bg_stroke
    };
    let button = egui::Button::new(egui::RichText::new(icon).size(18.0))
        .min_size(egui::vec2(TOOLBAR_BUTTON_SIZE, TOOLBAR_BUTTON_SIZE))
        .fill(fill)
        .stroke(stroke);
    ui.add(button).on_hover_text(tooltip).clicked()
}
