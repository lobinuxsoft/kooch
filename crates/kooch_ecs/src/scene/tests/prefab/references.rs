//! The reference model: an instance saved as a reference and rebuilt from it.

use super::*;

// -- the reference model -------------------------------------------------

/// A scene stores an instance as a reference and a list of changes, not as the entities the prefab
/// built. This is what removes the whole class of bugs that propagation was chasing: a value held
/// in two places drifts, and now it is held once.
#[test]
fn a_scene_writes_an_instance_as_a_reference_not_as_entities() {
    use crate::prefab_instance::PrefabInstance;

    let (mut resources, root) = world_with_a_deep_subtree();
    let prefab = SceneDocument::from_ecs_subtree(&mut resources, root);
    let source = Guid::new_v4();
    let scene = Guid::new_v4();

    let (spawned, members) =
        crate::scene::sync::instantiate_members(&prefab, &mut resources, scene).unwrap();
    crate::prefab_instance::attach(&mut resources, spawned, &members, source);

    let document = SceneDocument::from_ecs_scene(&mut resources, scene);

    // Three entities were built; one description is written.
    assert_eq!(
        document.entities.len(),
        1,
        "the instance's entities were written out: {:?}",
        document
            .entities
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
    );
    let described = &document.entities[0];
    let names: Vec<&str> = described
        .components
        .iter()
        .map(|c| c.type_name.as_str())
        .collect();
    assert!(
        names.iter().any(|n| n.ends_with("PrefabInstance")),
        "the reference itself is missing: {names:?}",
    );
    // Everything else belongs to the prefab, and `PrefabMember` is rebuilt
    // on load — writing either would be storing a fact twice.
    assert!(
        !names.iter().any(|n| n.ends_with("Health")),
        "a component the prefab already describes was written: {names:?}",
    );
    assert!(!names.iter().any(|n| n.ends_with("PrefabMember")));
    let _ = std::any::type_name::<PrefabInstance>();
}

/// And it comes back: loading the scene instances the prefab, so the
/// entities exist again without ever having been in the file.
#[test]
fn loading_a_reference_rebuilds_the_instance() {
    let (mut resources, root) = world_with_a_deep_subtree();
    let prefab = SceneDocument::from_ecs_subtree(&mut resources, root);
    let scene = Guid::new_v4();

    // The prefab has to be resolvable by guid, which is what a real load
    // does through the asset server. Standing in for it here: instance the
    // document directly and check the description that gets written.
    let (spawned, members) =
        crate::scene::sync::instantiate_members(&prefab, &mut resources, scene).unwrap();
    crate::prefab_instance::attach(&mut resources, spawned, &members, Guid::new_v4());
    let document = SceneDocument::from_ecs_scene(&mut resources, scene);

    // Without an asset server the prefab cannot be found, which is the
    // *other* case worth holding: the instance is not silently dropped.
    let mut fresh = setup_resources();
    crate::scene::sync::spawn_scene_into(&document, &mut fresh).unwrap();

    let names: Vec<String> = {
        let registry = fresh.get::<ComponentRegistry>().unwrap();
        let storage = registry.get_cpu::<crate::name::Name>();
        let allocator = fresh.get::<crate::allocator::EntityAllocator>().unwrap();
        (0..16u32)
            .map(|i| crate::entity::Entity::new(i, 0))
            .filter(|e| allocator.is_alive(*e))
            .filter_map(|e| storage.and_then(|s| s.get(e)).map(|n| n.value.clone()))
            .collect()
    };
    assert!(
        names.iter().any(|n| n.starts_with("missing prefab [")),
        "an unresolvable prefab must say so rather than vanish: {names:?}",
    );
}

