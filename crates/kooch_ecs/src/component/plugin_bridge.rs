//! Wiring plugin-declared component types into the ECS.
//!
//! `kooch_plugin_api` carries its own [`FieldKind`](kooch_plugin_api::FieldKind)
//! rather than reusing this crate's. That is not duplication for its own
//! sake: `kooch_ecs` pulls in wgpu, glam, serde and ron, and a plugin
//! should not have to link a GPU stack in order to say that a field
//! holds an `f32`. The plugin API depends on nothing at all.
//!
//! The cost of that is two enums that must agree, so the mapping lives
//! here, in one place, and [`tests::every_plugin_kind_maps`] fails the
//! build if a variant is ever added to one without the other.

use kooch_plugin_api::FieldKind as PluginFieldKind;
use kooch_plugin_api::component::{ComponentSchema, RegisterError};

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

/// Translates the ECS's field kind into a plugin's.
///
/// The direction a *project* needs: it owns the Rust types, so it reads
/// its own `FieldMeta` and describes them outward. Exhaustive for the
/// same reason as its inverse.
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
    }
}

/// Describes a project's own component type to the engine.
///
/// A project links `kooch_ecs`, so it can read `T`'s reflection and build
/// the schema itself — the editor's codegen only has to name the type,
/// never parse its fields. `Default` provides the instance
/// `Reflect::reflect_fields` needs; every editor-authored component
/// derives it already, because `insert_default_reflected` requires it.
///
/// # The name is derived, not supplied
///
/// It comes from [`std::any::type_name`], which is what
/// [`ComponentRegistry`](super::ComponentRegistry) and the remote
/// protocol already key components by. Letting a caller pass its own
/// string produced two names for one type: the editor listed a component
/// under the codegen's spelling and then asked the running project to add
/// it, which answered `UnknownComponent` because it had registered the
/// other one. One source, no divergence.
///
/// This is what a generated project's plugin calls, once per component.
pub fn declare_component<T>(engine: &mut dyn kooch_plugin_api::Engine) -> Result<(), RegisterError>
where
    T: crate::reflect::Reflect + Default,
{
    let probe = T::default();
    // The probe is the type's own `Default`, so its values are the ones a
    // freshly added component should hold. They used to be read for their
    // metadata and then dropped, leaving the editor able to say a
    // component has two `f32` but not that they are 20 and 8.
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
        // A plugin built against an older API sends nothing here, and a
        // malformed payload is not worth refusing the whole type over:
        // the component still appears, it just starts at its field kinds'
        // zero values, which is what happened before this existed.
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
mod tests {
    use super::*;
    use kooch_plugin_api::component::FieldSchema;

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

    /// The two directions must be inverses, or a project describing its
    /// own component would produce a schema the engine reads back as a
    /// different type of field.
    #[test]
    fn the_mapping_round_trips_both_ways() {
        for kind in PluginFieldKind::ALL {
            assert_eq!(
                to_plugin_field_kind(map_field_kind(*kind)),
                *kind,
                "{kind:?} did not survive a round trip"
            );
        }
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

    /// An `Engine` that records what a plugin declared.
    #[derive(Default)]
    struct Recorder {
        schemas: Vec<ComponentSchema>,
    }

    impl kooch_plugin_api::Engine for Recorder {
        fn spawn_entity(&mut self) -> Option<u64> {
            None
        }
        fn despawn_entity(&mut self, _: u64) -> bool {
            false
        }
        fn register_component(&mut self, schema: ComponentSchema) -> Result<(), RegisterError> {
            self.schemas.push(schema);
            Ok(())
        }
        fn add_system(&mut self, _: kooch_plugin_api::Stage, _: kooch_plugin_api::PluginSystem) {}
        fn log(&self, _: &str) {}
        fn set_data(&mut self, _: &str, _: &[u8]) {}
        fn get_data(&self, _: &str) -> Option<&[u8]> {
            None
        }
    }

    /// The bug a real drag-drop exposed: the editor listed a component
    /// under the codegen's spelling and then asked the running project to
    /// add it, which answered `UnknownComponent` because its registry had
    /// keyed the type by `type_name`. Declaring must produce exactly the
    /// name the registry uses, or the two halves disagree silently.
    #[test]
    fn a_declared_type_is_named_the_way_the_registry_names_it() {
        let mut recorder = Recorder::default();
        declare_component::<crate::transform::Transform>(&mut recorder).unwrap();

        assert_eq!(
            recorder.schemas[0].type_name,
            std::any::type_name::<crate::transform::Transform>(),
            "a declared name that is not the type's own name cannot be resolved by the project"
        );
        assert!(
            !recorder.schemas[0].fields.is_empty(),
            "Transform has reflected fields; the schema must carry them"
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
            defaults: r#"[("current", U32(100)), ("regen", F32(1.5))]"#.into(),
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

    /// A component the editor never compiled has to arrive with the
    /// values its author chose, not with the zeroes of its field kinds.
    ///
    /// This is the whole point of `defaults`: a `GroundMovement` that
    /// accelerates at 0 toward a top speed of 0 is indistinguishable from
    /// a broken component, and that is what adding one to a prefab
    /// produced before this existed.
    #[test]
    fn a_declared_component_carries_the_values_its_default_chose() {
        // What a plugin sends: its own `Default`, serialised the way a
        // scene file writes the same values.
        let defaults = vec![
            (
                "acceleration".to_owned(),
                crate::reflect::ReflectValue::F32(20.0),
            ),
            (
                "max_speed".to_owned(),
                crate::reflect::ReflectValue::F32(8.0),
            ),
        ];
        let schema = ComponentSchema {
            type_name: "my_game::GroundMovement".into(),
            fields: vec![
                FieldSchema::new("acceleration", PluginFieldKind::F32),
                FieldSchema::new("max_speed", PluginFieldKind::F32),
            ],
            defaults: ron::to_string(&defaults).expect("serialise defaults"),
        };

        // What the editor makes of it.
        let ty = to_dynamic_type(&schema, "my_game");
        assert_eq!(
            ty.defaults, defaults,
            "the values crossed the boundary as kinds without values"
        );
    }

    /// A plugin built before `defaults` existed sends an empty string,
    /// and a corrupt payload is not worth refusing a whole type over.
    /// Either way the component still registers — it just starts empty,
    /// which is exactly the old behaviour.
    #[test]
    fn a_plugin_without_usable_defaults_still_registers() {
        for payload in ["", "not ron at all", "[(1, 2)]"] {
            let schema = ComponentSchema {
                type_name: "my_game::Old".into(),
                fields: vec![FieldSchema::new("value", PluginFieldKind::F32)],
                defaults: payload.into(),
            };
            let ty = to_dynamic_type(&schema, "my_game");
            assert_eq!(ty.fields.len(), 1, "payload {payload:?} lost the fields");
            assert!(ty.defaults.is_empty(), "payload {payload:?} decoded");
        }
    }
}
