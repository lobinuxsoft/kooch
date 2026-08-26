//! Reconstructs a remote project's scene into a local ECS.
//!
//! The editor renders remote state by mirroring it: each
//! [`EntitySnapshot`] pulled over the wire becomes a real local entity,
//! its engine components (`Transform`, `MeshRenderer`, …) inserted so the
//! viewport can draw them, and its project components — which this binary
//! has no Rust type for — parked in [`DynamicComponents`] so the
//! Inspector can still show them.
//!
//! Unlike [`sync_scene_to_ecs`](kooch_ecs::scene::sync_scene_to_ecs), the
//! mirror is keyed by [`EntityId`], not by name: a remote scene routinely
//! has several identically-named entities (five `Mesh`es), and resolving
//! parents by name would attach them wrongly. [`RemoteMirror`] keeps a
//! remote→local id map so hierarchy is reconstructed exactly.
//!
//! Mirrored entities carry the [`MirrorEntity`] marker so a re-sync
//! despawns the previous mirror without touching editor-owned entities
//! (camera, gizmos).

use std::collections::{HashMap, HashSet};

use kooch_core::resource::Resources;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::Parent;

use kooch_remote::protocol::{EntityId, EntitySnapshot};

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
    /// The inverse: local entity → remote id, so an edit made against a
    /// mirrored entity can be addressed back to the server.
    remote_map: HashMap<Entity, EntityId>,
    /// Component type names applied to each mirrored entity last time.
    /// The diff needs it to notice a component the project has since
    /// removed — the local ECS cannot be asked, since parked components
    /// live outside the archetype.
    components: HashMap<Entity, Vec<String>>,
    /// The scenes the last snapshot said its entities belonged to.
    ///
    /// 🔴 The mirror is a DIFF and that is load-bearing for selection,
    /// so nothing here ever announces "a new world" on its own: a scene
    /// swap on the project's side arrives as a batch of despawns and a
    /// batch of spawns, indistinguishable one entity at a time from a
    /// game destroying things. The set of scenes is what tells them
    /// apart, and it is the same question `SceneManager::epoch` answers
    /// for a local session.
    scenes: HashSet<kooch_core::Guid>,
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

    /// The remote id behind a mirrored local entity, if any. Used to
    /// route an edit made in the editor back to the owning project.
    pub fn remote_of(&self, local: Entity) -> Option<EntityId> {
        self.remote_map.get(&local).copied()
    }

    /// Despawns every mirrored entity and forgets the id maps.
    ///
    /// Called when the session ends: the mirror's entities are ephemeral,
    /// so the ordinary project-close sweep leaves them behind.
    pub fn clear(&mut self, resources: &mut Resources) {
        self.retain_only(&[], resources);
        self.id_map.clear();
        self.remote_map.clear();
        self.components.clear();
        // Forgotten too, or reconnecting to the SAME scene reads as no
        // change and the mirror comes back over a stale page cache.
        self.scenes.clear();
    }

    /// Updates the local mirror to match `snapshot`.
    ///
    /// A **diff**, not a rebuild: an entity whose remote id was already
    /// mirrored keeps its local [`Entity`] and has its fields written in
    /// place. That stability is load-bearing — selection, the Inspector's
    /// open headers and the gizmo all address entities by handle, so
    /// respawning the world every refresh would drop the user's selection
    /// twice a second, and once per frame while the project is playing.
    pub fn apply(&mut self, snapshot: &[EntitySnapshot], resources: &mut Resources) {
        self.retain_only(snapshot, resources);

        // First pass: reuse or create the local entity for each snapshot
        // entry, so every id is mapped before anything reads the map.
        for snap in snapshot {
            if !self.id_map.contains_key(&snap.id) {
                let entity = self.spawn_mirror(resources);
                self.id_map.insert(snap.id, entity);
                self.remote_map.insert(entity, snap.id);
            }
        }

        // Second pass: sync each entity's components. Separate from the
        // pass above because a component can point at another entity, and
        // translating that reference needs the whole map — an entity later
        // in the snapshot than the one referring to it is ordinary.
        for snap in snapshot {
            let Some(&entity) = self.id_map.get(&snap.id) else {
                continue;
            };
            self.sync_components(resources, entity, snap);
        }

        // Third pass: the two things that travel beside the components
        // rather than as components — the parent and the scene.
        //
        // The parent travels as its own snapshot field rather than as a
        // component, so it needs its own sync — `sync_components` above
        // never sees it and cannot retire it. Both directions matter: an
        // entity the project reports with no parent has to *lose* its local
        // `Parent`, or unparenting is invisible in the mirror while
        // parenting works, which is exactly how it read.
        for snap in snapshot {
            let Some(&child) = self.id_map.get(&snap.id) else {
                continue;
            };
            match snap.parent.and_then(|parent| self.id_map.get(&parent)) {
                Some(&parent_local) => set_parent(resources, child, parent_local),
                None => clear_parent(resources, child),
            }
            set_scene(resources, child, snap.scene);
        }

        // Last, so it reports a world that is already standing. Readers
        // of the epoch run in later stages, but a bump published over a
        // half-applied mirror is a claim this cannot make honestly.
        let scenes: HashSet<kooch_core::Guid> = snapshot.iter().filter_map(|s| s.scene).collect();
        if scenes != self.scenes {
            self.scenes = scenes;
            if let Some(manager) = resources.get_mut::<kooch_ecs::SceneManager>() {
                manager.world_replaced();
            }
        }
    }

    /// Despawns mirrored entities whose remote id is absent from
    /// `snapshot` — they were despawned on the project's side.
    fn retain_only(&mut self, snapshot: &[EntitySnapshot], resources: &mut Resources) {
        let alive: HashSet<EntityId> = snapshot.iter().map(|s| s.id).collect();
        let gone: Vec<(EntityId, Entity)> = self
            .id_map
            .iter()
            .filter(|(id, _)| !alive.contains(id))
            .map(|(id, entity)| (*id, *entity))
            .collect();
        if gone.is_empty() {
            return;
        }
        let mut commands = resources
            .remove::<Commands>()
            .expect("Commands not in Resources");
        for (id, entity) in gone {
            commands.despawn(entity);
            self.id_map.remove(&id);
            self.remote_map.remove(&entity);
            self.components.remove(&entity);
        }
        commands.apply(resources);
        resources.insert(commands);
    }

    /// Brings `entity`'s components in line with `snap`: writes the
    /// fields of every component present, and drops any the project no
    /// longer has.
    fn sync_components(
        &mut self,
        resources: &mut Resources,
        entity: Entity,
        snap: &EntitySnapshot,
    ) {
        for comp in &snap.components {
            let fields = self.localise_refs(&comp.fields);
            insert_component(resources, entity, &comp.type_name, &fields);
        }

        let present: Vec<String> = snap
            .components
            .iter()
            .map(|c| c.type_name.clone())
            .collect();
        if let Some(previous) = self.components.get(&entity) {
            for stale in previous.iter().filter(|n| !present.contains(n)) {
                remove_component(resources, entity, stale);
            }
        }
        self.components.insert(entity, present);
    }

    /// Rewrites the entity references in `fields` to name mirror entities.
    ///
    /// The project's handles mean nothing here: index 4 in the project is
    /// not index 4 in the editor. Left untranslated, a joint's Body A
    /// would read as whichever mirror entity happened to land on that
    /// index — or as missing, which is what it looked like.
    ///
    /// A reference the map does not know is passed through rather than
    /// blanked: the target is an entity the snapshot does not carry (an
    /// ephemeral one), and the Inspector saying "missing" is truer than it
    /// saying "none".
    ///
    /// Allocates only for a component that actually holds a reference —
    /// the common case borrows nothing and copies the slice as it is.
    fn localise_refs(
        &self,
        fields: &[(String, kooch_ecs::reflect::ReflectValue)],
    ) -> Vec<(String, kooch_ecs::reflect::ReflectValue)> {
        use kooch_ecs::reflect::{EntityRef, ReflectValue};

        fields
            .iter()
            .map(|(name, value)| {
                let ReflectValue::EntityRef(Some(reference)) = value else {
                    return (name.clone(), value.clone());
                };
                let localised = match reference.entity() {
                    Some(remote) => self
                        .id_map
                        .get(&EntityId::from(remote))
                        .map(|&local| EntityRef::live(local))
                        .unwrap_or(*reference),
                    // Persistent: an identity, not a handle. It means the
                    // same thing in both processes.
                    None => *reference,
                };
                (name.clone(), ReflectValue::EntityRef(Some(localised)))
            })
            .collect()
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
    fields: &[(String, kooch_ecs::reflect::ReflectValue)],
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

/// Drops a component the project no longer has from the mirror.
fn remove_component(resources: &mut Resources, entity: Entity, type_name: &str) {
    let type_id = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.type_id_by_name(type_name));

    let Some(type_id) = type_id else {
        if let Some(dynamic) = resources.get_mut::<DynamicComponents>() {
            dynamic.remove(entity, type_name);
        }
        return;
    };

    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.remove_component(entity, &type_id);
    }
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(entity)
    {
        let new_arch = archetypes.archetype_after_remove_dynamic(current, type_id);
        archetypes.register_entity(entity, new_arch);
    }
}

