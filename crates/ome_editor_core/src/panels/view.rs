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
use ome_render::meshlet::{MeshletDebugMode, MeshletLodSettings, MeshletRenderStats};

use crate::editor_camera::EditorCameraController;
use crate::editor_camera::input::{HandleModeRequest, ViewportInputDelta, collect_viewport_input};
use crate::icons;
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
    meshlet_lod_settings: &mut MeshletLodSettings,
    meshlet_stats: MeshletRenderStats,
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

    // Horizontal toolbar at the top edge of the viewport. Sections
    // separated by vertical bars: (transform-only) gizmo mode + basis
    // + snap steps, then the always-on debug-view selector.
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

            // Gizmo + basis + snap clusters only operate on Transforms;
            // hide them when no selected entity carries one. The debug
            // selector below always renders so the user can flip viz
            // modes without an entity selected.
            if selection_has_transform {
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

                ui.separator();
            }

            // Debug-view dropdown (#451). Iterates the modes that have
            // shipped shader behaviour so the user never lands on a
            // silent no-op.
            egui::ComboBox::from_id_salt("meshlet_debug_view")
                .selected_text(format!("Debug: {}", meshlet_debug_mode.label()))
                .show_ui(ui, |ui| {
                    for &mode in MeshletDebugMode::all_implemented() {
                        ui.selectable_value(meshlet_debug_mode, mode, mode.label());
                    }
                })
                .response
                .on_hover_text(
                    "Meshlet pipeline visualization mode. Off = production shading.",
                );

            // LOD threshold slider — drives meshlet_lod_settings.target_error_pixels.
            // Logarithmic-feel: 0.1 keeps maximum detail, ≥10 forces
            // coarse roots even at close range. Lives next to the
            // debug dropdown so artists can sanity-check chain
            // behaviour without leaving the viewport.
            ui.separator();
            ui.label(egui::RichText::new("LOD ≤").small())
                .on_hover_text("Pixel-error threshold for the continuous-LOD selector.");
            ui.add(
                egui::DragValue::new(&mut meshlet_lod_settings.target_error_pixels)
                    .speed(0.05)
                    .range(0.1_f32..=50.0_f32)
                    .max_decimals(2)
                    .suffix("px"),
            )
            .on_hover_text(
                "Lower values keep more meshlets at any given distance. \
                 Crank this up to force coarser LOD selection and \
                 visually confirm the chain is being descended.",
            );

            // Stats overlay — only when a debug mode is active so the
            // toolbar stays minimal during normal editing. Per-stage
            // cull survivors (frustum / backface / hi-z) ship in #451b
            // alongside the reject-reason tagging buffer. cam_pos is
            // surfaced so the artist can verify the LOD selector is
            // actually following the active camera while moving in
            // the viewport.
            if *meshlet_debug_mode != MeshletDebugMode::Off {
                ui.separator();
                let [cx, cy, cz] = meshlet_stats.cam_pos;
                let total = meshlet_stats.pool_meshlets_total;
                let roots = meshlet_stats.pool_meshlets_roots;
                ui.label(
                    egui::RichText::new(format!(
                        "instances {} · disp {} · pool {}/{} roots · cam ({:.1}, {:.1}, {:.1})",
                        meshlet_stats.instances_uploaded,
                        meshlet_stats.cull_threads,
                        roots,
                        total,
                        cx, cy, cz,
                    ))
                    .monospace()
                    .small(),
                )
                .on_hover_text(
                    "Meshlet pipeline counters (previous frame).\n\
                     pool: roots/total — total meshlets in the global pool, \
                     of which `roots` are terminal (selector stops there).\n\
                     If roots == total the chain has no LOD depth.\n\
                     cam: world-space position the LOD selector saw — \
                     should follow the active editor camera.",
                );
            }
        });

    *input = Some(delta);
}

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
