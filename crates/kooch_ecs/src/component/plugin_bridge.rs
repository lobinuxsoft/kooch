//! Wiring plugin-declared component types into the ECS.

use kooch_plugin_api::FieldKind as PluginFieldKind;
use kooch_plugin_api::component::{ComponentSchema, RegisterError};

use crate::reflect::FieldKind;

use super::dynamic_types::{DynamicField, DynamicType, DynamicTypeRegistry};

/// Translates a plugin's field kind into the ECS's.
pub(crate) const fn map_field_kind(kind: PluginFieldKind) -> FieldKind {
    match kind {
        PluginFieldKind::F32 => FieldKind::F32,
        PluginFieldKind::F64 => FieldKind::F64,
        PluginFieldKind::U8 => FieldKind::U8,
        PluginFieldKind::U16 => FieldKind::U16,
        PluginFieldKind::U32 => FieldKind::U32,
        PluginFieldKind::U64 => FieldKind::U64,
        PluginFieldKind::I8 => FieldKind::I8,
        PluginFieldKind::I16 => FieldKind::I16,
        PluginFieldKind::I32 => FieldKind::I32,
        PluginFieldKind::I64 => FieldKind::I64,
        PluginFieldKind::Bool => FieldKind::Bool,
        PluginFieldKind::String => FieldKind::String,
        PluginFieldKind::Vec2 => FieldKind::Vec2,
        PluginFieldKind::Vec3 => FieldKind::Vec3,
        PluginFieldKind::Vec4 => FieldKind::Vec4,
        PluginFieldKind::Quat => FieldKind::Quat,
        PluginFieldKind::Mat4 => FieldKind::Mat4,
        PluginFieldKind::AssetRef => FieldKind::AssetRef,
        PluginFieldKind::EntityRef => FieldKind::EntityRef,
        PluginFieldKind::Nested => FieldKind::Nested,
        PluginFieldKind::List => FieldKind::List,
    }
}

/// Translates the ECS's field kind into a plugin's.
pub const fn to_plugin_field_kind(kind: FieldKind) -> PluginFieldKind {
    match kind {
        FieldKind::F32 => PluginFieldKind::F32,
        FieldKind::F64 => PluginFieldKind::F64,
        FieldKind::U8 => PluginFieldKind::U8,
        FieldKind::U16 => PluginFieldKind::U16,
        FieldKind::U32 => PluginFieldKind::U32,
        FieldKind::U64 => PluginFieldKind::U64,
        FieldKind::I8 => PluginFieldKind::I8,
        FieldKind::I16 => PluginFieldKind::I16,
        FieldKind::I32 => PluginFieldKind::I32,
        FieldKind::I64 => PluginFieldKind::I64,
        FieldKind::Bool => PluginFieldKind::Bool,
        FieldKind::String => PluginFieldKind::String,
        FieldKind::Vec2 => PluginFieldKind::Vec2,
        FieldKind::Vec3 => PluginFieldKind::Vec3,
        FieldKind::Vec4 => PluginFieldKind::Vec4,
        FieldKind::Quat => PluginFieldKind::Quat,
        FieldKind::Mat4 => PluginFieldKind::Mat4,
        FieldKind::AssetRef => PluginFieldKind::AssetRef,
        FieldKind::EntityRef => PluginFieldKind::EntityRef,
        FieldKind::Nested => PluginFieldKind::Nested,
        FieldKind::List => PluginFieldKind::List,
    }
}

/// Describes a project's own component type to the engine.
pub fn declare_component<T>(engine: &mut dyn kooch_plugin_api::Engine) -> Result<(), RegisterError>
where
    T: crate::reflect::Reflect + Default,
{
    let probe = T::default();
    // The probe is the type's own `Default`, so its values are the ones a freshly added component
    // should hold. They used to be read for their metadata and then dropped, leaving the editor
    // able to say a component has two `f32` but not that they are 20 and 8.
    let defaults: Vec<(String, crate::reflect::ReflectValue)> = probe
        .reflect_fields()
        .iter()
        .filter_map(|meta| Some((meta.name.to_owned(), probe.reflect_get(meta.name)?)))
        .collect();
    let schema = ComponentSchema {
        type_name: std::any::type_name::<T>().to_owned(),
        fields: probe
            .reflect_fields()
            .iter()
            .map(|meta| kooch_plugin_api::component::FieldSchema {
                name: meta.name.to_owned(),
                kind: to_plugin_field_kind(meta.kind),
                doc: meta.doc.to_owned(),
            })
            .collect(),
        // Same encoding a scene file uses, so there is one serialised
        // form for these values rather than two.
        defaults: ron::to_string(&defaults).unwrap_or_default(),
    };
    engine.register_component(schema)
}

/// Converts a plugin's schema into a registrable type.
pub(crate) fn to_dynamic_type(schema: &ComponentSchema, source: &str) -> DynamicType {
    DynamicType {
        type_name: schema.type_name.clone(),
        fields: schema
            .fields
            .iter()
            .map(|f| DynamicField {
                name: f.name.clone(),
                kind: map_field_kind(f.kind),
            })
            .collect(),
        // A plugin built against an older API sends nothing here, and a malformed payload is not
        // worth refusing the whole type over: the component still appears, it just starts at its
        // field kinds' zero values, which is what happened before this existed.
        defaults: ron::from_str(&schema.defaults).unwrap_or_default(),
        source: source.to_owned(),
    }
}

/// Registers `schema` into the [`DynamicTypeRegistry`], creating it if
/// this is the first plugin type to arrive.
pub fn register_schema(
    registry: &mut DynamicTypeRegistry,
    schema: &ComponentSchema,
    source: &str,
) -> Result<(), RegisterError> {
    registry
        .register(to_dynamic_type(schema, source))
        .map_err(|_owner| RegisterError::NameTaken {
            type_name: schema.type_name.clone(),
        })
}

#[cfg(test)]
mod tests;
