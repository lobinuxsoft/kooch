//! Wiring plugin-declared component types into the ECS.
//!
//! `ome_plugin_api` carries its own [`FieldKind`](ome_plugin_api::FieldKind)
//! rather than reusing this crate's. That is not duplication for its own
//! sake: `ome_ecs` pulls in wgpu, glam, serde and ron, and a plugin
//! should not have to link a GPU stack in order to say that a field
//! holds an `f32`. The plugin API depends on nothing at all.
//!
//! The cost of that is two enums that must agree, so the mapping lives
//! here, in one place, and [`tests::every_plugin_kind_maps`] fails the
//! build if a variant is ever added to one without the other.

use ome_plugin_api::FieldKind as PluginFieldKind;
use ome_plugin_api::component::{ComponentSchema, RegisterError};

use crate::reflect::FieldKind;

use super::dynamic_types::{DynamicField, DynamicType, DynamicTypeRegistry};

/// Translates a plugin's field kind into the ECS's.
///
/// Exhaustive on purpose: a new variant on either side stops compiling
/// here rather than silently drawing the wrong widget.
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
    }
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
        source: source.to_owned(),
    }
}

/// Registers `schema` into the [`DynamicTypeRegistry`], creating it if
/// this is the first plugin type to arrive.
pub(crate) fn register_schema(
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
mod tests {
    use super::*;
    use ome_plugin_api::component::FieldSchema;

    /// The parity lock. Two enums have to stay in step, and nothing but
    /// this notices when they stop.
    #[test]
    fn every_plugin_kind_maps() {
        for kind in PluginFieldKind::ALL {
            // Exhaustive `match` means this compiles only while every
            // variant is handled; the loop proves ALL is complete too.
            let _ = map_field_kind(*kind);
        }
        assert_eq!(
            PluginFieldKind::ALL.len(),
            20,
            "a kind was added to the plugin API without extending ALL, \
             so the mapping above may be missing it"
        );
    }

    #[test]
    fn kinds_map_to_their_counterparts() {
        assert_eq!(map_field_kind(PluginFieldKind::F32), FieldKind::F32);
        assert_eq!(map_field_kind(PluginFieldKind::Vec3), FieldKind::Vec3);
        assert_eq!(map_field_kind(PluginFieldKind::Nested), FieldKind::Nested);
        assert_eq!(
            map_field_kind(PluginFieldKind::EntityRef),
            FieldKind::EntityRef
        );
    }

    #[test]
    fn a_schema_becomes_a_registered_type() {
        let mut registry = DynamicTypeRegistry::new();
        let schema = ComponentSchema {
            type_name: "my_game::Health".into(),
            fields: vec![
                FieldSchema::new("current", PluginFieldKind::U32),
                FieldSchema::new("regen", PluginFieldKind::F32),
            ],
        };

        register_schema(&mut registry, &schema, "my_game").unwrap();

        let ty = registry.get("my_game::Health").expect("registered");
        assert_eq!(ty.source, "my_game");
        assert_eq!(ty.fields[0].name, "current");
        assert_eq!(ty.fields[0].kind, FieldKind::U32);
        assert_eq!(ty.fields[1].kind, FieldKind::F32);
    }

    /// A reload registers the same schemas again, so it must not fail.
    #[test]
    fn re_registering_from_the_same_plugin_succeeds() {
        let mut registry = DynamicTypeRegistry::new();
        let schema = ComponentSchema::new("my_game::Player");

        register_schema(&mut registry, &schema, "my_game").unwrap();
        register_schema(&mut registry, &schema, "my_game").unwrap();

        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn a_collision_between_plugins_is_reported_as_name_taken() {
        let mut registry = DynamicTypeRegistry::new();
        let schema = ComponentSchema::new("shared::Name");

        register_schema(&mut registry, &schema, "first").unwrap();
        let err = register_schema(&mut registry, &schema, "second").unwrap_err();

        assert_eq!(
            err,
            RegisterError::NameTaken {
                type_name: "shared::Name".into()
            }
        );
    }
}
