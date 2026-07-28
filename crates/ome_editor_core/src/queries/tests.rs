//! Tests for [`super`] — the editor's read-only view of the ECS.

#[cfg(test)]
mod plugin_types_in_the_menu {
    use ome_core::resource::Resources;
    use ome_ecs::allocator::EntityAllocator;
    use ome_ecs::archetype_registry::ArchetypeRegistry;
    use ome_ecs::commands::Commands;
    use ome_ecs::component::{
        ComponentId, ComponentNames, ComponentRegistry, DynamicField, DynamicType,
        DynamicTypeRegistry,
    };
    use ome_ecs::dynamic_components::DynamicComponents;
    use ome_ecs::query::AccessTracker;
    use ome_ecs::reflect::FieldKind;
    use ome_ecs::transform::Transform;

    use super::super::{gather_reflected_types, intern_registry_names};

    /// A local editor world with one plugin-declared type registered.
    fn world_with_plugin_type() -> Resources {
        let mut r = Resources::new();
        r.insert(EntityAllocator::new());
        r.insert(ComponentRegistry::new());
        r.insert(ArchetypeRegistry::new());
        r.insert(AccessTracker::new());
        r.insert(Commands::new());
        r.insert(DynamicComponents::new());
        r.insert(ComponentNames::new());
        r.get_mut::<ComponentRegistry>()
            .unwrap()
            .register_cpu_reflected::<Transform>();

        let mut types = DynamicTypeRegistry::new();
        types
            .register(DynamicType {
                type_name: "my_game::Health".into(),
                fields: vec![DynamicField {
                    name: "current".into(),
                    kind: FieldKind::U32,
                }],
                source: "my_game".into(),
            })
            .unwrap();
        r.insert(types);
        r
    }

    /// The whole point: a component the editor binary never compiled
    /// shows up in the Add Component menu.
    #[test]
    fn a_plugin_type_appears_beside_the_engines_own() {
        let mut resources = world_with_plugin_type();
        intern_registry_names(&mut resources);

        let types = gather_reflected_types(&resources);
        let names: Vec<&str> = types.iter().map(|t| t.short_name.as_str()).collect();

        assert!(
            names.contains(&"Health"),
            "plugin type missing from {names:?}"
        );
        assert!(names.contains(&"Transform"), "engine types must remain");
    }

    /// Listing it is useless if it cannot be added: an un-interned name
    /// resolves to INVALID and every action on it is dropped.
    #[test]
    fn its_component_id_resolves() {
        let mut resources = world_with_plugin_type();
        intern_registry_names(&mut resources);

        let types = gather_reflected_types(&resources);
        let health = types
            .iter()
            .find(|t| t.short_name == "Health")
            .expect("Health listed");

        assert_ne!(
            health.component,
            ComponentId::INVALID,
            "the name was never interned, so adding it would be dropped"
        );
    }

    /// Grouped by the plugin that brought them, so project components do
    /// not scatter through the engine's list.
    #[test]
    fn it_is_categorised_by_its_source() {
        let mut resources = world_with_plugin_type();
        intern_registry_names(&mut resources);

        let types = gather_reflected_types(&resources);
        let health = types.iter().find(|t| t.short_name == "Health").unwrap();

        assert_eq!(health.category.as_deref(), Some("my_game"));
    }

    /// No plugins loaded must change nothing.
    #[test]
    fn without_the_registry_the_menu_is_unchanged() {
        let mut resources = world_with_plugin_type();
        resources.remove::<DynamicTypeRegistry>();
        intern_registry_names(&mut resources);

        let types = gather_reflected_types(&resources);
        assert!(types.iter().all(|t| t.short_name != "Health"));
        assert!(types.iter().any(|t| t.short_name == "Transform"));
    }
}

#[cfg(test)]
mod remote_and_local {
    use ome_ecs::allocator::EntityAllocator;
    use ome_ecs::commands::Commands;
    use ome_ecs::query::AccessTracker;
    use ome_ecs::reflect::ReflectValue;
    use ome_ecs::transform::Transform;
    use ome_remote::protocol::{ComponentSnapshot, EntityId, EntitySnapshot};

    use crate::remote_mirror::{MirrorEntity, RemoteMirror};
    use crate::remote_session::RemoteState;

    use super::super::*;

