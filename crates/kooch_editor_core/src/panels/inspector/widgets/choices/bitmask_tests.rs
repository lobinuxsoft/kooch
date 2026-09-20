use super::*;

static BITS: &[(&str, i64)] = &[("A", 1 << 0), ("B", 1 << 1)];

/// The widget may only touch the bits it names. A mask authored by hand or by a newer editor has to
/// survive a visit — silently clearing the high half would be a filtering bug introduced by
/// *looking* at the field.
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

/// 🔴 A layer mask is drawn from the project's own names, and one cell per bit means bit `n` is
/// `1 << n` however the table is filled in: a name out of step would tick the wrong layer.
#[test]
fn a_layer_cell_is_its_own_bit() {
    let names = kooch_core::layers::LayerNames::default();
    let labels = names.labels();
    let cells: Vec<(&str, i64)> = labels
        .iter()
        .enumerate()
        .map(|(bit, label)| (label.as_str(), 1i64 << bit))
        .collect();
    assert_eq!(cells.len(), kooch_core::layers::LAYER_COUNT);
    assert_eq!(cells[0], ("Default", 1));
    assert_eq!(cells[5].1, 1 << 5);
    assert_eq!(named_mask(&cells), -1i64 as i64 & 0xffff_ffff);
}
