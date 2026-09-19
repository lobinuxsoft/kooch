//! The readout's building blocks: sections, grids, metrics and warnings.

use super::*;

/// Digit groups, because a pair-test count runs to seven figures and an
/// unbroken run of digits is a number nobody reads.
pub(super) fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// One section of the readout.
pub(super) fn section(
    ui: &mut egui::Ui,
    pinned: &mut bool,
    surface: PerfSurface,
    title: &str,
    default_open: bool,
    body: impl FnOnce(&mut egui::Ui),
) -> bool {
    if surface == PerfSurface::Overlay {
        if !*pinned {
            return false;
        }
        stack_card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(title).strong().small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button(crate::icons::X)
                        .on_hover_text("Hide this overlay")
                        .clicked()
                    {
                        *pinned = false;
                    }
                });
            });
            body(ui);
        });
        return true;
    }
    let id = ui.make_persistent_id(format!("perf_section_{title}"));
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        default_open,
    );
    let (_, _, body_response) = state
        .show_header(ui, |ui| {
            ui.label(egui::RichText::new(title).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut on = *pinned;
                if ui
                    .toggle_value(&mut on, crate::icons::MAP_PIN_SIMPLE_AREA)
                    .on_hover_text("Show as an overlay card on the game viewport")
                    .clicked()
                {
                    *pinned = on;
                }
            });
        })
        .body(body);
    body_response.is_some()
}

/// A semi-transparent card in the game viewport's overlay stack.
pub(crate) fn stack_card(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(20, 20, 24, 170))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(264.0);
            body(ui);
        });
    ui.add_space(6.0);
}

/// Two-column grid for label / value rows.
pub(super) fn grid(ui: &mut egui::Ui, salt: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(salt)
        .num_columns(2)
        .spacing([12.0, 4.0])
        .striped(true)
        .show(ui, body);
}

/// One label / value row inside a section grid.
pub(super) fn metric(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(label);
    ui.label(egui::RichText::new(value).monospace());
    ui.end_row();
}

/// A named block heading inside a section.
pub(super) fn block(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(title).small().strong());
}

/// A reading that says the frame is WRONG, in the colour reserved for it.
pub(super) fn alert(ui: &mut egui::Ui, text: &str, tooltip: &str) {
    ui.label(
        egui::RichText::new(text)
            .small()
            .color(egui::Color32::from_rgb(220, 120, 90)),
    )
    .on_hover_text(tooltip);
}

/// A reading that says the frame is under PRESSURE but still correct — the pool rationing, not the
/// pool failing. Amber rather than red, and the distinction is the point: one is a budget being
/// spent, the other is a bug.
pub(super) fn warn(ui: &mut egui::Ui, text: &str, tooltip: &str) {
    ui.label(
        egui::RichText::new(text)
            .small()
            .color(egui::Color32::from_rgb(230, 190, 90)),
    )
    .on_hover_text(tooltip);
}

/// [`metric_with_tooltip`] with the value in a colour, for a row that is
/// inside its grid but past a threshold.
pub(super) fn metric_coloured(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    colour: egui::Color32,
    tooltip: &str,
) {
    ui.label(label).on_hover_text(tooltip);
    ui.label(egui::RichText::new(value).monospace().color(colour))
        .on_hover_text(tooltip);
    ui.end_row();
}

/// Same as [`metric`] but attaches `tooltip` on hover for both the label and the value, used when
/// the metric's number alone is misleading without context (e.g. fixed editor passes that produce a
/// non-zero floor in an empty scene).
pub(super) fn metric_with_tooltip(ui: &mut egui::Ui, label: &str, value: &str, tooltip: &str) {
    ui.label(label).on_hover_text(tooltip);
    ui.label(egui::RichText::new(value).monospace())
        .on_hover_text(tooltip);
    ui.end_row();
}

/// A froxel's depth as the panel should state it.
pub(super) fn depth_label(metres: f32) -> String {
    match metres.is_finite() {
        true => format!("{metres:.1} m"),
        false => "unbounded".to_owned(),
    }
}
