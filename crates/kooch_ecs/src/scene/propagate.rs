//! Pushing a changed prefab out to the instances of it.
//!
//! Engine logic rather than editor logic, because the *project* is what
//! loads a scene and a scene has to catch up with its prefabs the moment
//! it opens. Leaving this in the editor would have meant waiting for the
//! mirror to arrive before knowing what to do.
//!
//! # The point of the whole feature
//!
//! With a dozen instances placed by hand, changing the prefab and
//! re-placing all twelve is work with no result — so nobody does it. They
//! edit the twelve by hand, and one ends up different. This is what makes
//! the change reach them.
//!
//! # Why these writes are not `SetField`
//!
//! Every `SetField` on an instance is recorded as an override — that is
//! how the editor knows what the user made different. Propagating through
//! `SetField` would mark every field it touched, so the first propagation
//! would pin the whole instance and it would never follow the prefab
//! again. Exactly backwards.
//!
//! So propagation carries its own writes to their destination: straight
//! into the registry locally, and as protocol calls in remote mode, both
//! bypassing the action layer that records.
//!
//! # Components appear and disappear with the prefab
//!
//! Adding a component to a prefab puts it on the instances; removing it
//! takes it off. Removing was held back at first as too destructive, and
//! that was the wrong call — a prefab whose instances keep a component it
//! no longer has is not a link, it is a link that works in one direction
//! and surprises you in the other.
//!
//! What makes it safe is that presence is recorded like any other
//! override. A component the user added to *this* instance is theirs and
//! is never deleted; one they took off stays off. So the destructive case
//! — losing something configured by hand — cannot happen, because
//! anything configured by hand is marked as such.
//!
//! A new child entity is still deferred: it has to be positioned relative
//! to whatever the instance became, and there is no answer that is right
//! often enough.

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
    /// Whether the entity has to grow the component before the field can
    /// be written.
    ///
    /// Carried on the write rather than kept as a separate list so the two
    /// cannot be applied out of order — a field written before its
    /// component exists is silently dropped.
    pub add_component: bool,
}

/// Works out everything that should change when `prefab` is saved.
///
/// Computed rather than applied so the caller decides how the writes
/// reach the world — the editor's own registry, or the project's over the
/// wire. Both need the same answer, and it depends on the mirror either
/// way.
pub fn plan(resources: &Resources, prefab: Guid) -> (Vec<PlannedWrite>, Vec<PlannedRemoval>) {
    let Some(document) = cached_document(resources, prefab) else {
        // Every step below is silent on failure, and a silent propagation
        // is indistinguishable from one that decided there was nothing to
        // do. Said out loud so the next report is about a stage rather
        // than about "it does not work".
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
                // The prefab lost an entity this instance still has. Its
                // fields simply stop being propagated; removing it is a
                // structural change and those are deliberately out of
                // scope.
                continue;
            };
            for component in &described.components {
                // A component the prefab grew since this instance was
                // placed. The instance has to grow it too, or the change
                // reaches every instance except as the one thing people
                // most often change about a prefab.
                // A component the user took off this instance stays off.
                // Restoring it on the next save would make removing one
                // impossible to keep.
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
///
/// Stripping these would cut an instance loose from its prefab and orphan
/// its children, which no prefab edit should ever do.
///
/// 🔴 `SceneMember` is on this list for a reason that is easy to miss: it
/// is *never* written to a prefab file. It is derived on load and holds
/// which scene an entity belongs to. So the comparison above — live
/// components against the ones the prefab describes — sees it on every
/// instance, finds it in no document, and calls it debris.
///
/// It stripped scene membership from every prefab-derived entity on the
/// first propagation pass. In `many_lights` that was 181 of 185 entities
/// dropping out of their scene and into "Unsaved" (#955), and nothing
/// failed: they were tagged correctly at load and quietly untagged a
/// moment later. Saving would then have written four entities.
///
/// The same argument as `Parent`, one level up. `Parent` holds an entity
/// to its parent; this holds it to its scene.
fn is_bookkeeping(type_name: &str) -> bool {
    matches!(
        type_name.rsplit("::").next().unwrap_or(type_name),
        "PrefabInstance"
            | "PrefabMember"
            | "Parent"
            | "Children"
            | "GlobalTransform"
            | "SceneMember"
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
///
/// # Why loading a scene has to do this
///
/// A scene file holds its instances in full — see #611 for why that is
/// deliberate — so a prefab edited while the scene was closed leaves stale
/// copies in it. Unity does not have this problem because it does not
/// store the instance at all: a `PrefabInstance` there is a reference plus
/// a list of modifications, merged in at load. There is nothing to update
/// because nothing was written.
///
/// Running propagation at load reaches the same place from the other
/// direction, and keeps what writing them in full buys: a scene that opens
/// correctly when the prefab file is missing, and no ordering dependency
/// between scenes and prefabs.
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
///
/// The local half. In remote mode the project owns the world and the same
/// plan goes over the wire instead — see `remote_edit`.
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

/// Grows `entity` by a default-constructed component, archetype included.
fn add_component(resources: &mut Resources, entity: Entity, type_id: std::any::TypeId) {
    let inserted = resources
        .get_mut::<crate::component::ComponentRegistry>()
        .is_some_and(|registry| registry.insert_default_reflected(&type_id, entity));
    if !inserted {
        return;
    }
    if let Some(archetypes) = resources.get_mut::<crate::archetype_registry::ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(entity)
    {
        let next = archetypes.archetype_after_add_dynamic(current, type_id);
        archetypes.register_entity(entity, next);
    }
}

/// The prefab's document as the editor currently holds it.
///
/// From the cache rather than the file: the cache is what was just saved,
/// and it is also what `spawn_prefab` reads, so propagation and the next
/// spawn agree.
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
///
/// The remote path sends this as a `SetField` instead — which is safe
/// because the recorder skips `PrefabInstance` for exactly this reason.
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

/// Drops overrides on the instance `entity` belongs to, and plans the
/// writes that put the prefab's values back.
///
/// `component` narrows it to one type; `None` reverts the instance.
///
/// Returns the new override set for the root alongside the writes, because
/// both have to be applied: dropping the record without restoring the
/// values leaves the instance looking overridden-free while still showing
/// the user's numbers, and restoring without dropping means the next
/// propagation puts them back.
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
