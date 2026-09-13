//! GlobalTransform component — world-space transform matrix.

use glam::{Mat4, Quat, Vec3};

use crate::component::Component;
use crate::reflect::{
    FieldKind, FieldMeta, InspectorVisibility, Reflect, ReflectError, ReflectValue,
};

/// World-space transform matrix, computed from the hierarchy chain.
#[derive(Debug, Clone, Copy)]
pub struct GlobalTransform {
    pub matrix: Mat4,
}

impl GlobalTransform {
    /// Returns the world-space translation extracted from the matrix.
    pub fn translation(&self) -> Vec3 {
        self.matrix.to_scale_rotation_translation().2
    }

    /// Returns the world-space rotation extracted from the matrix.
    pub fn rotation(&self) -> Quat {
        self.matrix.to_scale_rotation_translation().1
    }

    /// Returns the world-space scale approximated from the matrix.
    pub fn scale(&self) -> Vec3 {
        self.lossy_scale()
    }

    /// Returns the world-space scale approximated from the matrix.
    pub fn lossy_scale(&self) -> Vec3 {
        self.matrix.to_scale_rotation_translation().0
    }

    /// Returns `true` when the upper-left 3×3 block of the matrix has detectable shear
    /// (non-orthogonal columns).
    pub fn has_shear(&self, epsilon: f32) -> bool {
        let x = self.matrix.x_axis.truncate();
        let y = self.matrix.y_axis.truncate();
        let z = self.matrix.z_axis.truncate();
        x.dot(y).abs() > epsilon * x.length() * y.length()
            || x.dot(z).abs() > epsilon * x.length() * z.length()
            || y.dot(z).abs() > epsilon * y.length() * z.length()
    }
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
            group: "",
            doc: "World-space transform, recomputed every frame from this entity's \
Transform and its parents'.\n\nRead-only in practice: writing here is overwritten by the next \
propagation pass. Edit Transform instead.",
            type_name: "glam::Mat4",
            kind: FieldKind::Mat4,
            choices: &[],
            bits: &[],
            range: None,
            shown_when: None,
            asset_type: "",
            requires: "",
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
