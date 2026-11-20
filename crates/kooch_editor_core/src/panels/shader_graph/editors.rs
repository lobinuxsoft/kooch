//! The fields a node draws for what it holds, laid out as a column (#1170).

use bevy_lookup_curve::prelude::LookupCurveEguiEditor;
use egui_snarl::InPinId;

use crate::shader_graph::{GraphCurve, GraphNode, Width};

/// How wide a node's fields are. Narrow on purpose: a node reads as a column, not a long row.
pub(super) const FIELD: f32 = 110.0;

/// How wide one number on a pin is: a row of four has to fit beside the pin's name.
const PIN_NUMBER: f32 = 42.0;

/// The value of an unwired pin, beside its name. A pin that takes anything edits one number, the
/// same in every channel, until asked for all four or until they differ.
pub(super) fn pin_field(ui: &mut egui::Ui, cell: &mut GraphNode, pin: InPinId, width: Width) {
    let Some(before) = cell.pin_value(pin.input) else {
        return;
    };
    let mut value = before;
    let wide_id = egui::Id::new(("pin_wide", pin));
    let name = cell.inputs().get(pin.input).copied().unwrap_or_default();
    ui.push_id(pin.input, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let number = |ui: &mut egui::Ui, value: &mut f32| {
            ui.add_sized(
                [PIN_NUMBER, ui.spacing().interact_size.y],
                crate::numeric::drag(value).speed(0.01),
            );
        };
        match width {
            Width::Three | Width::Four if name.contains("colo") => {
                ui.color_edit_button_rgba_unmultiplied(&mut value);
            }
            Width::One => {
                number(ui, &mut value[0]);
                value = [value[0]; 4];
            }
            Width::Two => value[..2].iter_mut().for_each(|v| number(ui, v)),
            Width::Three => value[..3].iter_mut().for_each(|v| number(ui, v)),
            Width::Four => value.iter_mut().for_each(|v| number(ui, v)),
            Width::Any => {
                let mut wide = ui.data(|d| d.get_temp(wide_id).unwrap_or(false));
                wide |= value.iter().any(|v| *v != value[0]);
                if wide {
                    value.iter_mut().for_each(|v| number(ui, v));
                } else {
                    number(ui, &mut value[0]);
                    value = [value[0]; 4];
                }
                let toggle = ui.small_button(if wide { "1" } else { "4" });
                if toggle.on_hover_text("One number or four").clicked() {
                    ui.data_mut(|d| d.insert_temp(wide_id, !wide));
                    if wide {
                        value = [value[0]; 4];
                    }
                }
            }
        }
    });
    if value != before {
        cell.set_pin(pin.input, value);
    }
}

/// A parameter's name, on a line of its own.
pub(super) fn name_field(ui: &mut egui::Ui, name: &mut String) {
    ui.add(egui::TextEdit::singleline(name).desired_width(FIELD));
}

/// One number per line, labelled by the component it is.
pub(super) fn components(ui: &mut egui::Ui, values: &mut [f32]) {
    for (value, axis) in values.iter_mut().zip(["x", "y", "z", "w"]) {
        ui.add(
            crate::numeric::drag(value)
                .speed(0.01)
                .prefix(format!("{axis} ")),
        );
    }
}

/// One choice from a short list, as a dropdown: a row of buttons made every such node wide.
pub(super) fn choice(ui: &mut egui::Ui, id: &str, current: &mut String, options: &[&str]) {
    egui::ComboBox::from_id_salt(id)
        .width(FIELD)
        .selected_text(current.as_str())
        .show_ui(ui, |ui| {
            for option in options {
                if ui.selectable_label(current == option, *option).clicked() {
                    *current = (*option).to_owned();
                }
            }
        });
}

/// A float or a whole number, top to bottom: its name, whether it has a range, the range, and its
/// starting value — a slider inside the range, as the material's Inspector will draw it.
pub(super) fn number_editor(
    ui: &mut egui::Ui,
    name: &mut String,
    default: &mut f32,
    range: &mut Option<[f32; 2]>,
    whole: bool,
) {
    let step = if whole { 1.0 } else { 0.01 };
    let decimals = if whole { 0 } else { 2 };
    name_field(ui, name);
    let mut ranged = range.is_some();
    if ui.checkbox(&mut ranged, "range").changed() {
        *range = ranged.then_some([0.0, if whole { 10.0 } else { 1.0 }]);
    }
    if let Some([lo, hi]) = range.as_mut() {
        for (bound, label) in [(&mut *lo, "min "), (&mut *hi, "max ")] {
            ui.add(
                egui::DragValue::new(bound)
                    .speed(step)
                    .fixed_decimals(decimals)
                    .prefix(label),
            );
        }
        // 🔴 A range the wrong way round is a slider egui cannot draw; keep it ordered as it is typed.
        if *hi < *lo {
            *hi = *lo;
        }
    }
    match *range {
        Some([lo, hi]) => {
            ui.spacing_mut().slider_width = FIELD - 50.0;
            ui.add(
                crate::numeric::slider(default, lo..=hi)
                    .step_by(if whole { 1.0 } else { 0.0 })
                    .fixed_decimals(decimals),
            );
        }
        None => {
            ui.add(
                egui::DragValue::new(default)
                    .speed(step)
                    .fixed_decimals(decimals),
            );
        }
    }
    if whole {
        *default = default.round();
    }
}

