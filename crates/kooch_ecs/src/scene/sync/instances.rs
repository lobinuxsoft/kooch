//! Prefab instances inside a scene: their source, overrides, placeholders, and the children and archetypes they rebuild.

use super::*;

/// The prefab a description references, if it is an instance.
pub(super) fn instance_source(
    entity_desc: &super::document::EntityDescription,
) -> Option<kooch_core::Guid> {
    let instance = entity_desc
        .components
        .iter()
        .find(|c| c.type_name.ends_with("PrefabInstance"))?;
    instance
        .fields
        .iter()
        .find_map(|(name, value)| match value {
            crate::reflect::ReflectValue::AssetRef { guid, .. } if name == "source" => *guid,
            _ => None,
        })
}

/// The override list a description carries, still encoded.
pub(super) fn instance_overrides(entity_desc: &super::document::EntityDescription) -> String {
    entity_desc
        .components
        .iter()
        .find(|c| c.type_name.ends_with("PrefabInstance"))
        .and_then(|c| {
            c.fields.iter().find_map(|(name, value)| match value {
                crate::reflect::ReflectValue::String(s) if name == "overrides" => Some(s.clone()),
                _ => None,
            })
        })
        .unwrap_or_default()
}

/// Instances `source` and applies the description's overrides, returning the instance root.
pub(super) fn rebuild_instance(
    entity_desc: &super::document::EntityDescription,
    source: kooch_core::Guid,
    resources: &mut Resources,
    into: kooch_core::Guid,
) -> crate::entity::Entity {
    // Depth-first, so the chain is exactly the prefabs above this one.
    if resources.get::<InstancingChain>().is_none() {
        resources.insert(InstancingChain::default());
    }
    let cyclic = resources
        .get::<InstancingChain>()
        .is_some_and(|chain| chain.0.contains(&source));
    if cyclic {
        tracing::error!(
            target: "kooch_ecs::scene",
            %source,
            "prefab references itself; instancing it would not terminate",
        );
        return spawn_placeholder(resources, format!("cyclic prefab [{source}]"));
    }
    if let Some(chain) = resources.get_mut::<InstancingChain>() {
        chain.0.push(source);
    }

    // Named, not guessed. `spawn_members` reads the active scene out of `SceneManager` — which the
    // load lifted out of `Resources` to run, so it would answer with a fresh random `Guid` and put
    // this instance's members in a scene nobody has open (#955).
    let built = match super::prefab::spawn_members_into(source, resources, into) {
        Ok((root, members)) => {
            crate::prefab_instance::attach(resources, root, &members, source);
            apply_overrides(&instance_overrides(entity_desc), &members, resources);
            root
        }
        Err(e) => {
            tracing::error!(
                target: "kooch_ecs::scene",
                %source,
                "prefab could not be instanced: {e}",
            );
            spawn_placeholder(resources, format!("missing prefab [{source}]"))
        }
    };

    // Popped whichever way it went, or one failure would poison every
    // later instancing of the same prefab in this load.
    if let Some(chain) = resources.get_mut::<InstancingChain>() {
        chain.0.pop();
    }
    built
}

/// An entity that says out loud what went wrong, instead of vanishing.
pub(super) fn spawn_placeholder(resources: &mut Resources, name: String) -> crate::entity::Entity {
    let entity = {
        let mut commands = resources
            .remove::<Commands>()
            .expect("Commands not found in Resources");
        let entity = commands.spawn(resources).id();
        resources.insert(commands);
        entity
    };
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<crate::name::Name>();
        if let Some(storage) = registry.get_cpu_mut::<crate::name::Name>() {
            storage.insert(entity, crate::name::Name { value: name });
        }
    }
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(entity)
    {
        let next = archetypes
            .archetype_after_add_dynamic(current, std::any::TypeId::of::<crate::name::Name>());
        archetypes.register_entity(entity, next);
    }
    entity
}

