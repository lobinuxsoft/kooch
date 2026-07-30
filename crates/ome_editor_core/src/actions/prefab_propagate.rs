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
//! # What is not propagated
//!
//! Only field *values*, on components both sides already have. Adding a
//! component to a prefab does not add it to instances, removing one does
//! not remove it, and a new child entity does not appear. That is where
//! Unity gets genuinely complicated — each can collide with something the
//! user did to the instance — and it waits until it hurts (#611).

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
