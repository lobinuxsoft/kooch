//! Pushing a changed prefab out to the instances of it.

use super::SceneDocument;
use crate::entity::Entity;
use crate::prefab_instance::{OverrideAddress, PrefabInstance, PrefabMember};
use crate::query::Query;
use crate::reflect::ReflectValue;
use kooch_core::Guid;
use kooch_core::resource::Resources;

/// A component to take off an instance because the prefab dropped it.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedRemoval {
    pub entity: Entity,
    pub component: String,
}

/// One field to write into one live entity.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedWrite {
    pub entity: Entity,
    /// Full type path, as the prefab document stores it.
    pub component: String,
    pub field: String,
    pub value: ReflectValue,
    /// Whether the entity has to grow the component before the field can be written.
    pub add_component: bool,
}

/// Works out everything that should change when `prefab` is saved.
pub fn plan(resources: &Resources, prefab: Guid) -> (Vec<PlannedWrite>, Vec<PlannedRemoval>) {
    let Some(document) = cached_document(resources, prefab) else {
        // Every step below is silent on failure, and a silent propagation is indistinguishable from
        // one that decided there was nothing to do. Said out loud so the next report is about a
        // stage rather than about "it does not work".
        tracing::warn!(target: "kooch_ecs::prefab", %prefab, "no cached document; nothing to propagate");
        return (Vec::new(), Vec::new());
    };

    let instances = instances_of(resources, prefab);
    tracing::info!(
        target: "kooch_ecs::prefab",
        %prefab,
        instances = instances.len(),
        "propagating",
    );

    let mut writes = Vec::new();
    let mut removals = Vec::new();
    for (root, instance) in instances {
        for (entity, index) in members_of(resources, root) {
            let Some(described) = document.entities.get(index) else {
                // The prefab lost an entity this instance still has. Its fields simply stop being
                // propagated; removing it is a structural change and those are deliberately out of
                // scope.
                continue;
            };
            for component in &described.components {
                // A component the prefab grew since this instance was placed. The instance has to
                // grow it too, or the change reaches every instance except as the one thing people
                // most often change about a prefab.
                if instance.owns_component(index, &component.type_name) {
                    continue;
                }
                let missing = !has_component(resources, entity, &component.type_name);
                let mut first = true;
                for (field, value) in &component.fields {
                    let address = OverrideAddress {
                        entity: index,
                        component: component.type_name.clone(),
                        field: field.clone(),
                    };
                    // The one thing this must never do.
                    if instance.is_overridden(&address) {
                        continue;
                    }
                    writes.push(PlannedWrite {
                        entity,
                        component: component.type_name.clone(),
                        field: field.clone(),
                        value: value.clone(),
                        // Asked for once, on the first write that needs
                        // it: adding is idempotent but a round trip is not
                        // free, and in remote mode each one is a call.
                        add_component: missing && std::mem::take(&mut first),
                    });
                }
                // A component with no fields still has to arrive.
                if missing && first {
                    writes.push(PlannedWrite {
                        entity,
                        component: component.type_name.clone(),
                        field: String::new(),
                        value: ReflectValue::Bool(false),
                        add_component: true,
                    });
                }
            }
            // Whatever the instance still carries that the prefab no
            // longer describes — minus anything the user put there.
            for component in live_components(resources, entity) {
                let dropped = !described
                    .components
                    .iter()
                    .any(|c| c.type_name == component);
                if dropped
                    && !instance.owns_component(index, &component)
                    && !is_bookkeeping(&component)
                {
                    removals.push(PlannedRemoval { entity, component });
                }
            }
        }
    }
    tracing::info!(
        target: "kooch_ecs::prefab",
        writes = writes.len(),
        adds = writes.iter().filter(|w| w.add_component).count(),
        removals = removals.len(),
        "propagation planned",
    );
    (writes, removals)
}

/// Components that hold the instance together rather than describing it.
fn is_bookkeeping(type_name: &str) -> bool {
    matches!(
        type_name.rsplit("::").next().unwrap_or(type_name),
        "PrefabInstance"
            | "PrefabMember"
            | "Parent"
            | "Children"
            | "GlobalTransform"
            | "SceneMember"
            // Where an instance sits among its siblings is the scene's business, not the prefab's —
            // a prefab has no siblings. Propagating over it would reshuffle every instance of a
            // prefab the moment somebody edited it (#961).
            | "Order"
    )
}

