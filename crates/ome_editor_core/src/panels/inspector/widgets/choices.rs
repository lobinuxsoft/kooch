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
) {
    if let Some(label) = choice_label_for(value, choices) {
        ui.weak(label);
    } else {
        ui.weak(format!("{value}"));
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

/// Renders an integer field as one checkbox per named bit.
///
/// A collision filter written as a raw number fails silently — two things
/// pass through each other and nothing says why — so the named form is not
/// a convenience. It is the difference between a mistake you can see and
/// one you cannot.
///
/// Bits outside the named set are preserved rather than cleared: a mask
/// authored by a newer editor, or by hand, must survive a visit to this
/// widget untouched.
pub(crate) fn draw_bitmask(
    ui: &mut egui::Ui,
    value: &ReflectValue,
    bits: &'static [FieldChoice],
) -> Option<ReflectValue> {
    let current = reflect_value_as_i64(value)?;
    let mut next = current;

    ui.vertical(|ui| {
        // "All" and "None" earn their place: the default is every bit set,
        // and clicking thirty-two boxes to express "nothing" is how people
        // give up and go back to typing numbers.
        ui.horizontal(|ui| {
            if ui.small_button("All").clicked() {
                next = named_mask(bits) | (current & !named_mask(bits));
            }
            if ui.small_button("None").clicked() {
                next = current & !named_mask(bits);
            }
        });
        for bit in bits {
            let mut set = current & bit.value != 0;
            if ui.checkbox(&mut set, bit.label).changed() {
                next = match set {
                    true => current | bit.value,
                    false => current & !bit.value,
                };
            }
        }
    });

    (next != current).then(|| reflect_value_from_i64(value, next))?
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
