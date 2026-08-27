//! GlobalTransform component — world-space transform matrix.

use glam::{Mat4, Quat, Vec3};

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

impl GlobalTransform {
    /// Returns the world-space translation extracted from the matrix.
    ///
    /// This one is not lossy — translation is the fourth column of an
    /// affine matrix, untouched by shear in the rotation/scale block.
    pub fn translation(&self) -> Vec3 {
        self.matrix.to_scale_rotation_translation().2
    }

    /// Returns the world-space rotation extracted from the matrix.
    ///
    /// Lossy under shear — see `has_shear`. Uses a polar-style
    /// decomposition via `Mat4::to_scale_rotation_translation`, which
    /// returns the closest orthonormal rotation when the upper-left
    /// 3×3 block contains shear.
    pub fn rotation(&self) -> Quat {
        self.matrix.to_scale_rotation_translation().1
    }

    /// Returns the world-space scale approximated from the matrix.
    ///
    /// Lossy under shear. Alias for [`Self::lossy_scale`] kept for
    /// callers that don't need the explicit reminder in the name.
    pub fn scale(&self) -> Vec3 {
        self.lossy_scale()
    }

    /// Returns the world-space scale approximated from the matrix.
    ///
    /// Named after Unity's `Transform.lossyScale` for honesty: when
    /// the upper-left 3×3 block contains shear (which happens as soon
    /// as a non-uniformly-scaled parent composes with a rotated
    /// child), no `Vec3` scale exists that reproduces the matrix
    /// exactly. The value returned here is the best-fit diagonal —
    /// useful for display and debugging but not for round-tripping.
    pub fn lossy_scale(&self) -> Vec3 {
        self.matrix.to_scale_rotation_translation().0
    }

    /// Returns `true` when the upper-left 3×3 block of the matrix has
    /// detectable shear (non-orthogonal columns).
    ///
    /// Shear emerges when an ancestor with non-uniform scale composes
    /// with a rotated descendant — the product `R · S_nonuniform · R'`
    /// is not a pure rotation × scale and can't be stored losslessly
    /// in `Transform { rotation, scale }`. See issue #214 for the
    /// architectural discussion.
    ///
    /// `epsilon` gates tolerance against floating-point noise; a
    /// reasonable default is `1e-4`.
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