/// Records which scene a mirrored entity belongs to, or that it belongs
/// to none.
///
/// 🔴 Membership arrives in `EntitySnapshot::scene`, never among the
/// components: the host skips `SceneMember` when it builds a snapshot so
/// the fact is on the wire once. Without this the World panel groups a
/// mirrored world into an empty scene and a pile of orphans, which is
/// what it did: every entity under "Unsaved" while the open scene
/// reported zero.
///
/// Registered reflected — a generic world rebuild has to be able to carry
/// it — but hidden from the Inspector, since it is derived rather than
/// authored.
fn set_scene(resources: &mut Resources, entity: Entity, scene: Option<kooch_core::Guid>) {
    let current = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<kooch_ecs::SceneMember>())
        .and_then(|s| s.get(entity))
        .map(|member| member.scene);
    if current == scene {
        // Unchanged, and skipping keeps the archetype churn off every
        // entity on every refresh.
        return;
    }
    match scene {
        Some(scene) => {
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                registry.register_cpu_reflected::<kooch_ecs::SceneMember>();
                if let Some(storage) = registry.get_cpu_mut::<kooch_ecs::SceneMember>() {
                    storage.insert(entity, kooch_ecs::SceneMember::new(scene));
                }
            }
            update_archetype_add(resources, entity, type_id::<kooch_ecs::SceneMember>());
        }
        None => {
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                registry.remove_component(entity, &type_id::<kooch_ecs::SceneMember>());
            }
            if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
                && let Some(current) = archetypes.entity_archetype(entity)
            {
                let without = archetypes
                    .archetype_after_remove_dynamic(current, type_id::<kooch_ecs::SceneMember>());
                archetypes.register_entity(entity, without);
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

/// Drops `child`'s `Parent`, making it a root.
///
/// Deliberately *not* `kooch_ecs::hierarchy::reparent`: that rewrites the
/// child's local transform to preserve its world pose, which is right when a
/// person drags something in the editor and wrong here. The mirror is
/// copying a world the project already resolved — the project sends the
/// local transform it wants, and rewriting it on this side would fight the
/// snapshot every refresh.
fn clear_parent(resources: &mut Resources, child: Entity) {
    let had_parent = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Parent>())
        .is_some_and(|s| s.contains(child));
    if !had_parent {
        // Nothing to do, and skipping keeps the archetype churn off every
        // refresh for every root entity in the scene.
        return;
    }
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.remove_component(child, &type_id::<Parent>());
    }
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(child)
    {
        let new_arch = archetypes.archetype_after_remove_dynamic(current, type_id::<Parent>());
        archetypes.register_entity(child, new_arch);
    }
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
