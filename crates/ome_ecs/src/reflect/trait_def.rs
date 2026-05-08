use super::error::ReflectError;
use super::field::{FieldMeta, InspectorVisibility};
use super::value::ReflectValue;

/// Runtime introspection for component types.
///
/// Implement this trait to expose a component's fields to the editor
/// inspector, serialization, and scripting systems.
///
/// # Example
///
/// ```ignore
/// struct Health {
///     hp: u32,
///     max_hp: u32,
/// }
///
/// impl Reflect for Health {
///     fn reflect_fields(&self) -> &'static [FieldMeta] {
///         &[
///             FieldMeta { name: "hp", type_name: "u32", kind: FieldKind::U32 },
///             FieldMeta { name: "max_hp", type_name: "u32", kind: FieldKind::U32 },
///         ]
///     }
///
///     fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
///         match field {
///             "hp" => Some(ReflectValue::U32(self.hp)),
///             "max_hp" => Some(ReflectValue::U32(self.max_hp)),
///             _ => None,
///         }
///     }
///
///     fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), ReflectError> {
///         match field {
///             "hp" => match value {
///                 ReflectValue::U32(v) => { self.hp = v; Ok(()) }
///                 other => Err(ReflectError::TypeMismatch {
///                     field: "hp".into(), expected: FieldKind::U32, got: other.kind(),
///                 }),
///             },
///             "max_hp" => match value {
///                 ReflectValue::U32(v) => { self.max_hp = v; Ok(()) }
///                 other => Err(ReflectError::TypeMismatch {
///                     field: "max_hp".into(), expected: FieldKind::U32, got: other.kind(),
///                 }),
///             },
///             _ => Err(ReflectError::FieldNotFound(field.into())),
///         }
///     }
///
///     fn reflect_default() -> Self {
///         Health { hp: 100, max_hp: 100 }
///     }
/// }
/// ```
pub trait Reflect: Send + Sync + 'static {
    /// Returns static metadata for all fields of this type.
    fn reflect_fields(&self) -> &'static [FieldMeta];

    /// Gets the value of a field by name.
    fn reflect_get(&self, field: &str) -> Option<ReflectValue>;

    /// Sets the value of a field by name.
    fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), ReflectError>;

    /// Creates an instance with default values (for editor "Add Component").
    fn reflect_default() -> Self
    where
        Self: Sized;

    /// Returns the inspector visibility for this component type.
    fn inspector_visibility() -> InspectorVisibility
    where
        Self: Sized,
    {
        InspectorVisibility::Editable
    }

    /// Returns the editor category for grouping in the "Add Component" menu.
    ///
    /// `None` means uncategorized — the editor shows the type at the top
    /// level of the menu.
    fn category() -> Option<&'static str>
    where
        Self: Sized,
    {
        None
    }
}
