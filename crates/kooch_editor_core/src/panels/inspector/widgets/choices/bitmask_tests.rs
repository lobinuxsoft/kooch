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