/// A curve node's editor, `width` wide. Its view (pan, zoom) is widget state, kept in egui's memory
/// by node so it never reaches the saved graph.
pub(super) fn curve_editor(
    ui: &mut egui::Ui,
    node: egui_snarl::NodeId,
    curve: &mut GraphCurve,
    width: f32,
) {
    type View = std::sync::Arc<std::sync::Mutex<LookupCurveEguiEditor>>;
    let id = egui::Id::new(("curve view", node));
    let view: View = ui.ctx().data_mut(|data| {
        data.get_temp_mut_or_insert_with(id, || {
            View::new(std::sync::Mutex::new(
                LookupCurveEguiEditor::fitted_to_curve(&curve.0),
            ))
        })
        .clone()
    });
    let mut view = view
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let size = egui::vec2(width, width * 0.75);
    // A fixed rect the editor cannot grow: its header prints the hovered point unrounded, and a
    // panel that widened with it moved the canvas under the mouse, which changed the point again.
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let hovered = response.contains_pointer();
    ui.ctx()
        .data_mut(|d| d.insert_temp(curve_hover_id(), hovered));
    let builder = egui::UiBuilder::new()
        .max_rect(rect)
        .layout(egui::Layout::top_down(egui::Align::Min));
    let mut inner = ui.new_child(builder);
    inner.set_clip_rect(rect.intersect(ui.clip_rect()));
    view.grid_step_x = grid_step(view.scale.x);
    view.grid_step_y = grid_step(view.scale.y);
    view.ui(&mut inner, &mut curve.0, None);
    ui.weak(
        "Drag a knot to move it, drag the background to pan, scroll to zoom. Right-click the \
         background to add a knot, or a knot for its interpolation, tangents or to delete it. \
         Outside the knots the curve holds its end values.",
    );
}

/// Where the curve editor says the pointer is over it, so the column around it does not scroll on
/// the wheel that zooms the curve.
pub(super) fn curve_hover_id() -> egui::Id {
    egui::Id::new("shader_graph_curve_hovered")
}

/// The curve drawn small and still, for the node on the canvas.
pub(super) fn curve_thumbnail(ui: &mut egui::Ui, curve: &GraphCurve) {
    const SAMPLES: usize = 48;
    let size = egui::vec2(FIELD + 16.0, FIELD * 0.6);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
    if let Some(((x0, _), (x1, _))) = curve.ends() {
        let span = (x1 - x0).max(1e-5);
        let ys: Vec<f32> = (0..=SAMPLES)
            .map(|i| curve.0.lookup(x0 + span * i as f32 / SAMPLES as f32))
            .collect();
        let (lo, hi) = ys
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), y| {
                (lo.min(*y), hi.max(*y))
            });
        let inset = rect.shrink(4.0);
        let height = (hi - lo).max(1e-5);
        let points = ys
            .iter()
            .enumerate()
            .map(|(i, y)| {
                let t = i as f32 / SAMPLES as f32;
                egui::pos2(
                    inset.left() + t * inset.width(),
                    inset.bottom() - (y - lo) / height * inset.height(),
                )
            })
            .collect();
        let stroke = egui::Stroke::new(1.5, ui.visuals().selection.stroke.color);
        painter.add(egui::Shape::line(points, stroke));
    }
    response
        .on_hover_text("Select the node to edit its curve in the inspector, right of the graph");
}

/// A round grid step giving about four lines across `span`: the editor's fixed 0.1 drew a hundred
/// labels over a curve ten units tall.
fn grid_step(span: f32) -> f32 {
    let rough = span.abs() / 4.0;
    if !(rough > 0.0) || !rough.is_finite() {
        return 0.1;
    }
    let decade = 10f32.powf(rough.log10().floor());
    let nice = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .find(|m| m * decade >= rough)
        .unwrap_or(10.0);
    nice * decade
}
