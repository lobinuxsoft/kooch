//! GlobalTransform component — world-space transform matrix.

use glam::Mat4;

use crate::component::Component;
use crate::reflect::{
    FieldKind, FieldMeta, InspectorVisibility, Reflect, ReflectError, ReflectValue,
};

/// World-space transform matrix, computed from the hierarchy chain.
///
/// For root entities: `GlobalTransform = Transform::to_matrix()`.
/// For children: `GlobalTransform = parent.GlobalTransform * local.to_matrix()`.
///
/// This component is read-only from the user's perspective — it is
/// recomputed every frame by the transform propagation system.
#[derive(Debug, Clone, Copy)]
pub struct GlobalTransform {
    pub matrix: Mat4,
}

impl Component for GlobalTransform {}

impl Default for GlobalTransform {
    fn default() -> Self {
        Self {
            matrix: Mat4::IDENTITY,
        }
    }
}

impl Reflect for GlobalTransform {
    fn reflect_fields(&self) -> &'static [FieldMeta] {
        static FIELDS: &[FieldMeta] = &[FieldMeta {
            name: "matrix",
            type_name: "glam::Mat4",
            kind: FieldKind::Mat4,
            choices: &[],
        }];
        FIELDS
    }

    fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
        match field {
            "matrix" => Some(ReflectValue::Mat4(self.matrix)),
            _ => None,
        }
    }

    fn reflect_set(&mut self, _field: &str, _value: ReflectValue) -> Result<(), ReflectError> {
        Err(ReflectError::ReadOnly)
    }

    fn reflect_default() -> Self {
        Self::default()
    }

    fn inspector_visibility() -> InspectorVisibility {
        InspectorVisibility::ReadOnly
    }
}
