use super::*;
use crate::reflect::FieldMeta;

#[derive(Default)]
struct Probe {
    torque: f32,
}

impl Reflect for Probe {
    fn reflect_fields(&self) -> &'static [FieldMeta] {
        &[]
    }

    fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
        (field == "torque").then_some(ReflectValue::F32(self.torque))
    }

    fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), crate::ReflectError> {
        match (field, value) {
            ("torque", ReflectValue::F32(torque)) => {
                self.torque = torque;
                Ok(())
            }
            _ => Err(crate::ReflectError::FieldNotFound(field.to_owned())),
        }
    }

    fn reflect_default() -> Self {
        Self::default()
    }
}

impl DataAsset for Probe {
    const EXTENSION: &'static str = "probe";
}

/// An old file must keep what still fits, or a renamed field costs the author every value.
#[test]
fn a_gone_field_keeps_the_rest() {
    let text = data_text(&[
        ("gone".to_owned(), ReflectValue::F32(1.0)),
        ("torque".to_owned(), ReflectValue::F32(9.0)),
    ]);
    let fields: DataFields = ron::from_str(&text).expect("the text it wrote");
    assert_eq!(from_fields::<Probe>(fields).torque, 9.0);
}
