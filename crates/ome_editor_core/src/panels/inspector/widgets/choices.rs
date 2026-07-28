//! Choice-dropdown helpers for integer fields decorated with
//! `FieldChoice` metadata, plus the read-only label fallback used by
//! both single- and multi-entity rendering.

use ome_ecs::reflect::{FieldChoice, FieldMeta, ReflectValue};

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
///
/// Empty means "not a bitmask", the same way an empty `choices` means "not
/// a dropdown".
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
///
/// A checkbox per bit stacked vertically is what this was first, and with
/// sixteen bits across four mask fields it filled the Inspector with
/// sixty-four rows of the word "Group". The grid is the same information in
/// three rows: the number on the face, the full name on hover, and set or
/// unset legible at a glance across the whole mask — which is the thing a
/// filter is read for.
///
/// Unity and Unreal both use a grid for layer masks. Worth matching rather
/// than inventing, because it is the one part of collision filtering people
/// already know how to look at.
///
/// Bits outside the named set are preserved rather than cleared: a mask
/// authored by a newer editor, or by hand, must survive a visit to this
/// widget untouched.
///
/// # `field_name` is not decoration
///
/// egui derives a widget's id from its label. `Collider` has four mask
/// fields sharing one set of bit names, so without a scope per field the
/// same sixteen ids appear four times in one panel — and egui reported it,
/// sixty-five times a run: `Widget rect ... changed id between passes`.
/// Colliding ids do not merely warn; they send clicks to whichever widget
/// claimed the id, which reads as the whole Inspector ignoring the mouse.
pub(crate) fn draw_bitmask(
    ui: &mut egui::Ui,
    value: &ReflectValue,
    bits: &'static [FieldChoice],
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
                for bit in row {
                    let set = current & bit.value != 0;
                    let response = ui
                        .add(
                            egui::Button::new(short_label(bit.label))
                                .min_size(CELL)
                                .selected(set),
                        )
                        .on_hover_text(bit.label);
                    if response.clicked() {
                        // Toggle: the same click sets and clears, which is
                        // what a toggle in a grid has to do.
                        next ^= bit.value;
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

/// What goes on a cell: the trailing number if the label ends in one, so
/// "Group 12" reads as "12" and the grid stays a grid.
///
/// Anything else is truncated rather than dropped — a project renaming its
/// layers should get something on the face, and the full name is on hover
/// either way.
fn short_label(label: &'static str) -> String {
    match label.rsplit(' ').next() {
        Some(tail) if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) => {
            tail.to_owned()
        }
        _ => label.chars().take(3).collect(),
    }
}

/// The union of every named bit — everything this widget is allowed to
/// touch.
fn named_mask(bits: &'static [FieldChoice]) -> i64 {
    bits.iter().fold(0, |mask, bit| mask | bit.value)
}

#[cfg(test)]
mod bitmask_tests {
    use super::*;

    static BITS: &[FieldChoice] = &[
        FieldChoice {
            label: "A",
            value: 1 << 0,
        },
        FieldChoice {
            label: "B",
            value: 1 << 1,
        },
    ];

    /// The widget may only touch the bits it names. A mask authored by hand
    /// or by a newer editor has to survive a visit — silently clearing the
    /// high half would be a filtering bug introduced by *looking* at the
    /// field.
    #[test]
    fn only_the_named_bits_are_in_scope() {
        assert_eq!(named_mask(BITS), 0b11);
    }

    /// "None" clears the named bits and leaves the rest alone, which is the
    /// same rule stated from the other side.
    #[test]
    fn clearing_preserves_unnamed_bits() {
        let current: i64 = 0b1000_0011;
        let cleared = current & !named_mask(BITS);
        assert_eq!(cleared, 0b1000_0000, "an unnamed bit was cleared");
    }

    /// And setting everything named must not disturb them either.
    #[test]
    fn setting_all_preserves_unnamed_bits() {
        let current: i64 = 0b1000_0000;
        let all = named_mask(BITS) | (current & !named_mask(BITS));
        assert_eq!(all, 0b1000_0011);
    }

    /// The value has to come back as the field's own type, or writing a
    /// `u32` mask into a `u32` field would silently widen it.
    #[test]
    fn the_result_keeps_the_fields_numeric_type() {
        let rebuilt = reflect_value_from_i64(&ReflectValue::U32(0), 0b11);
        assert!(matches!(rebuilt, Some(ReflectValue::U32(3))));
    }

    /// A non-integer field is not a bitmask, and asking must not panic.
    #[test]
    fn a_non_integer_value_is_not_a_bitmask() {
        assert_eq!(reflect_value_as_i64(&ReflectValue::F32(1.0)), None);
    }
}

/// Looks up the `requires` hint for an entity-reference field: the short
/// name of a component its target has to carry, or `""` when anything
/// will do.
pub(crate) fn requires_for(field_metas: Option<&'static [FieldMeta]>, name: &str) -> &'static str {
    field_metas
        .and_then(|metas| metas.iter().find(|m| m.name == name))
        .map(|m| m.requires)
        .unwrap_or("")
}