/// Writes a saved override list onto a freshly built instance.
pub(super) fn apply_overrides(
    encoded: &str,
    members: &[crate::entity::Entity],
    resources: &mut Resources,
) {
    let mut instance = crate::prefab_instance::PrefabInstance::default();
    instance.overrides = encoded.to_owned();

    for entry in instance.overrides() {
        let Some(&entity) = members.get(entry.address.entity) else {
            // The prefab lost the entity this override addressed. The
            // override is dropped rather than guessed at.
            continue;
        };
        let type_id = resources
            .get::<ComponentRegistry>()
            .and_then(|registry| registry.type_id_by_name(&entry.address.component));
        let Some(type_id) = type_id else {
            continue;
        };
        // What a record *means* is decided by its field, not by whether a value came with it.
        let is_removal = entry.address.field == crate::prefab_instance::WHOLE_COMPONENT;
        match (is_removal, entry.value) {
            // A component the user took off this instance.
            (true, _) => {
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    registry.remove_component(entity, &type_id);
                }
            }
            // A field the user changed.
            (false, Some(value)) => {
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    // The component may be one the user *added* to this
                    // instance, in which case the prefab did not build it.
                    if registry.reflect_get_fields(&type_id, entity).is_none() {
                        registry.insert_default_reflected(&type_id, entity);
                        add_to_archetype(resources, entity, type_id);
                    }
                }
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    let _ =
                        registry.reflect_set_field(&type_id, entity, &entry.address.field, value);
                }
            }
            // A field override with nothing to write. Skipped rather than
            // guessed at: the prefab's own value is the honest fallback,
            // and it is already there.
            (false, None) => tracing::debug!(
                target: "kooch_ecs::scene",
                component = %entry.address.component,
                field = %entry.address.field,
                "override carries no value; leaving the prefab's",
            ),
        }
    }
}

/// Tells the archetype about a component just inserted.
pub(super) fn add_to_archetype(
    resources: &mut Resources,
    entity: crate::entity::Entity,
    type_id: std::any::TypeId,
) {
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(entity)
    {
        let next = archetypes.archetype_after_add_dynamic(current, type_id);
        archetypes.register_entity(entity, next);
    }
}

/// Fills in `Children` for a freshly spawned set from their `Parent`.
pub(super) fn rebuild_children(spawned: &[crate::entity::Entity], resources: &mut Resources) {
    use crate::hierarchy::{Children, Parent};

    let mut links: Vec<(crate::entity::Entity, crate::entity::Entity)> = Vec::new();
    if let Some(registry) = resources.get::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu::<Parent>()
    {
        for &entity in spawned {
            if let Some(parent) = storage.get(entity) {
                links.push((parent.entity, entity));
            }
        }
    }
    if links.is_empty() {
        return;
    }

    for (parent, child) in links {
        let existing = resources
            .get::<ComponentRegistry>()
            .and_then(|registry| registry.get_cpu::<Children>())
            .and_then(|storage| storage.get(parent))
            .map(|children| children.entities.clone())
            .unwrap_or_default();
        if existing.contains(&child) {
            continue;
        }
        let mut entities = existing;
        entities.push(child);
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            registry.register_cpu_reflected::<Children>();
            if let Some(storage) = registry.get_cpu_mut::<Children>() {
                storage.insert(parent, Children { entities });
            }
        }
        add_to_archetype(resources, parent, std::any::TypeId::of::<Children>());
    }
}

/// Records which scene an entity was authored in.
pub(super) fn tag_with_scene(
    resources: &mut Resources,
    entity: crate::entity::Entity,
    scene: kooch_core::Guid,
) {
    use crate::scene_member::SceneMember;

    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<SceneMember>();
        if let Some(storage) = registry.get_cpu_mut::<SceneMember>() {
            storage.insert(entity, SceneMember::new(scene));
        }
    }
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(entity)
    {
        let next =
            archetypes.archetype_after_add_dynamic(current, std::any::TypeId::of::<SceneMember>());
        archetypes.register_entity(entity, next);
    }
}
