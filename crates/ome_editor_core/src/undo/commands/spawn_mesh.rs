//! [`SpawnMeshCommand`] — spawns an entity bound to a meshlet asset.
//!
//! Unity-style flow: load the source `.glb` through the
//! [`AssetServer`] (which generates a `.meta` sidecar at first import
//! and registers the resulting [`Guid`] in the [`AssetDatabase`]),
//! then spawn an entity with `Name + Transform + MeshRenderer` and
//! write the resolved GUID into `MeshRenderer.mesh`.
//!
//! Distinct from the generic [`SpawnCommand`](super::SpawnCommand)
//! because the asset-bound case needs a side-effect (load + GUID
//! resolution) before the spawn step, and a direct field write that
//! reflection cannot perform — `MeshRenderer.mesh` is `#[reflect(skip)]`
//! so the inspector never tries to expose an opaque GUID as an editable
//! string.

use std::any::TypeId;
use std::path::PathBuf;

use ome_core::Guid;
use ome_core::asset_database::AssetDatabase;
use ome_core::asset_loader::AssetServer;
use ome_core::resource::Resources;
use ome_ecs::allocator::EntityAllocator;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::entity::Entity;
use ome_ecs::mesh_renderer::MeshRenderer;
use ome_ecs::reflect::ReflectValue;
use ome_render::meshlet::MeshletMesh;

use crate::undo::EditorCommand;

pub(crate) struct SpawnMeshCommand {
    /// Project-relative (or absolute) path to the source `.glb` /
    /// `.gltf` file. The `AssetServer` resolves it against its
    /// configured asset root.
    path: PathBuf,
    /// Display name written into the `Name` component.
    display_name: String,
    /// Entity allocated on first execute, reused on redo.
    entity: Option<Entity>,
    /// GUID resolved from the loaded asset's `.meta` sidecar. Cached
    /// so redo doesn't re-load.
    guid: Option<Guid>,
    /// Component types added during spawn — drives undo cleanup.
    spawned_component_types: Vec<TypeId>,
}

impl SpawnMeshCommand {
    pub fn new(path: PathBuf, display_name: String) -> Self {
        Self {
            path,
            display_name,
            entity: None,
            guid: None,
            spawned_component_types: Vec::new(),
        }
    }

    /// Loads the asset (idempotent: cached after the first execute)
    /// and returns the [`Guid`] the `AssetDatabase` registered for it.
    fn ensure_guid(&mut self, resources: &mut Resources) -> Option<Guid> {
        if let Some(guid) = self.guid {
            return Some(guid);
        }
        // Take the AssetServer so we can both mutate it and pass
        // `&mut Resources` into the load call.
        let mut server = match resources.remove::<AssetServer>() {
            Some(s) => s,
            None => {
                tracing::warn!(
                    target: "ome_editor_core::undo::spawn_mesh",
                    "AssetServer resource missing; cannot resolve mesh GUID",
                );
                return None;
            }
        };
        let load_result = server.load::<MeshletMesh>(&self.path, resources);
        let resolved = server.resolve_path(&self.path);
        resources.insert(server);

        if let Err(e) = load_result {
            tracing::warn!(
                target: "ome_editor_core::undo::spawn_mesh",
                path = %self.path.display(),
                error = %e,
                "failed to load meshlet asset; spawning without mesh GUID",
            );
            return None;
        }

        let guid = resources
            .get::<AssetDatabase>()
            .and_then(|db| db.guid_for(&resolved));
        if guid.is_none() {
            tracing::warn!(
                target: "ome_editor_core::undo::spawn_mesh",
                resolved = %resolved.display(),
                "AssetDatabase lacks an entry for the loaded asset path",
            );
        }
        self.guid = guid;
        guid
    }

    fn spawn_fresh_entity(&self, resources: &mut Resources) -> Entity {
        use ome_ecs::commands::Commands;
        let mut commands = resources
            .remove::<Commands>()
            .expect("Commands resource missing");
        let entity = commands.spawn(resources).id();
        resources.insert(commands);
        entity
    }

    fn do_spawn(&mut self, resources: &mut Resources) {
        let guid = self.ensure_guid(resources);

        // Allocate / revive entity.
        let entity = if let Some(e) = self.entity {
            let revived = resources
                .get_mut::<EntityAllocator>()
                .map(|alloc| alloc.revive(e))
                .unwrap_or(false);
            if revived {
                if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                    archetypes.register_entity(e, ome_ecs::archetype::ArchetypeId::EMPTY);
                }
                e
            } else {
                tracing::warn!(
                    target: "ome_editor_core::undo::spawn_mesh",
                    entity = %e,
                    "redo failed to revive entity; spawning fresh",
                );
                self.spawn_fresh_entity(resources)
            }
        } else {
            self.spawn_fresh_entity(resources)
        };
        self.entity = Some(entity);

        // Component order: Name, Transform, MeshRenderer.
        let mut all_types: Vec<TypeId> = Vec::new();
        if let Some(reg) = resources.get::<ComponentRegistry>() {
            let names = reg.reflected_type_names();
            for needle in &["Name", "Transform"] {
                if let Some((tid, _)) = names
                    .iter()
                    .find(|(_, n)| n.rsplit("::").next().unwrap_or(n) == *needle)
                {
                    all_types.push(*tid);
                }
            }
        }
        all_types.push(TypeId::of::<MeshRenderer>());

        for type_id in &all_types {
            let mut inserted = false;
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                inserted = registry.insert_default_reflected(type_id, entity);
            }
            if inserted {
                if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                    if let Some(current) = archetypes.entity_archetype(entity) {
                        let new_arch = archetypes.archetype_after_add_dynamic(current, *type_id);
                        archetypes.register_entity(entity, new_arch);
                    }
                }
            }
        }
        self.spawned_component_types = all_types;

        // Write the Name component value via reflection.
        let name_tid = TypeId::of::<ome_ecs::Name>();
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            let _ = registry.reflect_set_field(
                &name_tid,
                entity,
                "value",
                ReflectValue::String(self.display_name.clone()),
            );
        }

        // Write the GUID into MeshRenderer.mesh — direct storage access
        // because the field is `#[reflect(skip)]` (opaque GUID, not a
        // user-editable string).
        if let Some(guid) = guid {
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                if let Some(storage) = registry.get_cpu_mut::<MeshRenderer>() {
                    if let Some(renderer) = storage.get_mut(entity) {
                        renderer.mesh = Some(guid);
                    }
                }
            }
        }
    }
}

impl EditorCommand for SpawnMeshCommand {
    fn execute(&mut self, resources: &mut Resources) {
        self.do_spawn(resources);
    }

    fn undo(&mut self, resources: &mut Resources) {
        let Some(entity) = self.entity else { return };

        if let Some(alloc) = resources.get_mut::<EntityAllocator>() {
            alloc.despawn(entity);
        }
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            archetypes.unregister_entity(entity);
        }
        if let Some(components) = resources.get_mut::<ComponentRegistry>() {
            components.remove_entity(entity);
        }
    }

    fn description(&self) -> &str {
        "Spawn Mesh"
    }
}
