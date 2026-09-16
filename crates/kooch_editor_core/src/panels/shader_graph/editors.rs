//! The fields a node draws for what it holds, laid out as a column (#1170).

/// How wide a node's fields are. Narrow on purpose: a node reads as a column, not a long row.
pub(super) const FIELD: f32 = 110.0;

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
                egui::Slider::new(default, lo..=hi)
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
