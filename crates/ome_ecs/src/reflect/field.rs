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
