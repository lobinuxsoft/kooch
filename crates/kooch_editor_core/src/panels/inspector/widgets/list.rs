//! A reflected list: each item drawn with its own widget, plus add, remove and reorder (#1201).

use kooch_ecs::reflect::ReflectValue;

use super::value_widget::{FieldContext, draw_value_widget};
use crate::icons;

/// What happened to the list this frame.
enum Change {
    Edit(usize, ReflectValue),
    Remove(usize),
    Up(usize),
    Down(usize),
    Add,
}

/// Draws `items` one row each. Returns the whole new list when anything changed, so an edit is one
/// `SetField` and undoes as one step, however many items it moved.
pub(super) fn draw_list(
    ui: &mut egui::Ui,
    items: &[ReflectValue],
    element: &ReflectValue,
    field: &FieldContext<'_>,
) -> Option<ReflectValue> {
    let mut change = None;
    ui.vertical(|ui| {
        for (index, item) in items.iter().enumerate() {
            ui.push_id(index, |ui| {
                ui.horizontal(|ui| {
                    ui.weak(format!("{index}"));
                    if let Some(edited) = draw_value_widget(ui, item, field) {
                        change = Some(Change::Edit(index, edited));
                    }
                    let last = index + 1 == items.len();
                    if ui
                        .add_enabled(index > 0, egui::Button::new(icons::ARROW_UP).small())
                        .on_hover_text("Move up — runs earlier")
                        .clicked()
                    {
                        change = Some(Change::Up(index));
                    }
                    if ui
                        .add_enabled(!last, egui::Button::new(icons::ARROW_DOWN).small())
                        .on_hover_text("Move down — runs later")
                        .clicked()
                    {
                        change = Some(Change::Down(index));
                    }
                    if ui
                        .small_button(icons::TRASH)
                        .on_hover_text("Remove")
                        .clicked()
                    {
                        change = Some(Change::Remove(index));
                    }
                });
            });
        }
        if ui.small_button(format!("{} Add", icons::PLUS)).clicked() {
            change = Some(Change::Add);
        }
    });

    Some(applied(items, element, change?))
}

/// The list after `change`. Separate from the drawing so what a click does is testable.
fn applied(items: &[ReflectValue], element: &ReflectValue, change: Change) -> ReflectValue {
    let mut items = items.to_vec();
    match change {
        Change::Edit(index, value) => items[index] = value,
        Change::Remove(index) => {
            items.remove(index);
        }
        Change::Up(index) => items.swap(index, index - 1),
        Change::Down(index) => items.swap(index, index + 1),
        Change::Add => items.push(element.clone()),
    }
    ReflectValue::List {
        items,
        element: Box::new(element.clone()),
    }
}

#[cfg(test)]
mod tests;
