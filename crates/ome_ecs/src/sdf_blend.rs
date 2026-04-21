//! SDF blend component — optional CSG blending metadata for SDF shapes.
//!
//! Attach `SdfBlend` to an entity that carries any SDF shape component
//! (`SdfSphere`, `SdfBox`, etc.) to control how its distance combines
//! with the accumulated scene in the ray-march shader. Entities without
//! `SdfBlend` default to `MODE_REPLACE` (hard union), matching the
//! pre-CSG behavior.

use crate::component::Component;
use crate::reflect::FieldChoice;

#[allow(unused_imports)]
use crate::Reflect;

/// Hard union (boolean OR). The shape adds to the scene with a sharp
/// seam. This is the default behavior when the component is absent.
pub const MODE_REPLACE: u32 = 0;

/// Smooth union. `smoothness` controls the blend radius in world units;
/// larger values produce softer transitions between shapes.
pub const MODE_SMOOTH_UNION: u32 = 1;

/// Smooth intersection. Only the overlap of this shape with the scene
/// is kept, with soft falloff controlled by `smoothness`.
pub const MODE_SMOOTH_INTERSECTION: u32 = 2;

/// Smooth subtraction. Carves this shape out of the scene with a soft
/// transition controlled by `smoothness`.
pub const MODE_SMOOTH_SUBTRACTION: u32 = 3;

/// Dropdown hint for the `mode` field in the editor inspector. The
/// editor reads this slice via [`FieldMeta::choices`](crate::reflect::FieldMeta::choices)
/// and renders a ComboBox instead of a free-form numeric input.
pub const MODE_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Replace",
        value: MODE_REPLACE as i64,
    },
    FieldChoice {
        label: "SmoothUnion",
        value: MODE_SMOOTH_UNION as i64,
    },
    FieldChoice {
        label: "SmoothIntersection",
        value: MODE_SMOOTH_INTERSECTION as i64,
    },
    FieldChoice {
        label: "SmoothSubtraction",
        value: MODE_SMOOTH_SUBTRACTION as i64,
    },
];

/// Optional CSG blend metadata for an SDF shape entity.
///
/// # Fields
///
/// - `mode`: one of the `MODE_*` constants above. Values outside that
///   range fall back to hard union in the shader.
/// - `smoothness`: blend radius in world units. Only used when `mode`
///   is a smooth variant. `0.0` matches hard union.
///
/// # Default
///
/// - `mode`: `MODE_REPLACE`
/// - `smoothness`: 0.0
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "SDF")]
pub struct SdfBlend {
    /// Blend mode. Use one of the `MODE_*` constants.
    #[reflect(choices = MODE_CHOICES)]
    pub mode: u32,
    /// Blend radius in world units (smooth modes only).
    pub smoothness: f32,
}

impl Default for SdfBlend {
    fn default() -> Self {
        Self {
            mode: MODE_REPLACE,
            smoothness: 0.0,
        }
    }
}

impl Component for SdfBlend {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::Reflect;

    #[test]
    fn default_is_replace() {
        let b = SdfBlend::default();
        assert_eq!(b.mode, MODE_REPLACE);
        assert_eq!(b.smoothness, 0.0);
    }

    #[test]
    fn reflect_fields() {
        let b = SdfBlend::default();
        let fields = b.reflect_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        assert_eq!(names, &["mode", "smoothness"]);
    }

    #[test]
    fn mode_constants_are_distinct() {
        let modes = [
            MODE_REPLACE,
            MODE_SMOOTH_UNION,
            MODE_SMOOTH_INTERSECTION,
            MODE_SMOOTH_SUBTRACTION,
        ];
        for (i, a) in modes.iter().enumerate() {
            for b in &modes[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
}