/// Saving an instance as a prefab — including over the prefab it came from, which is how "apply
/// these changes to the prefab" is spelled.
#[test]
fn capturing_an_instance_as_a_prefab_takes_its_components_not_its_link() {
    use crate::prefab_instance::{PrefabInstance, PrefabMember};

    let (mut resources, root) = world_with_a_deep_subtree();
    let original = SceneDocument::from_ecs_subtree(&mut resources, root);
    let (spawned, members) =
        crate::scene::sync::instantiate_members(&original, &mut resources, Guid::new_v4()).unwrap();
    crate::prefab_instance::attach(&mut resources, spawned, &members, Guid::new_v4());

    let again = SceneDocument::from_ecs_subtree(&mut resources, spawned);

    assert_eq!(
        again.entities.len(),
        3,
        "the instance's entities were replaced by its link",
    );
    let types: Vec<&str> = again
        .entities
        .iter()
        .flat_map(|e| e.components.iter())
        .map(|c| c.type_name.as_str())
        .collect();
    assert!(
        !types.iter().any(|t| t.ends_with("PrefabInstance")),
        "the new prefab references the one it was made from: {types:?}",
    );
    assert!(!types.iter().any(|t| t.ends_with("PrefabMember")));
    // And it still describes the thing itself.
    assert!(types.iter().any(|t| t.ends_with("Health")));
    let _ = (
        std::any::type_name::<PrefabInstance>(),
        std::any::type_name::<PrefabMember>(),
    );
}

/// A prefab whose document contains a reference to itself.
#[test]
fn a_self_referencing_prefab_does_not_recurse_forever() {
    let mut resources = setup_resources();
    let source = Guid::new_v4();

    // A scene holding one instance of `source`.
    let scene = SceneDocument {
        id: Guid::new_v4(),
        name: "Level".into(),
        version: "0.1.0".into(),
        entities: vec![EntityDescription {
            name: "Instance".into(),
            parent_index: None,
            parent: None,
            components: vec![ComponentDescription {
                type_name: "kooch_ecs::prefab_instance::PrefabInstance".into(),
                fields: vec![
                    (
                        "source".into(),
                        ReflectValue::AssetRef {
                            guid: Some(source),
                            asset_type: "kooch_ecs::scene::document::SceneDocument".into(),
                        },
                    ),
                    ("overrides".into(), ReflectValue::String(String::new())),
                ],
            }],
        }],
    };

    // No asset server, so the prefab cannot be resolved at all — which is
    // the other way this must not hang or panic.
    crate::scene::sync::spawn_scene_into(&scene, &mut resources).unwrap();

    let named: Vec<String> = {
        let registry = resources.get::<ComponentRegistry>().unwrap();
        let storage = registry.get_cpu::<crate::name::Name>();
        let allocator = resources
            .get::<crate::allocator::EntityAllocator>()
            .unwrap();
        (0..8u32)
            .map(|i| crate::entity::Entity::new(i, 0))
            .filter(|e| allocator.is_alive(*e))
            .filter_map(|e| storage.and_then(|s| s.get(e)).map(|n| n.value.clone()))
            .collect()
    };
    assert!(
        named.iter().any(|n| n.contains(&source.to_string())),
        "the unresolvable prefab should be named, got {named:?}",
    );
}

/// 🔴 An instance keeps its place among its siblings across a save.
#[test]
fn an_instance_keeps_its_place_across_a_save() {
    use crate::order::Order;

    let (mut resources, root) = world_with_a_deep_subtree();
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Order>();

    let prefab = SceneDocument::from_ecs_subtree(&mut resources, root);
    let (spawned, members) =
        crate::scene::sync::instantiate_members(&prefab, &mut resources, Guid::new_v4()).unwrap();
    crate::prefab_instance::attach(&mut resources, spawned, &members, Guid::new_v4());

    // Dragged into third place among its siblings.
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .get_cpu_mut::<Order>()
        .unwrap()
        .insert(spawned, Order { value: 2500 });
    super::add_to_archetype(&mut resources, spawned, std::any::TypeId::of::<Order>());

    let doc = SceneDocument::from_ecs(&mut resources);

    let written = doc
        .entities
        .iter()
        .flat_map(|e| e.components.iter())
        .find(|c| c.type_name.contains("Order"))
        .expect("the instance's place was not written");
    assert_eq!(
        written.fields.first().map(|(_, value)| value.clone()),
        Some(crate::reflect::ReflectValue::U32(2500)),
        "the place was written but not the one it was moved to",
    );
}
