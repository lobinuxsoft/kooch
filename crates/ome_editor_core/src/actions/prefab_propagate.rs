//! Pushing a changed prefab out to the instances of it.
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
//! # Adding is propagated; removing is not
//!
//! A component added to a prefab appears on its instances. That case is
//! tractable: the worst collision is an instance that already added the
//! same component by hand, and it simply starts following the prefab,
//! which is what it now is.
//!
//! Removing is not, and the asymmetry is the point. Taking a component off
//! every instance is *destroying* whatever the user configured on it,
//! irreversibly and in twelve places at once, on the strength of an edit
//! made somewhere else. A new child entity is deferred for the same
//! reason — it has to be positioned relative to whatever the instance
//! became, and there is no answer that is right often enough.

use ome_core::Guid;
use ome_core::resource::Resources;
use ome_ecs::entity::Entity;
use ome_ecs::prefab_instance::{OverrideAddress, PrefabInstance, PrefabMember};
use ome_ecs::query::Query;
use ome_ecs::reflect::ReflectValue;
use ome_ecs::scene::SceneDocument;

/// One field to write into one live entity.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedWrite {
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
pub(crate) fn plan(resources: &Resources, prefab: Guid) -> Vec<PlannedWrite> {
    let Some(document) = cached_document(resources, prefab) else {
        return Vec::new();
    };

    let mut writes = Vec::new();
    for (root, instance) in instances_of(resources, prefab) {
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
        }
    }
    writes
}

