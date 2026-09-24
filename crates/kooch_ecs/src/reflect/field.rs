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
    /// Optional named bits for an integer field used as a bitmask. When non-empty, the inspector
    /// renders a checkbox per entry instead of a number. Ignored for non-integer `kind`s.
    pub bits: &'static [FieldChoice],
    /// Bounds and granularity for a numeric field. When set, the inspector draws a SLIDER over the
    /// range instead of an unbounded drag.
    pub range: Option<&'static FieldRange>,
    /// When set, the field is only meaningful while another field of the
    /// same component holds one of the listed values — see
    /// [`FieldCondition`]. `None` means always shown.
    pub shown_when: Option<&'static FieldCondition>,
    /// For [`FieldKind::AssetRef`] fields, the static asset type the field expects (e.g.
    /// `"kooch_render::meshlet::MeshletMesh"`). The inspector passes this to
    /// `AssetDatabase::entries_of_type` to build the picker dropdown. `""` for non-asset fields.
    pub asset_type: &'static str,
    /// For [`FieldKind::EntityRef`] fields, the short name of a component the target must carry
    /// (e.g. `"PhysicsBody"`). `""` when anything will do.
    pub requires: &'static str,
    /// The field's doc comment, shown as a tooltip in the Inspector. `""` when the field has none.
    pub doc: &'static str,
    /// Heading this field is drawn under in the Inspector, from `#[reflect(group = "...")]`. `""`
    /// for a field that belongs to no group and is drawn before the first heading.
    pub group: &'static str,
    /// For a list of structs, the element's fields, so a nested number keeps its range and doc
    /// (#1209). `&[]` for every other field.
    pub fields: &'static [FieldMeta],
    /// A mask over the project's own layer names (#1218), from `#[reflect(layers)]`. The inspector
    /// draws a box per layer, named by the `.layers` table rather than by anything in the code.
    pub layers: bool,
    /// One of the project's layers, from `#[reflect(layer)]`: the inspector draws a dropdown of the
    /// `.layers` names rather than a number (#1302).
    pub layer: bool,
}

/// Bounds and granularity for a numeric field — see [`FieldMeta::range`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldRange {
    pub min: f64,
    pub max: f64,
    /// Granularity. `0.0` leaves it to the widget.
    pub step: f64,
}

/// A labelled value in a [`FieldMeta::choices`] set.
#[derive(Debug, Clone, Copy)]
pub struct FieldChoice {
    /// Human-readable label shown in the dropdown.
    pub label: &'static str,
    /// Underlying integer value.
    pub value: i64,
}

/// Makes a field's relevance depend on a discriminant field beside it.
#[derive(Debug, Clone, Copy)]
pub struct FieldCondition {
    /// Name of the discriminant field, on the same component.
    pub field: &'static str,
    /// Values of that field for which the annotated field is meaningful.
    pub values: &'static [i64],
}

impl FieldCondition {
    /// Whether the annotated field should be shown, given the discriminant's current value.
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
    /// Reference to an asset, addressed by `Guid`. The inspector
    /// renders this as a typed dropdown picker (filtered by
    /// [`FieldMeta::asset_type`]) rather than a free-form text field.
    AssetRef,
    /// Reference to another entity, addressed by [`EntityGuid`](crate::persistent_id::EntityGuid)
    /// once saved. The inspector renders this as an entity picker / drop target rather than a text
    /// field.
    EntityRef,
    /// Struct that also implements `Reflect`; its value is [`ReflectValue::Struct`](super::ReflectValue::Struct).
    Nested,
    /// An ordered list; the value carries what each item is (#1201).
    List,
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
mod tests;
