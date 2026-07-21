//! [`WorldSnapshot`] — capture and restore a world *with its identities*.
//!
//! Saving through [`SceneDocument`](crate::scene::SceneDocument) and
//! loading it back is not a restore: the scene format keys entities by
//! name, so a round-trip despawns everything and respawns it. Handles
//! come back with different indices, different generations and a
//! different order, and the allocator's generation counters keep
//! climbing. Anything holding an [`Entity`] — a selection, a remote
//! mirror, a `Parent`, a system's cached handle — is silently pointing
//! at the wrong thing or at nothing.
//!
//! That is fine for loading a file authored elsewhere. It is wrong for
//! *stop*: pressing stop should leave the world as it was before play,
//! down to the identities, exactly as starting play never happened.
//!
//! [`WorldSnapshot`] therefore captures the [`EntityAllocator`] verbatim
//! alongside per-entity component values keyed by the concrete
//! [`Entity`], and restores both. Handles, generations, the free list
//! and the entity order all come back unchanged.
//!
//! # What is not covered
//!
//! Components with no reflector are not captured — the same limitation
//! the scene format has, since there is no generic way to copy an opaque
//! type. State a system keeps *outside* the ECS (in its own resource) is
//! likewise untouched: this snapshots the world, not the program.

use std::any::TypeId;

use ome_core::resource::Resources;

use crate::allocator::EntityAllocator;
use crate::archetype_registry::ArchetypeRegistry;
use crate::component::ComponentRegistry;
use crate::dynamic_components::DynamicComponents;
use crate::entity::Entity;
use crate::reflect::ReflectValue;

/// One entity's reflected state, addressed by its exact handle.
struct EntityState {
    entity: Entity,
    /// Reflected components, as `(type, fields)`.
    components: Vec<(TypeId, Vec<(String, ReflectValue)>)>,
    /// Components parked by name because this binary has no type for
    /// them — carried so a host mirroring a foreign project restores
    /// those too.
    parked: Vec<(String, Vec<(String, ReflectValue)>)>,
}

/// A restorable capture of the world's entities and their components.
///
/// See the module docs for what it does and does not cover.
pub struct WorldSnapshot {
    /// The allocator as it stood, so restoring reinstates generations
    /// and the free list rather than continuing past them.
    allocator: EntityAllocator,
    /// Entity state in the world's own iteration order, which the
    /// restore reinstates — order is observable, both to systems and to
    /// a client reading the world.
    entities: Vec<EntityState>,
}

impl WorldSnapshot {
    /// Captures the current world.
    pub fn capture(resources: &Resources) -> Self {
        let allocator = resources
            .get::<EntityAllocator>()
            .cloned()
            .unwrap_or_default();

        let mut entities: Vec<EntityState> = Vec::new();
        if let (Some(archetypes), Some(registry)) = (
            resources.get::<ArchetypeRegistry>(),
            resources.get::<ComponentRegistry>(),
        ) {
            let dynamic = resources.get::<DynamicComponents>();
            for archetype in archetypes.iter_matching(&[]) {
                for &entity in archetype.entities() {
                    let components = archetype
                        .components()
                        .iter()
                        .filter_map(|tid| {
                            let fields = registry.reflect_get_fields(tid, entity)?;
                            Some((*tid, fields))
                        })
                        .collect();
                    let parked = dynamic
                        .as_ref()
                        .map(|d| {
                            d.iter_entity(entity)
                                .map(|(name, fields)| (name.to_owned(), fields.to_vec()))
                                .collect()
                        })
                        .unwrap_or_default();
                    entities.push(EntityState {
                        entity,
                        components,
                        parked,
                    });
                }
            }
        }
        Self {
            allocator,
            entities,
        }
    }

