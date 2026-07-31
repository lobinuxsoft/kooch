//! Floats that JSON has no number for.
//!
//! JSON's grammar has no infinity and no NaN. `serde_json` does not refuse
//! them — it writes `null`, which then fails to read back as a number. So
//! a component holding one crossed the editor protocol, arrived as a type
//! error on the far side, and the mirror stopped updating with nothing
//! said about which field did it.
//!
//! Infinity is not exotic here: it is how a physics joint spells "no
//! ceiling on this motor" and "this never breaks".
//!
//! A finite value keeps its ordinary number form, so nothing about the
//! scene format changes for the values scenes actually contain. Only the
//! three that have no number get spelled out as text — which RON could
//! already write as `inf`, and reads back either way through the visitor
//! below.

use std::fmt;

use serde::de::{Deserializer, Unexpected, Visitor};
use serde::ser::Serializer;

/// How a non-finite value is written.
const INFINITY: &str = "inf";
const NEG_INFINITY: &str = "-inf";
const NAN: &str = "NaN";

/// Reads a float written either as a number or as one of the three names.
struct FloatVisitor;

impl<'de> Visitor<'de> for FloatVisitor {
    type Value = f64;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, r#"a number, or "inf", "-inf" or "NaN""#)
    }

    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<f64, E> {
        Ok(v)
    }

    fn visit_f32<E: serde::de::Error>(self, v: f32) -> Result<f64, E> {
        Ok(v as f64)
    }

    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<f64, E> {
        Ok(v as f64)
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<f64, E> {
        Ok(v as f64)
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<f64, E> {
        match v {
            // Beyond what this writes: RON spells them `inf` and `NaN`,
            // and other producers use the long forms. Reading is generous
            // and writing is not, which is the usual arrangement.
            INFINITY | "Infinity" | "+inf" => Ok(f64::INFINITY),
            NEG_INFINITY | "-Infinity" => Ok(f64::NEG_INFINITY),
            NAN | "nan" | "NAN" => Ok(f64::NAN),
            other => Err(E::invalid_value(Unexpected::Str(other), &self)),
        }
    }
}

/// The name a non-finite value is written under, or `None` when it has a
/// perfectly good number.
fn name(value: f64) -> Option<&'static str> {
    match (
        value.is_nan(),
        value.is_infinite(),
        value.is_sign_negative(),
    ) {
        (true, _, _) => Some(NAN),
        (_, true, false) => Some(INFINITY),
        (_, true, true) => Some(NEG_INFINITY),
        _ => None,
    }
}

/// `#[serde(with = "...")]` for an `f32` field.
pub(super) mod f32_repr {
    use super::*;

    pub(in crate::reflect) fn serialize<S: Serializer>(
        value: &f32,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match name(*value as f64) {
            Some(name) => serializer.serialize_str(name),
            None => serializer.serialize_f32(*value),
        }
    }

    pub(in crate::reflect) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<f32, D::Error> {
        deserializer.deserialize_any(FloatVisitor).map(|v| v as f32)
    }
}

/// `#[serde(with = "...")]` for an `f64` field.
pub(super) mod f64_repr {
    use super::*;

    pub(in crate::reflect) fn serialize<S: Serializer>(
        value: &f64,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match name(*value) {
            Some(name) => serializer.serialize_str(name),
            None => serializer.serialize_f64(*value),
        }
    }

    pub(in crate::reflect) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<f64, D::Error> {
        deserializer.deserialize_any(FloatVisitor)
    }
}

#[cfg(test)]
mod tests {
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
}
