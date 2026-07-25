/// Metadata describing a single field of a reflected component.
#[derive(Debug, Clone, Copy)]
pub struct FieldMeta {
    /// Field name (e.g. `"hp"`, `"position"`).
    pub name: &'static str,
    /// Full type name (e.g. `"f32"`, `"glam::Vec3"`).
    pub type_name: &'static str,
    /// Discriminant for the field's value type.
    pub kind: FieldKind,
    /// Optional enum-like choice set for integer fields. When non-empty,
    /// the editor inspector renders the field as a dropdown instead of
    /// a free-form numeric input. Ignored for non-integer `kind`s.
    pub choices: &'static [FieldChoice],
    /// When set, the field is only meaningful while another field of the
    /// same component holds one of the listed values — see
    /// [`FieldCondition`]. `None` means always shown.
    pub shown_when: Option<&'static FieldCondition>,
    /// For [`FieldKind::AssetRef`] fields, the static asset type the
    /// field expects (e.g. `"ome_render::meshlet::MeshletMesh"`). The
    /// inspector passes this to `AssetDatabase::entries_of_type` to
    /// build the picker dropdown. `""` for non-asset fields.
    pub asset_type: &'static str,
}

/// A labelled value in a [`FieldMeta::choices`] set.
///
/// The `value` is stored as `i64` so a single representation covers
/// every integer [`FieldKind`]; it is narrowed back to the target type
/// when applied.
#[derive(Debug, Clone, Copy)]
pub struct FieldChoice {
    /// Human-readable label shown in the dropdown.
    pub label: &'static str,
    /// Underlying integer value.
    pub value: i64,
}

/// Makes a field's relevance depend on a discriminant field beside it.
///
/// Reflection has no enum representation, so a variant is a `u32`
/// discriminant with every variant's parameters side by side. The storage
/// has to stay that way — switching variant and back must not lose the
/// other one's values — but showing a capsule's `half_height` while a
/// sphere is selected implies it does something.
///
/// This is the display half: which fields the current variant actually
/// reads. Nothing here affects storage, serialisation or a scene
/// round-trip.
#[derive(Debug, Clone, Copy)]
pub struct FieldCondition {
    /// Name of the discriminant field, on the same component.
    pub field: &'static str,
    /// Values of that field for which the annotated field is meaningful.
    ///
    /// `i64` for the same reason [`FieldChoice::value`] is: one
    /// representation covers every integer [`FieldKind`].
    pub values: &'static [i64],
}

impl FieldCondition {
    /// Whether the annotated field should be shown, given the
    /// discriminant's current value.
    ///
    /// `None` — the named field is absent from the component — reads as
    /// shown. A condition pointing at a field that does not exist is a
    /// typo in an attribute, and it should look like one rather than like
    /// a field that silently vanished.
    pub fn is_met(&self, discriminant: Option<i64>) -> bool {
        match discriminant {
            Some(value) => self.values.contains(&value),
            None => true,
        }
    }
}

/// Discriminant for supported reflected field types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    F32,
    F64,
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    Bool,
    String,
    Vec2,
    Vec3,
    Vec4,
    Quat,
    Mat4,
    /// Reference to an asset, addressed by [`Guid`]. The inspector
    /// renders this as a typed dropdown picker (filtered by
    /// [`FieldMeta::asset_type`]) rather than a free-form text field.
    AssetRef,
    /// Struct that also implements [`Reflect`].
    Nested,
}

/// Controls how the inspector displays a reflected component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InspectorVisibility {
    /// Component is not shown in the inspector.
    Hidden,
    /// Component is shown but fields are not editable.
    ReadOnly,
    /// Component is fully editable (default).
    #[default]
    Editable,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPHERE: i64 = 0;
    const CUBOID: i64 = 1;
    const CAPSULE: i64 = 2;

    static RADIUS_WHEN: FieldCondition = FieldCondition {
        field: "shape",
        values: &[SPHERE, CAPSULE],
    };

    #[test]
    fn a_condition_is_met_only_by_its_listed_values() {
        assert!(RADIUS_WHEN.is_met(Some(SPHERE)));
        assert!(RADIUS_WHEN.is_met(Some(CAPSULE)));
        assert!(!RADIUS_WHEN.is_met(Some(CUBOID)));
    }

    /// A condition naming a field the component does not have reads as
    /// met. A typo in an attribute should look like a mistake, not like a
    /// field that silently vanished — the field the author annotated is
    /// still the field they wanted to see.
    #[test]
    fn a_missing_discriminant_shows_the_field() {
        assert!(RADIUS_WHEN.is_met(None));
    }

    /// An empty value list hides the field for every discriminant. Not a
    /// useful annotation, but it must not read as "always shown", or a
    /// mistake there would be invisible.
    #[test]
    fn an_empty_condition_hides_the_field() {
        static NEVER: FieldCondition = FieldCondition {
            field: "shape",
            values: &[],
        };
        assert!(!NEVER.is_met(Some(SPHERE)));
        assert!(NEVER.is_met(None), "a missing discriminant still shows");
    }
}
