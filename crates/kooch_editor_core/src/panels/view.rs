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

use crate::editor_camera::EditorCameraController;
use crate::editor_camera::input::{HandleModeRequest, ViewportInputDelta, collect_viewport_input};
use crate::icons;
use crate::state::RotationDisplayMode;
use kooch_gizmos_handles::{HandleMode, SnapSettings};

const TOOLBAR_BUTTON_SIZE: f32 = 28.0;
const TOOLBAR_PADDING: f32 = 6.0;
const TOOLBAR_OFFSET: egui::Vec2 = egui::vec2(8.0, 8.0);

/// Draws the viewport image + the mode + Local/World toolbar.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_view_content(
    ui: &mut egui::Ui,
    focused: bool,
    texture_id: egui::TextureId,
    request: &mut Option<(u32, u32)>,
    input: &mut Option<ViewportInputDelta>,
    controller: &EditorCameraController,
    current_mode: HandleMode,
    rotation_mode: &mut RotationDisplayMode,
    snap_settings: &mut SnapSettings,
    selection_has_transform: bool,
    gizmo_visibility: &mut crate::gizmos::GizmoVisibility,
    gizmo_groups: &[crate::gizmos::GizmoGroup],
    physics_debug: &mut kooch_physics::backend::DebugCategories,
    actions: &mut Vec<crate::actions::EditorAction>,
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
    let response =
        ui.add(egui::Image::new((texture_id, available)).sense(egui::Sense::click_and_drag()));
    let mut delta = collect_viewport_input(&response, ui, controller, focused);

    // A prefab dropped here lands under the cursor. What is passed on is the
    // cursor, not a world position: unprojecting needs the camera's
    // orientation, which lives on the camera entity and not in anything this
    // panel is handed. See `viewport_pick`.
    //
    // Guarded behind `dnd_hover_payload` because `dnd_release_payload` takes
    // the payload before checking its type; see the ordering note in
    // `panels/world/entity_row.rs`.
    if response
        .dnd_hover_payload::<crate::drag_drop::DraggedAsset>()
        .is_some_and(|a| a.type_name == crate::drag_drop::PREFAB_TYPE_NAME)
    {
        ui.painter().rect_stroke(
            response.rect,
            0.0,
            egui::Stroke::new(2.0, egui::Color32::from_rgb(60, 200, 100)),
            egui::StrokeKind::Inside,
        );
        if let Some(prefab) = response.dnd_release_payload::<crate::drag_drop::DraggedAsset>() {
            // `cursor_local` is `None` once the pointer leaves the image, so
            // a release recorded outside it has no place to name and falls
            // back to the authored position rather than to a guess.
            let at = match delta.cursor_local {
                Some(cursor) => crate::viewport_pick::DropPoint::Viewport {
                    cursor,
                    viewport_size: delta.viewport_size,
                },
                None => crate::viewport_pick::DropPoint::Authored,
            };
            actions.push(crate::actions::EditorAction::InstantiatePrefab {
                prefab: prefab.guid,
                at,
            });
        }
    }

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
                    crate::numeric::drag(&mut snap_settings.translate)
                        .speed(0.01)
                        .range(0.001..=10.0)
                        .max_decimals(3)
                        .prefix(format!("{} ", icons::ARROWS_OUT_CARDINAL)),
                )
                .on_hover_text("Translate snap step (world units, hold Ctrl while dragging)");

                ui.add(
                    crate::numeric::drag(&mut snap_settings.rotate_deg)
                        .speed(0.1)
                        .range(0.1..=180.0)
                        .suffix("°")
                        .max_decimals(1)
                        .prefix(format!("{} ", icons::ARROWS_CLOCKWISE)),
                )
                .on_hover_text("Rotate snap step (degrees, hold Ctrl while dragging)");

                ui.separator();

                // Gizmo visibility. Marked when something is hidden, so a
                // missing outline is traceable to a choice rather than
                // looking like a broken gizmo — which is the failure this
                // menu exists to prevent.
                let filtered = gizmo_visibility.has_exceptions();
                let label = if filtered {
                    format!("{} Gizmos*", icons::EYE)
                } else {
                    format!("{} Gizmos", icons::EYE)
                };
                ui.menu_button(label, |ui| {
                    ui.set_min_width(200.0);
                    crate::gizmos::draw_gizmo_menu(ui, gizmo_visibility, gizmo_groups);
                })
                .response
                .on_hover_text(if filtered {
                    "Some gizmos are hidden"
                } else {
                    "Choose which gizmos draw"
                });

                // The solver's own account of itself, separate from the
                // Gizmos menu on purpose: those draw components, this
                // draws what the physics world actually holds, and the
                // whole value is in being able to compare the two.
                let active = physics_debug.any();
                let label = if active {
                    format!("{} Physics*", icons::SLIDERS)
                } else {
                    format!("{} Physics", icons::SLIDERS)
                };
                ui.menu_button(label, |ui| {
                    ui.set_min_width(240.0);
                    draw_physics_debug_menu(ui, physics_debug);
                })
                .response
                .on_hover_text(if active {
                    "The physics overlay is on — it costs frame time"
                } else {
                    "Draw what the solver holds: contacts, joints, mass"
                });
            });
    }

    // The perf sidebar used to live here. It moved to the Game panel:
    // the numbers describe what it costs to draw the game, and reading
    // them beside the game is the point (#592).
    *input = Some(delta);
}

/// Width of the perf sidebar overlay anchored to the right edge of
/// the viewport. 260 px fits the widest "n/a (TIMESTAMP_QUERY
/// unavailable)" GPU-frame-time row without wrapping while leaving
/// room to read the actual viewport.

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

/// The physics overlay's per-category switches.
///
/// Each line says what the category answers rather than what it draws. An
/// author reaching for this menu has a question ("why is it not
/// colliding"), not a shopping list.
fn draw_physics_debug_menu(
    ui: &mut egui::Ui,
    categories: &mut kooch_physics::backend::DebugCategories,
) {
    ui.checkbox(&mut categories.contacts, "Contacts")
        .on_hover_text("Where bodies are actually touching. Start here.");
    ui.checkbox(&mut categories.body_axes, "Centre of mass and axes")
        .on_hover_text(
            "Each body's axes, drawn at its centre of mass — not at its origin. \
             A compound body's centre of mass is rarely where you assume.",
        );
    ui.checkbox(&mut categories.joints, "Joint anchors")
        .on_hover_text(
            "A joint anchored to the wrong point looks exactly like one that is broken.",
        );
    ui.checkbox(&mut categories.collider_aabbs, "Broad-phase bounds")
        .on_hover_text("For when nothing collides at all and the question is whether the broad phase can see it.");

    ui.separator();
    ui.checkbox(&mut categories.collider_shapes, "Solver collider shapes")
        .on_hover_text(
            "The shapes the SOLVER holds, not the ones the components describe. The green \
             collider gizmo already draws the latter — switch this on to compare them, because \
             where they disagree is the bug. The expensive category.",
        );

    ui.separator();
    if ui.button("Turn everything off").clicked() {
        *categories = Default::default();
    }
}