/// Every component the live entity currently carries, by full type path.
fn live_components(resources: &Resources, entity: Entity) -> Vec<String> {
    let Some(archetypes) = resources.get::<crate::archetype_registry::ArchetypeRegistry>() else {
        return Vec::new();
    };
    let Some(registry) = resources.get::<crate::component::ComponentRegistry>() else {
        return Vec::new();
    };
    let Some(archetype) = archetypes.entity_archetype(entity) else {
        return Vec::new();
    };
    archetypes
        .get(archetype)
        .map(|archetype| {
            archetype
                .components()
                .iter()
                .filter_map(|type_id| registry.component_name(type_id).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Brings every instance in the world up to date with its prefab.
pub fn refresh_all(resources: &mut Resources) {
    let mut sources: Vec<Guid> = Vec::new();
    {
        let query = Query::<&PrefabInstance>::new(resources);
        query.for_each(|instance| {
            if let Some(source) = instance.source
                && !sources.contains(&source)
            {
                sources.push(source);
            }
        });
    }
    if sources.is_empty() {
        return;
    }
    tracing::info!(
        target: "kooch_ecs::prefab",
        prefabs = sources.len(),
        "refreshing instances against their prefabs",
    );
    for source in sources {
        let (writes, removals) = plan(resources, source);
        apply(resources, &writes, &removals);
    }
}

/// Applies a plan to the editor's own world.
pub fn apply(resources: &mut Resources, writes: &[PlannedWrite], removals: &[PlannedRemoval]) {
    // Removals first: a component the prefab dropped and re-added under
    // another name would otherwise be added and then taken straight off.
    for removal in removals {
        let type_id = resources
            .get::<crate::component::ComponentRegistry>()
            .and_then(|registry| registry.type_id_by_name(&removal.component));
        if let Some(type_id) = type_id
            && let Some(registry) = resources.get_mut::<crate::component::ComponentRegistry>()
        {
            registry.remove_component(removal.entity, &type_id);
        }
    }
    for write in writes {
        let type_id = resources
            .get::<crate::component::ComponentRegistry>()
            .and_then(|registry| registry.type_id_by_name(&write.component));
        let Some(type_id) = type_id else {
            continue;
        };
        if let Some(registry) = resources.get_mut::<crate::component::ComponentRegistry>()
            && let Err(e) = registry.reflect_set_field(
                &type_id,
                write.entity,
                &write.field,
                write.value.clone(),
            )
        {
            tracing::debug!(
                "prefab propagation skipped {}.{}: {e}",
                write.component,
                write.field,
            );
        }
    }
}

/// Whether `entity` already carries the component named `type_name`.
fn has_component(resources: &Resources, entity: Entity, type_name: &str) -> bool {
    let Some(registry) = resources.get::<crate::component::ComponentRegistry>() else {
        return false;
    };
    let Some(type_id) = registry.type_id_by_name(type_name) else {
        // Unknown to this binary. Treated as present so propagation does
        // not try to add something it cannot construct.
        return true;
    };
    registry.reflect_get_fields(&type_id, entity).is_some()
}

/// The prefab's document as the editor currently holds it.
fn cached_document(resources: &Resources, prefab: Guid) -> Option<SceneDocument> {
    // Guid to path to handle: the server caches by path, and the database
    // is what maps identity to one.
    let path = resources
        .get::<kooch_core::asset_database::AssetDatabase>()?
        .entry(prefab)?
        .path
        .clone();
    let handle = resources
        .get::<kooch_core::asset_loader::AssetServer>()?
        .get_cached::<SceneDocument>(&path)?;
    resources
        .get::<kooch_core::assets::Assets<SceneDocument>>()?
        .get(handle)
        .cloned()
}

/// Every instance of `prefab` in the world.
fn instances_of(resources: &Resources, prefab: Guid) -> Vec<(Entity, PrefabInstance)> {
    let query = Query::<&PrefabInstance>::new(resources);
    let mut found = Vec::new();
    query.for_each_entity(|entity, instance| {
        if instance.source == Some(prefab) {
            found.push((entity, instance.clone()));
        }
    });
    found
}

/// The entities belonging to one instance, with the prefab entity each
/// one stands for.
fn members_of(resources: &Resources, root: Entity) -> Vec<(Entity, usize)> {
    let query = Query::<&PrefabMember>::new(resources);
    let mut found = Vec::new();
    query.for_each_entity(|entity, member| {
        if member.root == root {
            found.push((entity, member.index as usize));
        }
    });
    found
}

/// Stores an instance's override set locally.
pub fn write_overrides(resources: &mut Resources, root: Entity, overrides: &str) {
    let type_id = resources
        .get::<crate::component::ComponentRegistry>()
        .and_then(|registry| registry.type_id_by_name(std::any::type_name::<PrefabInstance>()));
    let Some(type_id) = type_id else {
        return;
    };
    if let Some(registry) = resources.get_mut::<crate::component::ComponentRegistry>() {
        let _ = registry.reflect_set_field(
            &type_id,
            root,
            "overrides",
            ReflectValue::String(overrides.to_owned()),
        );
    }
}

// ---------------------------------------------------------------------------
// Revert
// ---------------------------------------------------------------------------

/// Drops overrides on the instance `entity` belongs to, and plans the writes that put the prefab's
/// values back.
pub fn plan_revert(
    resources: &Resources,
    entity: Entity,
    component: Option<crate::component::ComponentId>,
) -> Option<(Entity, String, Vec<PlannedWrite>)> {
    // The panel speaks `ComponentId`; an override address is a type name,
    // because it outlives the process that recorded it.
    let component = match component {
        Some(id) => Some(
            resources
                .get::<crate::component::ComponentNames>()?
                .name(id)?
                .to_owned(),
        ),
        None => None,
    };
    let component = component.as_deref();
    let member = resources
        .get::<crate::component::ComponentRegistry>()?
        .get_cpu::<PrefabMember>()?
        .get(entity)
        .cloned()?;
    let root = member.root;
    let mut instance = resources
        .get::<crate::component::ComponentRegistry>()?
        .get_cpu::<PrefabInstance>()?
        .get(root)
        .cloned()?;

    // Narrowed by *this* entity as well as the component: reverting a
    // child's Transform must not revert the root's.
    let kept: Vec<crate::prefab_instance::Override> = instance
        .overrides()
        .into_iter()
        .filter(|o| match component {
            Some(component) => {
                !(o.address.entity == member.index as usize && o.address.component == component)
            }
            None => false,
        })
        .collect();
    instance.set_overrides(kept);

    // Planned against the instance as it will be, so the fields just
    // released are the ones that come back.
    let writes = plan_for(resources, root, &instance)?;
    Some((root, instance.overrides, writes))
}

/// The writes a single instance needs, given the override set to respect.
fn plan_for(
    resources: &Resources,
    root: Entity,
    instance: &PrefabInstance,
) -> Option<Vec<PlannedWrite>> {
    let document = cached_document(resources, instance.source?)?;
    let mut writes = Vec::new();
    for (entity, index) in members_of(resources, root) {
        let Some(described) = document.entities.get(index) else {
            continue;
        };
        for component in &described.components {
            // Reverting restores the prefab's fields, not the user's
            // decision to drop a component — that is its own override and
            // is released by the same revert if it was asked for.
            if instance.owns_component(index, &component.type_name) {
                continue;
            }
            for (field, value) in &component.fields {
                let address = OverrideAddress {
                    entity: index,
                    component: component.type_name.clone(),
                    field: field.clone(),
                };
                if instance.is_overridden(&address) {
                    continue;
                }
                writes.push(PlannedWrite {
                    entity,
                    component: component.type_name.clone(),
                    field: field.clone(),
                    value: value.clone(),
                    add_component: !has_component(resources, entity, &component.type_name),
                });
            }
        }
    }
    Some(writes)
}

#[cfg(test)]
mod tests;
