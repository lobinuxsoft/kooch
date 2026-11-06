//! The one layout of every Inspector row: a label column, a field that fills what is left, and
//! a ↺ column. Fixed sides, so grids of different properties line up with each other.

/// The label column; `border-bottom-width*` and its info icon are the longest.
const LABEL: f32 = 175.0;
/// The ↺ column.
const RESET: f32 = 22.0;
/// Kept clear at the pane's edge, where the scroll bar floats.
const SCROLL_BAR: f32 = 10.0;
/// Room a slider's number takes beside its track.
const SLIDER_VALUE: f32 = 56.0;
/// Room `X` and its gap take before an axis's drag.
const AXIS: f32 = 18.0;

/// A label, field and ↺ grid whose field column is sized from the panel. Left to egui, a middle
/// column is as wide as it was last frame, so a field that overflows it widens it every frame.
pub(super) fn grid(ui: &egui::Ui, id: impl egui::AsIdSalt) -> egui::Grid {
    let gaps = ui.spacing().item_spacing.x * 2.0;
    // From what is visible: `available_width` read past the pane, and the ↺ landed out of reach.
    let room = ui.clip_rect().max.x - SCROLL_BAR - ui.cursor().min.x;
    egui::Grid::new(id)
        .num_columns(3)
        .max_col_width((room - LABEL - RESET - gaps).max(80.0))
}

/// Holds the label column's width whatever the name's length.
pub(super) fn hold_label(ui: &mut egui::Ui) {
    ui.set_min_width(LABEL);
}

/// `widget` across the whole field.
pub(super) fn fill(ui: &mut egui::Ui, widget: impl egui::Widget) -> egui::Response {
    let size = [ui.available_width(), ui.spacing().interact_size.y];
    ui.add_sized(size, widget)
}

/// What `add` draws with combos and sliders across the whole field.
pub(super) fn wide<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope(|ui| {
        let width = ui.available_width();
        let spacing = ui.spacing_mut();
        spacing.combo_width = width;
        spacing.slider_width = (width - SLIDER_VALUE).max(40.0);
        add(ui)
    })
    .inner
}

/// A colour button across the whole field.
pub(super) fn color(ui: &mut egui::Ui, bytes: &mut [u8; 4]) -> bool {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.x = ui.available_width();
        ui.color_edit_button_srgba_unmultiplied(bytes).changed()
    })
    .inner
}

/// A drag per axis, sharing `width`; whether any moved.
pub(super) fn axes(ui: &mut egui::Ui, numbers: &mut [f32], width: f32, range: Range) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let gap = ui.spacing().item_spacing.x;
        let each = (width / numbers.len() as f32 - AXIS - gap).max(30.0);
        for (number, axis) in numbers.iter_mut().zip(["X", "Y", "Z", "W"]) {
            ui.add_sized(
                [AXIS - gap, ui.spacing().interact_size.y],
                egui::Label::new(axis),
            );
            let drag = egui::DragValue::new(number)
                .speed(0.005)
                .range(range.clone());
            changed |= ui
                .add_sized([each, ui.spacing().interact_size.y], drag)
                .changed();
        }
    });
    changed
}

pub(super) type Range = std::ops::RangeInclusive<f32>;

/// The ↺ of a set field; whether it was clicked.
pub(super) fn reset(ui: &mut egui::Ui, hint: &str) -> bool {
    let size = [RESET, ui.spacing().interact_size.y];
    ui.add_sized(size, egui::Button::new("↺").small())
        .on_hover_text(hint)
        .clicked()
}

/// The ↺ column of a row with nothing to reset: unreserved, a grid of unset rows ends wider.
pub(super) fn no_reset(ui: &mut egui::Ui) {
    ui.allocate_space(egui::vec2(RESET, 0.0));
}
