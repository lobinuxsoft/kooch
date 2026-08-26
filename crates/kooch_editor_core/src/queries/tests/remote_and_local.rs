//! The editor's read-only view of the ECS, mirrored and local.

use std::collections::HashSet;

use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::commands::Commands;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::reflect::ReflectValue;
use kooch_ecs::transform::Transform;
use kooch_remote::protocol::{ComponentSnapshot, EntityId, EntitySnapshot};

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
        scene: None,
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
    assert_eq!(gather_entity_data(&resources, &HashSet::new()).len(), 1);
}

/// A component with no local Rust type still reaches the Inspector,
/// with its field values, read-only while no session can apply an edit.
#[test]
fn parked_components_surface_read_only_without_a_session() {
    let mut resources = mirrored_world();
    intern_registry_names(&mut resources);

    // Selected: this is the Inspector's view, and the values are what
    // the test is about.
    let all: HashSet<_> = gather_entity_data(&resources, &HashSet::new())
        .iter()
        .map(|e| e.entity)
        .collect();
    let entities = gather_entity_data(&resources, &all);
    let spin = entities[0]
        .components
        .iter()
        .find(|c| c.short_name == "Spin")
        .expect("parked component displayed");

    assert_eq!(spin.visibility, InspectorVisibility::ReadOnly);
    assert_ne!(spin.component, ComponentId::INVALID);
    assert_eq!(
        spin.fields.values().map(Vec::as_slice),
        Some(&[("rpm".to_owned(), ReflectValue::F32(33.0))][..])
    );
}

/// A `RemoteState` reporting a connected session whose schema is
/// `components`.
fn connected_with_schema(port: u16, components: &[(&str, Option<&str>)]) -> RemoteState {
    use kooch_remote::protocol::ComponentSchema;

    let mut state = RemoteState::new();
    let mut session = crate::remote_session::RemoteSession::attach("kooch_test_schema_only.sock");
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
            ("kooch_ecs::transform::Transform", None),
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
        kooch_ecs::component::ComponentId::INVALID,
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
    state.session = Some(crate::remote_session::RemoteSession::attach(
        "kooch_test_never_connects.sock",
    ));
    resources.insert(state);

    intern_registry_names(&mut resources);
    let types = gather_reflected_types(&resources);
    assert!(
        types.iter().any(|t| t.short_name == "Transform"),
        "a half-connected session emptied the menu"
    );
}