    /// Number of captured entities.
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// `true` when nothing was captured.
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Puts the captured world back.
    ///
    /// Clears whatever is there now, reinstates the allocator, then
    /// rebuilds each entity against its original handle — so an
    /// [`Entity`] taken before the capture still addresses the same
    /// thing afterwards.
    pub fn restore(&self, resources: &mut Resources) {
        clear_world(resources);

        if let Some(allocator) = resources.get_mut::<EntityAllocator>() {
            *allocator = self.allocator.clone();
            // The world was rebuilt underneath any GPU-side alive mask,
            // so every slot has to be re-synced, not just the ones that
            // happened to be dirty when the capture was taken.
            allocator.mark_all_pending_sync();
        }

        for state in &self.entities {
            for (type_id, fields) in &state.components {
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    registry.insert_default_reflected(type_id, state.entity);
                    for (field, value) in fields {
                        let _ =
                            registry.reflect_set_field(type_id, state.entity, field, value.clone());
                    }
                }
                add_to_archetype(resources, state.entity, *type_id);
            }
            if !state.parked.is_empty() {
                if resources.get::<DynamicComponents>().is_none() {
                    resources.insert(DynamicComponents::new());
                }
                if let Some(dynamic) = resources.get_mut::<DynamicComponents>() {
                    for (name, fields) in &state.parked {
                        dynamic.insert(state.entity, name, fields.clone());
                    }
                }
            }
        }

        // Rebuilding walks each entity through a chain of archetypes, so
        // where it lands depends on the order its components went in —
        // not the order the world had. Put the observable order back.
        let order: Vec<Entity> = self.entities.iter().map(|e| e.entity).collect();
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            archetypes.reorder_entities(&order);
        }
    }
}

/// Strips every entity's components and archetype membership.
fn clear_world(resources: &mut Resources) {
    let entities: Vec<Entity> = resources
        .get::<ArchetypeRegistry>()
        .map(|a| {
            a.iter_matching(&[])
                .flat_map(|arch| arch.entities().to_vec())
                .collect()
        })
        .unwrap_or_default();

    for entity in entities {
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            registry.remove_entity(entity);
        }
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            archetypes.unregister_entity(entity);
        }
    }
    if let Some(dynamic) = resources.get_mut::<DynamicComponents>() {
        dynamic.clear();
    }
}

/// Moves `entity` into the archetype it belongs in after adding a type.
fn add_to_archetype(resources: &mut Resources, entity: Entity, type_id: TypeId) {
    let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() else {
        return;
    };
    let current = match archetypes.entity_archetype(entity) {
        Some(current) => current,
        // First component: the entity is not in any archetype yet, so
        // start it from the empty one.
        None => {
            let empty = archetypes.get_or_create(Default::default());
            archetypes.register_entity(entity, empty);
            empty
        }
    };
    let next = archetypes.archetype_after_add_dynamic(current, type_id);
    archetypes.register_entity(entity, next);
}

#[cfg(test)]
mod tests {
    use ome_core::resource::Resources;

    use crate::commands::Commands;
    use crate::name::Name;
    use crate::query::AccessTracker;
    use crate::transform::Transform;

    use super::*;

    fn ecs() -> Resources {
        let mut r = Resources::new();
        r.insert(EntityAllocator::new());
        r.insert(ComponentRegistry::new());
        r.insert(ArchetypeRegistry::new());
        r.insert(AccessTracker::new());
        r.insert(Commands::new());
        r.insert(DynamicComponents::new());
        let registry = r.get_mut::<ComponentRegistry>().unwrap();
        registry.register_cpu_reflected::<Name>();
        registry.register_cpu_reflected::<Transform>();
        r
    }

