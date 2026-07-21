//! Reconstructs a remote project's scene into a local ECS.
//!
//! The editor renders remote state by mirroring it: each
//! [`EntitySnapshot`] pulled over the wire becomes a real local entity,
//! its engine components (`Transform`, `MeshRenderer`, …) inserted so the
//! viewport can draw them, and its project components — which this binary
//! has no Rust type for — parked in [`DynamicComponents`] so the
//! Inspector can still show them.
//!
//! Unlike [`sync_scene_to_ecs`](ome_ecs::scene::sync_scene_to_ecs), the
//! mirror is keyed by [`EntityId`], not by name: a remote scene routinely
//! has several identically-named entities (five `Mesh`es), and resolving
//! parents by name would attach them wrongly. [`RemoteMirror`] keeps a
//! remote→local id map so hierarchy is reconstructed exactly.
//!
//! Mirrored entities carry the [`MirrorEntity`] marker so a re-sync
//! despawns the previous mirror without touching editor-owned entities
//! (camera, gizmos).

use std::collections::HashMap;

use ome_core::resource::Resources;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::commands::Commands;
use ome_ecs::component::{Component, ComponentRegistry};
use ome_ecs::dynamic_components::DynamicComponents;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::Parent;

use ome_remote::protocol::{EntityId, EntitySnapshot};

/// Marker for entities created by the mirror. Registered as ephemeral by
/// the editor so mirrored entities stay out of scene saves, and used here
/// to find the previous mirror when re-syncing.
#[derive(Default)]
pub struct MirrorEntity;
impl Component for MirrorEntity {}

/// Owns the local mirror of a remote scene across refreshes.
///
/// Holds the remote→local id map from the last [`Self::apply`], so
/// selection and hierarchy stay addressable by remote [`EntityId`].
#[derive(Default)]
pub struct RemoteMirror {
    /// Remote entity id → the local entity standing in for it.
    id_map: HashMap<EntityId, Entity>,
}

impl RemoteMirror {
    /// Creates an empty mirror.
    pub fn new() -> Self {
        Self::default()
    }

    /// The local entity currently mirroring `remote`, if any.
    pub fn local_of(&self, remote: EntityId) -> Option<Entity> {
        self.id_map.get(&remote).copied()
    }

    /// Rebuilds the local mirror to match `snapshot`.
    ///
    /// Despawns the previous mirror, then recreates every snapshot entity
    /// with its components and hierarchy. Cheap to call each refresh while
    /// the entity count is small; a diffing update is a later
    /// optimisation once selection stability demands it.
    pub fn apply(&mut self, snapshot: &[EntitySnapshot], resources: &mut Resources) {
        self.despawn_previous(resources);
        self.id_map.clear();

        // First pass: spawn each entity, insert its components, remember
        // the remote→local mapping.
        for snap in snapshot {
            let entity = self.spawn_mirror(resources);
            self.id_map.insert(snap.id, entity);
            for comp in &snap.components {
                insert_component(resources, entity, &comp.type_name, &comp.fields);
            }
        }

        // Second pass: wire parents now that every id is mapped.
        for snap in snapshot {
            let (Some(&child), Some(parent)) = (self.id_map.get(&snap.id), snap.parent) else {
                continue;
            };
            if let Some(&parent_local) = self.id_map.get(&parent) {
                set_parent(resources, child, parent_local);
            }
        }
    }

    /// Despawns the entities created by the previous [`Self::apply`].
    fn despawn_previous(&mut self, resources: &mut Resources) {
        if self.id_map.is_empty() {
            return;
        }
        let mut commands = resources
            .remove::<Commands>()
            .expect("Commands not in Resources");
        for &entity in self.id_map.values() {
            commands.despawn(entity);
        }
        commands.apply(resources);
        resources.insert(commands);
    }

    /// Spawns one marked mirror entity and returns it.
    fn spawn_mirror(&self, resources: &mut Resources) -> Entity {
        let mut commands = resources
            .remove::<Commands>()
            .expect("Commands not in Resources");
        let entity = commands.spawn(resources).id();
        commands.apply(resources);
        resources.insert(commands);

        // Tag it so a later re-sync can find and clear it.
        if resources
            .get_mut::<ComponentRegistry>()
            .is_some_and(|r| r.insert_default_reflected(&type_id::<MirrorEntity>(), entity))
        {
            update_archetype_add(resources, entity, type_id::<MirrorEntity>());
        }
        entity
    }
}

/// Inserts a component named `type_name` on `entity`, setting `fields`.
///
/// A type this binary knows is inserted as a real reflected component (so
/// it participates in rendering and queries); an unknown one is parked in
/// [`DynamicComponents`] so the Inspector can still display it.
fn insert_component(
    resources: &mut Resources,
    entity: Entity,
    type_name: &str,
    fields: &[(String, ome_ecs::reflect::ReflectValue)],
) {
    let type_id = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.type_id_by_name(type_name));

    let Some(type_id) = type_id else {
        if resources.get::<DynamicComponents>().is_none() {
            resources.insert(DynamicComponents::new());
        }
        if let Some(dynamic) = resources.get_mut::<DynamicComponents>() {
            dynamic.insert(entity, type_name, fields.to_vec());
        }
        return;
    };

    let inserted = resources
        .get_mut::<ComponentRegistry>()
        .is_some_and(|r| r.insert_default_reflected(&type_id, entity));
    if inserted {
        update_archetype_add(resources, entity, type_id);
    }
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        for (field, value) in fields {
            if let Err(e) = registry.reflect_set_field(&type_id, entity, field, value.clone()) {
                tracing::debug!(component = type_name, field, "mirror set_field failed: {e}");
            }
        }
    }
}

/// Attaches `child` under `parent` in the mirror.
fn set_parent(resources: &mut Resources, child: Entity, parent: Entity) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<Parent>();
        if let Some(storage) = registry.get_cpu_mut::<Parent>() {
            storage.insert(child, Parent { entity: parent });
        }
    }
    update_archetype_add(resources, child, type_id::<Parent>());
}

/// Moves `entity` to the archetype it belongs in after adding `type_id`.
fn update_archetype_add(resources: &mut Resources, entity: Entity, type_id: std::any::TypeId) {
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(entity)
    {
        let new_arch = archetypes.archetype_after_add_dynamic(current, type_id);
        archetypes.register_entity(entity, new_arch);
    }
}

fn type_id<T: 'static>() -> std::any::TypeId {
    std::any::TypeId::of::<T>()
}