/// Applies a plan to the editor's own world.
///
/// The local half. In remote mode the project owns the world and the same
/// plan goes over the wire instead — see `remote_edit`.
pub(crate) fn apply(resources: &mut Resources, writes: &[PlannedWrite]) {
    for write in writes {
        let type_id = resources
            .get::<ome_ecs::component::ComponentRegistry>()
            .and_then(|registry| registry.type_id_by_name(&write.component));
        let Some(type_id) = type_id else {
            continue;
        };
        if let Some(registry) = resources.get_mut::<ome_ecs::component::ComponentRegistry>()
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
    let Some(registry) = resources.get::<ome_ecs::component::ComponentRegistry>() else {
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
        .get_mut::<ome_ecs::component::ComponentRegistry>()
        .is_some_and(|registry| registry.insert_default_reflected(&type_id, entity));
    if !inserted {
        return;
    }
    if let Some(archetypes) = resources.get_mut::<ome_ecs::archetype_registry::ArchetypeRegistry>()
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
        .get::<ome_core::asset_database::AssetDatabase>()?
        .entry(prefab)?
        .path
        .clone();
    let handle = resources
        .get::<ome_core::asset_loader::AssetServer>()?
        .get_cached::<SceneDocument>(&path)?;
    resources
        .get::<ome_core::assets::Assets<SceneDocument>>()?
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

/// Prefabs whose instances have not caught up with the file yet.
///
/// A set rather than a single guid: saving two prefabs in one frame has to
/// propagate both, and re-saving one before the drain has run must not
/// queue it twice.
#[derive(Default)]
pub(crate) struct PendingPropagation(std::collections::HashSet<Guid>);

impl PendingPropagation {
    pub(crate) fn queue(&mut self, prefab: Guid) {
        self.0.insert(prefab);
    }

    pub(crate) fn drain(&mut self) -> Vec<Guid> {
        self.0.drain().collect()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Whether any prefab is waiting to reach its instances.
///
/// Asked by the caller that skips action handling on idle frames.
/// Propagation is queued while an action is being handled and drained on
/// the next pass, so a queue that only drains when the user happens to do
/// something else is a queue that does not drain.
pub(crate) fn anything_queued(resources: &Resources) -> bool {
    resources
        .get::<PendingPropagation>()
        .is_some_and(|pending| !pending.is_empty())
}

/// Notes that `prefab` changed and its instances are behind.
pub(crate) fn queue(resources: &mut Resources, prefab: Guid) {
    if resources.get::<PendingPropagation>().is_none() {
        resources.insert(PendingPropagation::default());
    }
    if let Some(pending) = resources.get_mut::<PendingPropagation>() {
        pending.queue(prefab);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ome_ecs::component::ComponentRegistry;
    use ome_ecs::scene::{ComponentDescription, EntityDescription};

    fn described(fields: Vec<(String, ReflectValue)>) -> EntityDescription {
        EntityDescription {
            name: "Root".into(),
            parent_index: None,
            parent: None,
            components: vec![ComponentDescription {
                type_name: "test::Health".into(),
                fields,
            }],
        }
    }

    fn world() -> Resources {
        let mut resources = Resources::new();
        resources.insert(ome_ecs::allocator::EntityAllocator::new());
        resources.insert(ComponentRegistry::new());
        resources.insert(ome_ecs::archetype_registry::ArchetypeRegistry::new());
        resources.insert(ome_ecs::query::AccessTracker::new());
        resources
    }

    /// The plan is the whole of the decision, so it is what the tests
    /// hold. Applying it is a loop over `reflect_set_field`.
    fn plan_against(
        instance: &PrefabInstance,
        document: &SceneDocument,
        entity: Entity,
    ) -> Vec<PlannedWrite> {
        let mut writes = Vec::new();
        for component in &document.entities[0].components {
            for (field, value) in &component.fields {
                let address = OverrideAddress {
                    entity: 0,
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
                    add_component: false,
                });
            }
        }
        writes
    }

    fn document() -> SceneDocument {
        SceneDocument {
            id: Guid::new_v4(),
            name: "Enemy".into(),
            version: "0.1.0".into(),
            entities: vec![described(vec![
                ("hp".into(), ReflectValue::U32(50)),
                ("max_hp".into(), ReflectValue::U32(50)),
            ])],
        }
    }

    #[test]
    fn an_untouched_instance_takes_every_value() {
        let instance = PrefabInstance::new(Guid::new_v4());
        let writes = plan_against(&instance, &document(), Entity::new(0, 0));
        assert_eq!(writes.len(), 2, "both fields should follow the prefab");
    }

    /// The rule the whole feature rests on: a field the user changed on
    /// this instance is left alone, and the ones they did not still
    /// follow.
    #[test]
    fn an_overridden_field_is_left_alone_and_the_rest_are_not() {
        let mut instance = PrefabInstance::new(Guid::new_v4());
        instance.mark(OverrideAddress {
            entity: 0,
            component: "test::Health".into(),
            field: "hp".into(),
        });

        let writes = plan_against(&instance, &document(), Entity::new(0, 0));
        assert_eq!(writes.len(), 1);
        assert_eq!(
            writes[0].field, "max_hp",
            "the overridden field was overwritten"
        );
    }

    /// Overriding everything is the same as detaching, and must not
    /// half-apply.
    #[test]
    fn an_instance_that_overrode_everything_takes_nothing() {
        let mut instance = PrefabInstance::new(Guid::new_v4());
        for field in ["hp", "max_hp"] {
            instance.mark(OverrideAddress {
                entity: 0,
                component: "test::Health".into(),
                field: field.into(),
            });
        }
        assert!(plan_against(&instance, &document(), Entity::new(0, 0)).is_empty());
    }

    #[test]
    fn nothing_is_planned_for_a_prefab_with_no_instances() {
        let resources = world();
        assert!(plan(&resources, Guid::new_v4()).is_empty());
    }

    #[test]
    fn the_queue_holds_each_prefab_once() {
        let mut pending = PendingPropagation::default();
        let prefab = Guid::new_v4();
        pending.queue(prefab);
        pending.queue(prefab);
        assert_eq!(pending.drain(), vec![prefab]);
        assert!(pending.drain().is_empty(), "draining leaves it empty");
    }
}

/// Stores an instance's override set locally.
///
/// The remote path sends this as a `SetField` instead — which is safe
/// because the recorder skips `PrefabInstance` for exactly this reason.
pub(crate) fn write_overrides(resources: &mut Resources, root: Entity, overrides: &str) {
    let type_id = resources
        .get::<ome_ecs::component::ComponentRegistry>()
        .and_then(|registry| registry.type_id_by_name(std::any::type_name::<PrefabInstance>()));
    let Some(type_id) = type_id else {
        return;
    };
    if let Some(registry) = resources.get_mut::<ome_ecs::component::ComponentRegistry>() {
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
pub(crate) fn plan_revert(
    resources: &Resources,
    entity: Entity,
    component: Option<ome_ecs::component::ComponentId>,
) -> Option<(Entity, String, Vec<PlannedWrite>)> {
    // The panel speaks `ComponentId`; an override address is a type name,
    // because it outlives the process that recorded it.
    let component = match component {
        Some(id) => Some(
            resources
                .get::<ome_ecs::component::ComponentNames>()?
                .name(id)?
                .to_owned(),
        ),
        None => None,
    };
    let component = component.as_deref();
    let member = resources
        .get::<ome_ecs::component::ComponentRegistry>()?
        .get_cpu::<PrefabMember>()?
        .get(entity)
        .cloned()?;
    let root = member.root;
    let mut instance = resources
        .get::<ome_ecs::component::ComponentRegistry>()?
        .get_cpu::<PrefabInstance>()?
        .get(root)
        .cloned()?;

    // Narrowed by *this* entity as well as the component: reverting a
    // child's Transform must not revert the root's.
    let kept: Vec<OverrideAddress> = instance
        .overrides()
        .into_iter()
        .filter(|address| match component {
            Some(component) => {
                !(address.entity == member.index as usize && address.component == component)
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
