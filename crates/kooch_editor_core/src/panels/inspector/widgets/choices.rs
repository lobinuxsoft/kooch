//! Choice-dropdown helpers for integer fields decorated with
//! `FieldChoice` metadata, plus the read-only label fallback used by
//! both single- and multi-entity rendering.

use kooch_ecs::reflect::{FieldChoice, FieldMeta, ReflectValue};

/// Looks up the `choices` slice for a field by name. Returns an empty
/// slice if the metadata is missing or the field has no `choices` hint.
pub(crate) fn choices_for(
    field_metas: Option<&'static [FieldMeta]>,
    name: &str,
) -> &'static [FieldChoice] {
    field_metas
        .and_then(|metas| metas.iter().find(|m| m.name == name))
        .map(|m| m.choices)
        .unwrap_or(&[])
}

/// Renders a read-only display for a single value. If the field has a
/// `choices` hint, prefer the matching label over the raw numeric value.
pub(crate) fn draw_readonly_value(
    ui: &mut egui::Ui,
    value: &ReflectValue,
    choices: &'static [FieldChoice],
    bits: &'static [FieldChoice],
) {
    if let Some(label) = choice_label_for(value, choices) {
        ui.weak(label);
    } else if !bits.is_empty() {
        // A mask shown as its number is the unreadability the grid exists
        // to fix; read-only is not a reason to go back to `4294967295`.
        ui.weak(set_bit_names(value, bits));
    } else {
        ui.weak(format!("{value}"));
    }
}

/// The named bits that are set, or a word for the two common extremes.
fn set_bit_names(value: &ReflectValue, bits: &'static [FieldChoice]) -> String {
    let Some(current) = reflect_value_as_i64(value) else {
        return format!("{value}");
    };
    let set: Vec<&str> = bits
        .iter()
        .filter(|bit| current & bit.value != 0)
        .map(|bit| bit.label)
        .collect();
    match set.len() {
        0 => "None".to_owned(),
        n if n == bits.len() => "All".to_owned(),
        _ => set.join(", "),
    }
}

/// Returns the `choices` label for an integer-valued field, if any.
fn choice_label_for(value: &ReflectValue, choices: &'static [FieldChoice]) -> Option<&'static str> {
    let current = reflect_value_as_i64(value)?;
    choices.iter().find(|c| c.value == current).map(|c| c.label)
}

/// Converts an integer [`ReflectValue`] into `i64` for dropdown matching.
fn reflect_value_as_i64(value: &ReflectValue) -> Option<i64> {
    match value {
        ReflectValue::U8(v) => Some(*v as i64),
        ReflectValue::U16(v) => Some(*v as i64),
        ReflectValue::U32(v) => Some(*v as i64),
        ReflectValue::U64(v) => Some(*v as i64),
        ReflectValue::I8(v) => Some(*v as i64),
        ReflectValue::I16(v) => Some(*v as i64),
        ReflectValue::I32(v) => Some(*v as i64),
        ReflectValue::I64(v) => Some(*v),
        _ => None,
    }
}

/// Builds a [`ReflectValue`] of the same variant as `template` from an
/// `i64`. Used to materialise the user's dropdown pick back into the
/// component's actual numeric type.
fn reflect_value_from_i64(template: &ReflectValue, v: i64) -> Option<ReflectValue> {
    match template {
        ReflectValue::U8(_) => Some(ReflectValue::U8(v as u8)),
        ReflectValue::U16(_) => Some(ReflectValue::U16(v as u16)),
        ReflectValue::U32(_) => Some(ReflectValue::U32(v as u32)),
        ReflectValue::U64(_) => Some(ReflectValue::U64(v as u64)),
        ReflectValue::I8(_) => Some(ReflectValue::I8(v as i8)),
        ReflectValue::I16(_) => Some(ReflectValue::I16(v as i16)),
        ReflectValue::I32(_) => Some(ReflectValue::I32(v as i32)),
        ReflectValue::I64(_) => Some(ReflectValue::I64(v)),
        _ => None,
    }
}

/// Renders a dropdown for an integer field with `choices` metadata.
/// Returns `Some(new_value)` when the user picks a different entry.
pub(super) fn draw_choice_dropdown(
    ui: &mut egui::Ui,
    value: &ReflectValue,
    choices: &'static [FieldChoice],
    field_name: &str,
) -> Option<ReflectValue> {
    let current = reflect_value_as_i64(value)?;
    let selected_label = choices
        .iter()
        .find(|c| c.value == current)
        .map(|c| c.label)
        .unwrap_or("(unknown)");
    let mut picked: Option<i64> = None;
    egui::ComboBox::from_id_salt(("choice_dropdown", field_name))
        .selected_text(selected_label)
        .show_ui(ui, |ui| {
            for choice in choices {
                if ui
                    .selectable_label(choice.value == current, choice.label)
                    .clicked()
                {
                    picked = Some(choice.value);
                }
            }
        });
    let new_val = picked?;
    if new_val == current {
        return None;
    }
    reflect_value_from_i64(value, new_val)
}

/// Looks up the `bits` slice for a field by name.
/// Whether the field is a mask over the project's layer names (#1218).
pub(crate) fn layers_for(field_metas: Option<&'static [FieldMeta]>, name: &str) -> bool {
    field_metas
        .and_then(|metas| metas.iter().find(|m| m.name == name))
        .is_some_and(|m| m.layers)
}

pub(crate) fn bits_for(
    field_metas: Option<&'static [FieldMeta]>,
    name: &str,
) -> &'static [FieldChoice] {
    field_metas
        .and_then(|metas| metas.iter().find(|m| m.name == name))
        .map(|m| m.bits)
        .unwrap_or(&[])
}