    /// A mirrored world: ECS resources plus the ephemeral registration
    /// the editor plugin performs at startup.
    fn mirrored_world() -> Resources {
        let mut r = Resources::new();
        r.insert(EntityAllocator::new());
        r.insert(ComponentRegistry::new());
        r.insert(ArchetypeRegistry::new());
        r.insert(AccessTracker::new());
        r.insert(Commands::new());
        r.insert(DynamicComponents::new());
        r.insert(ComponentNames::new());
        r.get_mut::<ComponentRegistry>()
            .unwrap()
            .register_cpu_reflected::<Transform>();

        let mut ephemeral = EphemeralComponents::new();
        ephemeral.insert(std::any::TypeId::of::<MirrorEntity>());
        r.insert(ephemeral);

        let snapshot = vec![EntitySnapshot {
            id: EntityId {
                index: 0,
                generation: 0,
            },
            name: Some("Mesh".into()),
            parent: None,
            components: vec![
                ComponentSnapshot {
                    type_name: std::any::type_name::<Transform>().into(),
                    fields: vec![],
                },
                ComponentSnapshot {
                    type_name: "game::spin::Spin".into(),
                    fields: vec![("rpm".into(), ReflectValue::F32(33.0))],
                },
            ],
        }];
        RemoteMirror::new().apply(&snapshot, &mut r);
        r
    }

    /// Mirrored entities stay visible even though `MirrorEntity` is
    /// ephemeral — in remote mode they are the whole World panel.
    #[test]
    fn mirrored_entities_are_not_filtered_as_ephemeral() {
        let mut resources = mirrored_world();
        intern_registry_names(&mut resources);
        assert_eq!(gather_entity_data(&resources).len(), 1);
    }

    /// A component with no local Rust type still reaches the Inspector,
    /// with its field values, read-only while no session can apply an edit.
    #[test]
    fn parked_components_surface_read_only_without_a_session() {
        let mut resources = mirrored_world();
        intern_registry_names(&mut resources);

        let entities = gather_entity_data(&resources);
        let spin = entities[0]
            .components
            .iter()
            .find(|c| c.short_name == "Spin")
            .expect("parked component displayed");

        assert_eq!(spin.visibility, InspectorVisibility::ReadOnly);
        assert_ne!(spin.component, ComponentId::INVALID);
        assert_eq!(
            spin.fields.as_deref(),
            Some(&[("rpm".to_owned(), ReflectValue::F32(33.0))][..])
        );
    }

    /// A `RemoteState` reporting a connected session whose schema is
    /// `components`.
    fn connected_with_schema(port: u16, components: &[(&str, Option<&str>)]) -> RemoteState {
        use ome_remote::protocol::ComponentSchema;

        let mut state = RemoteState::new();
        let mut session = crate::remote_session::RemoteSession::attach(port);
        session.connected_with_schema_for_test(
            components
                .iter()
                .map(|(name, category)| ComponentSchema {
                    type_name: (*name).to_owned(),
                    fields: None,
                    category: category.map(str::to_owned),
                })
                .collect(),
        );
        state.session = Some(session);
        state
    }

    /// The point of the whole change: with a project connected, the menu
    /// lists the *project's* components — including ones this binary has no
    /// Rust type for. Asking the editor's own registry answers with whatever
    /// the editor was compiled with and omits everything the project defines.
    #[test]
    fn the_menu_lists_the_projects_components_not_the_editors() {
        let mut resources = mirrored_world();
        // The editor knows Transform; the project also has a component this
        // binary has never heard of.
        resources.insert(connected_with_schema(
            1,
            &[
                ("game::spin::Spin", Some("Gameplay")),
                ("ome_ecs::transform::Transform", None),
            ],
        ));

        intern_registry_names(&mut resources);
        let types = gather_reflected_types(&resources);

        let names: Vec<&str> = types.iter().map(|t| t.short_name.as_str()).collect();
        assert!(
            names.contains(&"Spin"),
            "a project-defined component is missing: {names:?}"
        );
        assert_eq!(names.len(), 2, "the local registry leaked in: {names:?}");

        // And it carries a usable identity, or AddComponent could not name it
        // over the wire.
        let spin = types.iter().find(|t| t.short_name == "Spin").unwrap();
        assert_ne!(
            spin.component,
            ome_ecs::component::ComponentId::INVALID,
            "the project's component was never interned"
        );
        assert_eq!(spin.category.as_deref(), Some("Gameplay"));
    }

    /// Local mode is unchanged: no session means the editor's own registry,
    /// which is the only thing that exists to ask.
    #[test]
    fn local_mode_still_lists_the_local_registry() {
        let mut resources = mirrored_world();
        intern_registry_names(&mut resources);
        let types = gather_reflected_types(&resources);

        assert!(
            types.iter().any(|t| t.short_name == "Transform"),
            "the local registry is no longer listed"
        );
    }