    /// Spawns `count` entities carrying a Transform, returning handles.
    fn spawn_all(resources: &mut Resources, count: usize) -> Vec<Entity> {
        let mut spawned = Vec::new();
        for _ in 0..count {
            let mut commands = resources.remove::<Commands>().unwrap();
            let entity = commands.spawn(resources).id();
            commands.apply(resources);
            resources.insert(commands);
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                registry.insert_default_reflected(&TypeId::of::<Transform>(), entity);
            }
            add_to_archetype(resources, entity, TypeId::of::<Transform>());
            spawned.push(entity);
        }
        spawned
    }

    fn position(resources: &Resources, entity: Entity) -> Option<glam::Vec3> {
        resources
            .get::<ComponentRegistry>()?
            .get_cpu::<Transform>()?
            .get(entity)
            .map(|t| t.position)
    }

    /// The point of the whole type: handles *and* iteration order
    /// survive a round-trip, so anything holding an `Entity` still
    /// addresses the same thing and systems see the same sequence.
    #[test]
    fn restore_preserves_entity_handles_and_values() {
        let mut resources = ecs();
        let spawned = spawn_all(&mut resources, 3);

        // Shuffle the world's iteration order away from index order, so
        // the test would fail if restore merely sorted by index.
        resources
            .get_mut::<ArchetypeRegistry>()
            .unwrap()
            .reorder_entities(&[spawned[2], spawned[0], spawned[1]]);
        let expected: Vec<Entity> = vec![spawned[2], spawned[0], spawned[1]];

        let snapshot = WorldSnapshot::capture(&resources);
        assert_eq!(snapshot.len(), 3);

        // Play: move everything and add an entity.
        for &entity in &spawned {
            let registry = resources.get_mut::<ComponentRegistry>().unwrap();
            let _ = registry.reflect_set_field(
                &TypeId::of::<Transform>(),
                entity,
                "position",
                ReflectValue::Vec3(glam::Vec3::splat(7.0)),
            );
        }
        spawn_all(&mut resources, 1);

        snapshot.restore(&mut resources);

        // Same handles — index *and* generation — same order, and the
        // runtime spawn is gone.
        let restored: Vec<Entity> = resources
            .get::<ArchetypeRegistry>()
            .unwrap()
            .iter_matching(&[])
            .flat_map(|a| a.entities().to_vec())
            .collect();
        assert_eq!(restored, expected, "handles or order churned");
        for &entity in &spawned {
            assert_eq!(position(&resources, entity), Some(glam::Vec3::ZERO));
        }
    }

    /// The allocator comes back too, so the next spawn after a stop gets
    /// the handle it would have got if play had never happened.
    #[test]
    fn restore_rewinds_the_allocator() {
        let mut resources = ecs();
        spawn_all(&mut resources, 2);

        let snapshot = WorldSnapshot::capture(&resources);
        let next_without_play = {
            let mut probe = resources.get::<EntityAllocator>().cloned().unwrap();
            probe.spawn()
        };

        // A play session churns handles: spawn and despawn a few.
        let temps = spawn_all(&mut resources, 3);
        for entity in temps {
            resources
                .get_mut::<EntityAllocator>()
                .unwrap()
                .despawn(entity);
        }

        snapshot.restore(&mut resources);

        let next_after_stop = resources.get_mut::<EntityAllocator>().unwrap().spawn();
        assert_eq!(
            next_after_stop, next_without_play,
            "generations drifted across a play session"
        );
    }

    /// Components with no local Rust type are parked, and must survive a
    /// stop as well — a remote host mirrors a project whose components it
    /// cannot instantiate.
    #[test]
    fn restore_brings_back_parked_components() {
        let mut resources = ecs();
        let entity = spawn_all(&mut resources, 1)[0];
        resources.get_mut::<DynamicComponents>().unwrap().insert(
            entity,
            "game::spin::Spin",
            vec![("rpm".into(), ReflectValue::F32(33.0))],
        );

        let snapshot = WorldSnapshot::capture(&resources);
        resources.get_mut::<DynamicComponents>().unwrap().clear();
        snapshot.restore(&mut resources);

        let parked: Vec<_> = resources
            .get::<DynamicComponents>()
            .unwrap()
            .iter_entity(entity)
            .map(|(name, fields)| (name.to_owned(), fields.to_vec()))
            .collect();
        assert_eq!(parked.len(), 1);
        assert_eq!(parked[0].0, "game::spin::Spin");
        assert_eq!(
            parked[0].1,
            vec![("rpm".to_owned(), ReflectValue::F32(33.0))]
        );
    }
}