/// Renders an integer field as a compact grid of toggles.
pub(crate) fn draw_bitmask(
    ui: &mut egui::Ui,
    value: &ReflectValue,
    bits: &[FieldChoice],
    field_name: &str,
) -> Option<ReflectValue> {
    let cells: Vec<(&str, i64)> = bits.iter().map(|bit| (bit.label, bit.value)).collect();
    draw_cells(ui, value, &cells, field_name)
}

/// A layer mask (#1218): a summary of what is ticked, and a list of the project's names behind it.
///
/// 🔴 Not the grid above: thirty-two cells do not fit the Inspector at any width anyone uses, and
/// what a mask says has to be readable without opening anything.
pub(crate) fn draw_layer_mask(
    ui: &mut egui::Ui,
    value: &ReflectValue,
    labels: &[String],
    field_name: &str,
) -> Option<ReflectValue> {
    /// Past this the list scrolls rather than growing off the screen.
    const LIST_HEIGHT: f32 = 320.0;

    let current = reflect_value_as_i64(value)?;
    let mut next = current;
    let every = layer_mask(labels);
    egui::ComboBox::from_id_salt(("layers", field_name))
        .selected_text(summary(current, labels))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.small_button("All").clicked() {
                    next |= every;
                }
                if ui.small_button("None").clicked() {
                    next &= !every;
                }
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(LIST_HEIGHT)
                .show(ui, |ui| {
                    for (bit, label) in labels.iter().enumerate() {
                        let mut on = current & (1i64 << bit) != 0;
                        if ui.checkbox(&mut on, label).changed() {
                            next ^= 1i64 << bit;
                        }
                    }
                });
        });

    (next != current).then(|| reflect_value_from_i64(value, next))?
}

/// What the mask says at a glance: the layer's own name while it is the only one, and how many
/// otherwise. A number would be a number.
fn summary(mask: i64, labels: &[String]) -> String {
    let every = layer_mask(labels);
    let ticked: Vec<&String> = labels
        .iter()
        .enumerate()
        .filter(|(bit, _)| mask & (1i64 << bit) != 0)
        .map(|(_, label)| label)
        .collect();
    match ticked.len() {
        0 => "Nothing".to_owned(),
        1 => ticked[0].clone(),
        _ if mask & every == every => "Everything".to_owned(),
        count => format!("Mixed ({count})"),
    }
}

/// Every bit the table names — what "All" and "None" are allowed to touch.
fn layer_mask(labels: &[String]) -> i64 {
    (0..labels.len()).fold(0, |mask, bit| mask | (1i64 << bit))
}

fn draw_cells(
    ui: &mut egui::Ui,
    value: &ReflectValue,
    bits: &[(&str, i64)],
    field_name: &str,
) -> Option<ReflectValue> {
    /// Wide enough for two digits, uniform so the grid lines up.
    const CELL: egui::Vec2 = egui::vec2(24.0, 18.0);
    /// Eight per row: two rows covers the sixteen groups, and eight cells
    /// stay narrow enough for the Inspector at its usual width.
    const PER_ROW: usize = 8;

    let current = reflect_value_as_i64(value)?;
    let mut next = current;

    ui.push_id(field_name, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
        for row in bits.chunks(PER_ROW) {
            ui.horizontal(|ui| {
                for &(label, value) in row {
                    let set = current & value != 0;
                    let response = ui
                        .add(
                            egui::Button::new(short_label(label))
                                .min_size(CELL)
                                .selected(set),
                        )
                        .on_hover_text(label);
                    if response.clicked() {
                        // Toggle: the same click sets and clears, which is
                        // what a toggle in a grid has to do.
                        next ^= value;
                    }
                }
            });
        }
        // Worth the row: the default is every bit set, and clicking sixteen
        // cells to express "nothing" is how people go back to typing
        // numbers.
        ui.horizontal(|ui| {
            if ui.small_button("All").clicked() {
                next |= named_mask(bits);
            }
            if ui.small_button("None").clicked() {
                next &= !named_mask(bits);
            }
        });
    });

    (next != current).then(|| reflect_value_from_i64(value, next))?
}

/// What goes on a cell: the trailing number if the label ends in one, so "Group 12" reads as "12"
/// and the grid stays a grid.
fn short_label(label: &str) -> String {
    match label.rsplit(' ').next() {
        Some(tail) if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) => {
            tail.to_owned()
        }
        _ => label.chars().take(3).collect(),
    }
}

/// The union of every named bit — everything this widget is allowed to
/// touch.
fn named_mask(bits: &[(&str, i64)]) -> i64 {
    bits.iter().fold(0, |mask, &(_, value)| mask | value)
}

#[cfg(test)]
mod bitmask_tests;

/// Looks up the `requires` hint for an entity-reference field: the short
/// name of a component its target has to carry, or `""` when anything
/// will do.
pub(crate) fn requires_for(field_metas: Option<&'static [FieldMeta]>, name: &str) -> &'static str {
    field_metas
        .and_then(|metas| metas.iter().find(|m| m.name == name))
        .map(|m| m.requires)
        .unwrap_or("")
}

/// For a list of structs, the element's fields (#1209); `&[]` otherwise.
pub(crate) fn fields_for(
    field_metas: Option<&'static [FieldMeta]>,
    name: &str,
) -> &'static [FieldMeta] {
    field_metas
        .and_then(|metas| metas.iter().find(|m| m.name == name))
        .map(|m| m.fields)
        .unwrap_or(&[])
}

/// The numeric bounds declared for one field, if any.
pub(crate) fn range_for(
    field_metas: Option<&'static [FieldMeta]>,
    name: &str,
) -> Option<&'static kooch_ecs::reflect::FieldRange> {
    field_metas
        .and_then(|metas| metas.iter().find(|m| m.name == name))
        .and_then(|m| m.range)
}