    /// A session that has not finished connecting must not blank the menu:
    /// its schema is empty until the handshake completes, and answering with
    /// nothing would look like a project with no components.
    #[test]
    fn a_connecting_session_falls_back_to_the_local_registry() {
        let mut resources = mirrored_world();
        // `attach` starts in Connecting, and `is_connected` is false there.
        let mut state = RemoteState::new();
        state.session = Some(crate::remote_session::RemoteSession::attach(1));
        resources.insert(state);

        intern_registry_names(&mut resources);
        let types = gather_reflected_types(&resources);
        assert!(
            types.iter().any(|t| t.short_name == "Transform"),
            "a half-connected session emptied the menu"
        );
    }
}

/// The full authoring loop for a component the editor never compiled:
/// it is listed, it can be added, it shows its fields, and they edit.
#[cfg(test)]
mod the_whole_cycle {
    use ome_core::resource::Resources;
    use ome_ecs::allocator::EntityAllocator;
    use ome_ecs::archetype_registry::ArchetypeRegistry;
    use ome_ecs::commands::Commands;
    use ome_ecs::component::{
        ComponentNames, ComponentRegistry, DynamicField, DynamicType, DynamicTypeRegistry,
    };
    use ome_ecs::dynamic_components::DynamicComponents;
    use ome_ecs::query::AccessTracker;
    use ome_ecs::reflect::{FieldKind, InspectorVisibility, ReflectValue};

    use crate::actions::EditorAction;

    use super::super::{gather_entity_data, gather_reflected_types, intern_registry_names};

    fn world() -> (Resources, ome_ecs::Entity) {
        let mut r = Resources::new();
        let mut alloc = EntityAllocator::new();
        let entity = alloc.spawn();
        r.insert(alloc);
        r.insert(ComponentRegistry::new());
        let mut archetypes = ArchetypeRegistry::new();
        let empty = archetypes.get_or_create(Default::default());
        archetypes.register_entity(entity, empty);
        r.insert(archetypes);
        r.insert(AccessTracker::new());
        r.insert(Commands::new());
        r.insert(DynamicComponents::new());
        r.insert(ComponentNames::new());
        r.insert(crate::undo::UndoStack::new());

        let mut types = DynamicTypeRegistry::new();
        types
            .register(DynamicType {
                type_name: "my_game::Health".into(),
                fields: vec![DynamicField {
                    name: "current".into(),
                    kind: FieldKind::U32,
                }],
                source: "my_game".into(),
            })
            .unwrap();
        r.insert(types);
        (r, entity)
    }

    #[test]
    fn list_add_inspect_edit_undo() {
        let (mut resources, entity) = world();
        intern_registry_names(&mut resources);

        // 1. It is offered by the menu, with a usable id.
        let listed = gather_reflected_types(&resources);
        let health = listed
            .iter()
            .find(|t| t.short_name == "Health")
            .expect("Health must be listed");

        // 2. Adding it goes through the ordinary action path.
        let mut undo = resources.remove::<crate::undo::UndoStack>().unwrap();
        crate::actions::apply_actions(
            &mut resources,
            &[EditorAction::AddComponent {
                entity,
                component: health.component,
            }],
            &mut undo,
        );
        resources.insert(undo);

        // 3. The Inspector sees it, with its field, editable.
        let shown = gather_entity_data(&resources);
        let row = shown
            .iter()
            .find(|e| e.entity == entity)
            .expect("entity present");
        let comp = row
            .components
            .iter()
            .find(|c| c.short_name == "Health")
            .expect("Health must be on the entity");
        assert_eq!(
            comp.visibility,
            InspectorVisibility::Editable,
            "a project's own component must be editable in the editor that authors it"
        );
        let fields = comp.fields.as_ref().expect("fields shown");
        assert_eq!(fields[0].0, "current");
        assert_eq!(fields[0].1, ReflectValue::U32(0));

        // 4. Editing it lands.
        let mut undo = resources.remove::<crate::undo::UndoStack>().unwrap();
        crate::actions::apply_actions(
            &mut resources,
            &[EditorAction::SetField {
                entity,
                component: health.component,
                field: "current".into(),
                value: ReflectValue::U32(80),
            }],
            &mut undo,
        );
        resources.insert(undo);
        let stored = resources
            .get::<DynamicComponents>()
            .unwrap()
            .iter_entity(entity)
            .find(|(n, _)| *n == "my_game::Health")
            .map(|(_, f)| f[0].1.clone())
            .unwrap();
        assert_eq!(stored, ReflectValue::U32(80), "the edit did not land");
    }
}
