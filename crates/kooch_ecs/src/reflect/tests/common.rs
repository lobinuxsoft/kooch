use crate::reflect::{FieldKind, FieldMeta, Reflect, ReflectError, ReflectValue};

// -- Test component with manual Reflect impl -----------------------------

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Health {
    pub(super) hp: u32,
    pub(super) max_hp: u32,
}

impl crate::component::traits::Component for Health {}

impl Reflect for Health {
    fn reflect_fields(&self) -> &'static [FieldMeta] {
        static FIELDS: &[FieldMeta] = &[
            FieldMeta {
                name: "hp",
                type_name: "u32",
                kind: FieldKind::U32,
                choices: &[],
                bits: &[],
                shown_when: None,
                asset_type: "",
                requires: "",
                doc: "",
                group: "",
            },
            FieldMeta {
                name: "max_hp",
                type_name: "u32",
                kind: FieldKind::U32,
                choices: &[],
                bits: &[],
                shown_when: None,
                asset_type: "",
                requires: "",
                doc: "",
                group: "",
            },
        ];
        FIELDS
    }

    fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
        match field {
            "hp" => Some(ReflectValue::U32(self.hp)),
            "max_hp" => Some(ReflectValue::U32(self.max_hp)),
            _ => None,
        }
    }

    fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), ReflectError> {
        match field {
            "hp" => match value {
                ReflectValue::U32(v) => {
                    self.hp = v;
                    Ok(())
                }
                other => Err(ReflectError::TypeMismatch {
                    field: "hp".into(),
                    expected: FieldKind::U32,
                    got: other.kind(),
                }),
            },
            "max_hp" => match value {
                ReflectValue::U32(v) => {
                    self.max_hp = v;
                    Ok(())
                }
                other => Err(ReflectError::TypeMismatch {
                    field: "max_hp".into(),
                    expected: FieldKind::U32,
                    got: other.kind(),
                }),
            },
            _ => Err(ReflectError::FieldNotFound(field.into())),
        }
    }

    fn reflect_default() -> Self {
        Health {
            hp: 100,
            max_hp: 100,
        }
    }
}

// -- GPU component with Reflect ------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(super) struct Position {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) z: f32,
    pub(super) _pad: f32,
}

impl crate::component::traits::Component for Position {}

impl Reflect for Position {
    fn reflect_fields(&self) -> &'static [FieldMeta] {
        static FIELDS: &[FieldMeta] = &[
            FieldMeta {
                name: "x",
                type_name: "f32",
                kind: FieldKind::F32,
                choices: &[],
                bits: &[],
                shown_when: None,
                asset_type: "",
                requires: "",
                doc: "",
                group: "",
            },
            FieldMeta {
                name: "y",
                type_name: "f32",
                kind: FieldKind::F32,
                choices: &[],
                bits: &[],
                shown_when: None,
                asset_type: "",
                requires: "",
                doc: "",
                group: "",
            },
            FieldMeta {
                name: "z",
                type_name: "f32",
                kind: FieldKind::F32,
                choices: &[],
                bits: &[],
                shown_when: None,
                asset_type: "",
                requires: "",
                doc: "",
                group: "",
            },
        ];
        FIELDS
    }

    fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
        match field {
            "x" => Some(ReflectValue::F32(self.x)),
            "y" => Some(ReflectValue::F32(self.y)),
            "z" => Some(ReflectValue::F32(self.z)),
            _ => None,
        }
    }

    fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), ReflectError> {
        match field {
            "x" => match value {
                ReflectValue::F32(v) => {
                    self.x = v;
                    Ok(())
                }
                other => Err(ReflectError::TypeMismatch {
                    field: "x".into(),
                    expected: FieldKind::F32,
                    got: other.kind(),
                }),
            },
            "y" => match value {
                ReflectValue::F32(v) => {
                    self.y = v;
                    Ok(())
                }
                other => Err(ReflectError::TypeMismatch {
                    field: "y".into(),
                    expected: FieldKind::F32,
                    got: other.kind(),
                }),
            },
            "z" => match value {
                ReflectValue::F32(v) => {
                    self.z = v;
                    Ok(())
                }
                other => Err(ReflectError::TypeMismatch {
                    field: "z".into(),
                    expected: FieldKind::F32,
                    got: other.kind(),
                }),
            },
            _ => Err(ReflectError::FieldNotFound(field.into())),
        }
    }

    fn reflect_default() -> Self {
        Position {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            _pad: 0.0,
        }
    }
}
