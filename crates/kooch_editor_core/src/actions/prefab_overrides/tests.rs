use super::*;
use kooch_ecs::transform::Transform;

/// A world with one two-entity instance already linked.
fn world_with_an_instance() -> (Resources, Entity, Entity) {
    let mut resources = Resources::new();
    resources.insert(kooch_ecs::allocator::EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(kooch_ecs::archetype_registry::ArchetypeRegistry::new());
    resources.insert(kooch_ecs::query::AccessTracker::new());
    resources.insert(kooch_ecs::commands::Commands::new());

    let mut names = ComponentNames::new();
    names.intern(std::any::type_name::<PrefabInstance>());
    names.intern(std::any::type_name::<Transform>());
    resources.insert(names);

    let root = Entity::new(0, 0);
    let child = Entity::new(1, 0);
    kooch_ecs::prefab_instance::attach(
        &mut resources,
        root,
        &[root, child],
        kooch_core::Guid::new_v4(),
    );
    (resources, root, child)
}

fn component(resources: &Resources, name: &str) -> kooch_ecs::component::ComponentId {
    resources.get::<ComponentNames>().unwrap().id(name).unwrap()
}

fn overrides_written(actions: &[EditorAction]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|a| match a {
            EditorAction::SetField {
                value: ReflectValue::String(s),
                ..
            } => Some(s.clone()),
            _ => None,
        })
        .collect()
}

/// Most edits in a scene are not on a prefab instance, and paying
/// anything for them would tax the common case.
#[test]
fn an_edit_outside_any_instance_records_nothing() {
    let (resources, _, _) = world_with_an_instance();
    let edit = EditorAction::SetField {
        entity: Entity::new(99, 0),
        component: component(&resources, std::any::type_name::<Transform>()),
        field: "position".to_owned(),
        value: ReflectValue::Vec3(glam::Vec3::ONE),
    };
    assert!(record(&resources, &[&edit]).is_empty());
}

/// An edit on a *child* of an instance is recorded against the root,
/// which is where the set lives — and addressed by the child's index
/// in the prefab, not by its handle.
#[test]
fn an_edit_on_a_member_is_recorded_against_its_instance() {
    let (resources, root, child) = world_with_an_instance();
    let edit = EditorAction::SetField {
        entity: child,
        component: component(&resources, std::any::type_name::<Transform>()),
        field: "position".to_owned(),
        value: ReflectValue::Vec3(glam::Vec3::ONE),
    };

    let out = record(&resources, &[&edit]);
    assert_eq!(out.len(), 1, "one write, on the instance root");
    match &out[0] {
        EditorAction::SetField { entity, field, .. } => {
            assert_eq!(*entity, root);
            assert_eq!(field, "overrides");
        }
        _ => panic!("expected a SetField on the instance root"),
    }
    assert_eq!(overrides_written(&out).len(), 1);
}

/// A translate must not claim the rotation was overridden. Marking a
/// field pins it to its current value, so an over-eager mark quietly
/// stops the prefab from ever reaching that field again.
#[test]
fn a_transform_drag_records_only_what_moved() {
    let (resources, _, child) = world_with_an_instance();
    let before = Transform::default();
    let after = Transform {
        position: glam::Vec3::X,
        ..Transform::default()
    };
    let edit = EditorAction::TransformEdit {
        entity: child,
        before,
        after,
        desc: "Move",
    };

    let written = overrides_written(&record(&resources, &[&edit]));
    assert_eq!(written.len(), 1);
    assert!(written[0].contains("position"));
    assert!(
        !written[0].contains("rotation") && !written[0].contains("scale"),
        "an untouched field was marked: {}",
        written[0],
    );
}

/// The component that stores overrides must not record its own write,
/// or persisting a mark would mark persisting it.
#[test]
fn writing_the_override_set_is_not_itself_an_override() {
    let (resources, root, _) = world_with_an_instance();
    let edit = EditorAction::SetField {
        entity: root,
        component: component(&resources, std::any::type_name::<PrefabInstance>()),
        field: "overrides".to_owned(),
        value: ReflectValue::String("0\u{1f}T\u{1f}x".to_owned()),
    };
    assert!(record(&resources, &[&edit]).is_empty());
}
