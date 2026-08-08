use crate::reflect::ReflectValue;

/// The case that froze a session: a joint's "no ceiling" motor force.
#[test]
fn an_infinity_survives_json() {
    for original in [f32::INFINITY, f32::NEG_INFINITY] {
        let encoded = serde_json::to_string(&ReflectValue::F32(original)).expect("serialises");
        assert!(
            !encoded.contains("null"),
            "a non-finite float must not become null: {encoded}",
        );
        let decoded: ReflectValue = serde_json::from_str(&encoded).expect("deserialises");
        assert_eq!(decoded, ReflectValue::F32(original));
    }
}

#[test]
fn a_nan_survives_json() {
    let encoded = serde_json::to_string(&ReflectValue::F64(f64::NAN)).expect("serialises");
    let decoded: ReflectValue = serde_json::from_str(&encoded).expect("deserialises");
    assert!(
        matches!(decoded, ReflectValue::F64(v) if v.is_nan()),
        "expected a NaN back, got {decoded:?}",
    );
}

/// The values scenes actually hold must keep the shape they had, or
/// every existing `.scene` reads differently than it was written.
#[test]
fn an_ordinary_float_keeps_its_number_form() {
    let encoded = ron::to_string(&ReflectValue::F32(1.5)).expect("serialises");
    assert!(
        encoded.contains("1.5") && !encoded.contains('"'),
        "a finite float must stay a number, got {encoded}",
    );
    let decoded: ReflectValue = ron::from_str(&encoded).expect("deserialises");
    assert_eq!(decoded, ReflectValue::F32(1.5));
}

/// RON writes a non-finite float as a bare `inf`; the visitor has to
/// read that as well as the quoted form.
#[test]
fn a_non_finite_float_round_trips_through_ron() {
    let encoded = ron::to_string(&ReflectValue::F32(f32::INFINITY)).expect("serialises");
    let decoded: ReflectValue = ron::from_str(&encoded).expect("deserialises");
    assert_eq!(decoded, ReflectValue::F32(f32::INFINITY));
}
