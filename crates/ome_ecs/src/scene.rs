//! Scene document and serialization.
//!
//! [`SceneDocument`] is the source-of-truth scene format (`.ome_scene`).
//! It stores entity descriptions with reflected component data that can be
//! serialized to/from RON files. The live ECS is just a mirror — the scene
//! file is what persists between sessions.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::allocator::EntityAllocator;
use crate::archetype_registry::ArchetypeRegistry;
use crate::commands::Commands;
use crate::component::ComponentRegistry;
use crate::reflect::{ReflectError, ReflectValue};
use ome_core::resource::Resources;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Top-level scene container, serialized as RON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SceneDocument {
    pub name: String,
    pub version: String,
    pub entities: Vec<EntityDescription>,
}

/// A single entity with its named components.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntityDescription {
    pub name: String,
    /// Parent entity name (for hierarchy reconstruction on load).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub components: Vec<ComponentDescription>,
}

/// A single component stored as its full type path and reflected fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComponentDescription {
    pub type_name: String,
    pub fields: Vec<(String, ReflectValue)>,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during scene save/load/sync.
#[derive(Debug)]
pub enum SceneError {
    /// File I/O error.
    Io(std::io::Error),
    /// RON serialization error.
    Ron(ron::Error),
    /// RON deserialization error (with span info).
    RonSpanned(ron::error::SpannedError),
    /// A component type referenced in the scene is not registered.
    UnknownComponent(String),
    /// A reflection operation failed.
    Reflect(ReflectError),
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "failed to access scene file: {e}"),
            Self::Ron(e) => write!(f, "failed to serialize scene RON: {e}"),
            Self::RonSpanned(e) => write!(f, "failed to parse scene RON: {e}"),
            Self::UnknownComponent(name) => write!(f, "unknown component type: {name}"),
            Self::Reflect(e) => write!(f, "reflection error: {e}"),
        }
    }
}

impl std::error::Error for SceneError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Ron(e) => Some(e),
            Self::RonSpanned(e) => Some(e),
            Self::Reflect(e) => Some(e),
            Self::UnknownComponent(_) => None,
        }
    }
}

impl From<std::io::Error> for SceneError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<ron::Error> for SceneError {
    fn from(e: ron::Error) -> Self {
        Self::Ron(e)
    }
}

impl From<ron::error::SpannedError> for SceneError {
    fn from(e: ron::error::SpannedError) -> Self {
        Self::RonSpanned(e)
    }
}

impl From<ReflectError> for SceneError {
    fn from(e: ReflectError) -> Self {
        Self::Reflect(e)
    }
}

// ---------------------------------------------------------------------------
// SceneDocument methods
// ---------------------------------------------------------------------------

impl SceneDocument {
    /// Saves the scene as pretty-printed RON to `path`.
    pub fn save(&self, path: &Path) -> Result<(), SceneError> {
        let config = ron::ser::PrettyConfig::default()
            .struct_names(false)
            .enumerate_arrays(false);
        let data = ron::ser::to_string_pretty(self, config)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    /// Loads a scene from a RON file at `path`.
    pub fn load(path: &Path) -> Result<Self, SceneError> {
        let data = std::fs::read_to_string(path)?;
        let doc: Self = ron::from_str(&data)?;
        Ok(doc)
    }

    /// Snapshots the current ECS state into a `SceneDocument`.
    ///
    /// Iterates all archetypes/entities and captures reflected component
    /// fields. Entities without any reflected components are skipped.
    ///
    /// Hierarchy components (`Parent`, `Children`, `GlobalTransform`) are
    /// excluded from the component list. Instead, parent relationships are
    /// stored as an entity name reference in `EntityDescription::parent`.
    ///
    /// Entities whose archetype contains a marker registered in
    /// [`EphemeralComponents`](crate::ephemeral::EphemeralComponents) are
    /// skipped entirely — used by editor crates to keep helper entities
    /// (cameras, gizmos) out of user scene files.
    pub fn from_ecs(resources: &Resources) -> Self {
        use crate::ephemeral::EphemeralComponents;
        use crate::hierarchy::{Children, GlobalTransform, Parent};

        // Type IDs of hierarchy components to skip during serialization.
        let skip_types = [
            std::any::TypeId::of::<Parent>(),
            std::any::TypeId::of::<Children>(),
            std::any::TypeId::of::<GlobalTransform>(),
        ];

        // Snapshot ephemeral markers; default to empty if the resource is
        // not present (e.g., headless tests without an editor plugin).
        let ephemeral = resources
            .get::<EphemeralComponents>()
            .map(|e| e.clone())
            .unwrap_or_default();

        // (entity_index, EntityDescription) for stable ordering.
        let mut indexed_entities: Vec<(u32, EntityDescription)> = Vec::new();
        // Map entity → vec index for parent name lookup.
        let mut entity_to_idx: std::collections::HashMap<crate::entity::Entity, usize> =
            std::collections::HashMap::new();

        let archetypes = resources.get::<ArchetypeRegistry>();
        let components = resources.get::<ComponentRegistry>();

        if let (Some(archetypes), Some(components)) = (archetypes, components) {
            for archetype in archetypes.iter_matching(&[]) {
                // Skip entire archetypes that carry an ephemeral marker.
                // Every entity in an archetype shares the same component
                // set, so the decision is per-archetype, not per-entity.
                if ephemeral.intersects(archetype.components()) {
                    continue;
                }
                for &entity in archetype.entities() {
                    let mut comp_descs = Vec::new();

                    for &type_id in archetype.components() {
                        // Skip hierarchy components.
                        if skip_types.contains(&type_id) {
                            continue;
                        }

                        let Some(type_name) = components.component_name(&type_id) else {
                            continue;
                        };

                        if !components.has_reflector(&type_id) {
                            continue;
                        }

                        let Some(fields) = components.reflect_get_fields(&type_id, entity) else {
                            continue;
                        };

                        comp_descs.push(ComponentDescription {
                            type_name: type_name.to_owned(),
                            fields,
                        });
                    }

                    if comp_descs.is_empty() {
                        continue;
                    }

                    // Try to extract a display name from a "Name" component's
                    // "value" field, falling back to "Entity <index>".
                    let display_name = comp_descs
                        .iter()
                        .find(|c| {
                            c.type_name
                                .rsplit("::")
                                .next()
                                .unwrap_or(&c.type_name)
                                == "Name"
                        })
                        .and_then(|c| {
                            c.fields.iter().find_map(|(k, v)| {
                                if k == "value" {
                                    if let ReflectValue::String(s) = v {
                                        if !s.is_empty() {
                                            return Some(s.clone());
                                        }
                                    }
                                }
                                None
                            })
                        })
                        .unwrap_or_else(|| format!("Entity {}", entity.index()));

                    let idx = indexed_entities.len();
                    entity_to_idx.insert(entity, idx);
                    indexed_entities.push((
                        entity.index(),
                        EntityDescription {
                            name: display_name,
                            parent: None, // Filled in second pass.
                            components: comp_descs,
                        },
                    ));
                }
            }

            // Second pass: resolve parent names.
            if let Some(parent_storage) = components.get_cpu::<Parent>() {
                for (&entity, idx) in &entity_to_idx {
                    if let Some(parent_comp) = parent_storage.get(entity) {
                        if let Some(&parent_idx) = entity_to_idx.get(&parent_comp.entity) {
                            indexed_entities[*idx].1.parent =
                                Some(indexed_entities[parent_idx].1.name.clone());
                        }
                    }
                }
            }
        }

        // Sort by entity index for stable ordering across save/load.
        indexed_entities.sort_by_key(|(idx, _)| *idx);
        let entities: Vec<EntityDescription> =
            indexed_entities.into_iter().map(|(_, desc)| desc).collect();

        SceneDocument {
            name: "Untitled Scene".into(),
            version: "0.1.0".into(),
            entities,
        }
    }
}

/// Clears the live ECS and rebuilds it from a [`SceneDocument`].
///
/// All existing entities are despawned. For each entity in the document,
/// a new entity is spawned, and its components are inserted via reflection.
/// Hierarchy relationships are reconstructed from `EntityDescription::parent`.
pub fn sync_scene_to_ecs(
    scene: &SceneDocument,
    resources: &mut Resources,
) -> Result<(), SceneError> {
    use crate::hierarchy::Parent;

    // 1. Despawn all existing entities.
    despawn_all(resources);

    // 2. First pass: spawn entities and insert components.
    // Track name → Entity for parent resolution.
    let mut name_to_entity: std::collections::HashMap<String, crate::entity::Entity> =
        std::collections::HashMap::new();
    let mut spawned_order: Vec<(crate::entity::Entity, Option<String>)> = Vec::new();

    for entity_desc in &scene.entities {
        // Spawn a fresh entity.
        let entity = {
            let mut commands = resources
                .remove::<Commands>()
                .expect("Commands not found in Resources");
            let entity = commands.spawn(resources).id();
            resources.insert(commands);
            entity
        };

        name_to_entity.insert(entity_desc.name.clone(), entity);
        spawned_order.push((entity, entity_desc.parent.clone()));

        for comp_desc in &entity_desc.components {
            // Look up the TypeId by full type name.
            let type_id = {
                let components = resources
                    .get::<ComponentRegistry>()
                    .ok_or_else(|| {
                        SceneError::UnknownComponent(comp_desc.type_name.clone())
                    })?;
                components
                    .type_id_by_name(&comp_desc.type_name)
                    .ok_or_else(|| {
                        SceneError::UnknownComponent(comp_desc.type_name.clone())
                    })?
            };

            // Insert default component via reflection.
            {
                let mut inserted = false;
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    inserted = registry.insert_default_reflected(&type_id, entity);
                }
                if inserted {
                    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                        if let Some(current) = archetypes.entity_archetype(entity) {
                            let new_arch =
                                archetypes.archetype_after_add_dynamic(current, type_id);
                            archetypes.register_entity(entity, new_arch);
                        }
                    }
                }
            }

            // Set each field value.
            for (field_name, value) in &comp_desc.fields {
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    registry.reflect_set_field(
                        &type_id,
                        entity,
                        field_name,
                        value.clone(),
                    )?;
                }
            }
        }
    }

    // 3. Second pass: establish hierarchy from parent names.
    let parent_tid = std::any::TypeId::of::<Parent>();
    for (entity, parent_name) in &spawned_order {
        if let Some(parent_name) = parent_name {
            if let Some(&parent_entity) = name_to_entity.get(parent_name) {
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    registry.register_cpu_reflected::<Parent>();
                    if let Some(storage) = registry.get_cpu_mut::<Parent>() {
                        storage.insert(*entity, Parent { entity: parent_entity });
                    }
                }
                // Update the archetype to include Parent.
                if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                    if let Some(current) = archetypes.entity_archetype(*entity) {
                        let new_arch =
                            archetypes.archetype_after_add_dynamic(current, parent_tid);
                        archetypes.register_entity(*entity, new_arch);
                    }
                }
            }
        }
    }

    Ok(())
}

/// Despawns every alive entity in the ECS, except those marked ephemeral.
///
/// Entities whose archetype contains a marker registered in
/// [`EphemeralComponents`](crate::ephemeral::EphemeralComponents) are
/// preserved across scene loads. This keeps editor helper entities
/// (cameras, gizmos) alive when the user opens a different scene.
fn despawn_all(resources: &mut Resources) {
    use crate::ephemeral::EphemeralComponents;

    // Snapshot ephemeral markers; default to empty if the resource is
    // not present (e.g., headless tests without an editor plugin).
    let ephemeral = resources
        .get::<EphemeralComponents>()
        .map(|e| e.clone())
        .unwrap_or_default();

    // Collect all alive entities from archetypes, skipping ephemeral ones.
    let entities: Vec<_> = resources
        .get::<ArchetypeRegistry>()
        .map(|archetypes| {
            archetypes
                .iter_matching(&[])
                .filter(|arch| !ephemeral.intersects(arch.components()))
                .flat_map(|arch| arch.entities().to_vec())
                .collect()
        })
        .unwrap_or_default();

    for entity in entities {
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

    // GC empty archetypes after clearing everything.
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
        archetypes.gc_empty_archetypes();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::Component;
    use crate::query::{AccessTracker, Query};
    use crate::reflect::{FieldKind, FieldMeta, Reflect};
    use crate::transform::Transform;
    use glam::Vec3;

    // -- Test component with manual Reflect impl ----------------------------

    #[derive(Debug, Clone, PartialEq)]
    struct Health {
        hp: u32,
        max_hp: u32,
    }

    impl Component for Health {}

    impl Reflect for Health {
        fn reflect_fields(&self) -> &'static [FieldMeta] {
            static FIELDS: &[FieldMeta] = &[
                FieldMeta {
                    name: "hp",
                    type_name: "u32",
                    kind: FieldKind::U32,
                    choices: &[],
                    asset_type: "",
                },
                FieldMeta {
                    name: "max_hp",
                    type_name: "u32",
                    kind: FieldKind::U32,
                    choices: &[],
                    asset_type: "",
                },
            ];
            FIELDS
        }

        fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
            match field {
                "hp" => Some(ReflectValue::U32(self.hp)),
                "max_hp" => Some(ReflectValue::U32(self.max_hp)),
                _ => None,
            }
        }

        fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), ReflectError> {
            match field {
                "hp" => match value {
                    ReflectValue::U32(v) => {
                        self.hp = v;
                        Ok(())
                    }
                    other => Err(ReflectError::TypeMismatch {
                        field: "hp".into(),
                        expected: FieldKind::U32,
                        got: other.kind(),
                    }),
                },
                "max_hp" => match value {
                    ReflectValue::U32(v) => {
                        self.max_hp = v;
                        Ok(())
                    }
                    other => Err(ReflectError::TypeMismatch {
                        field: "max_hp".into(),
                        expected: FieldKind::U32,
                        got: other.kind(),
                    }),
                },
                _ => Err(ReflectError::FieldNotFound(field.into())),
            }
        }

        fn reflect_default() -> Self {
            Health {
                hp: 100,
                max_hp: 100,
            }
        }
    }

    /// Helper: set up a minimal ECS with `Commands`, `ComponentRegistry`,
    /// `ArchetypeRegistry`, `EntityAllocator`, and `AccessTracker`.
    fn setup_resources() -> Resources {
        let mut resources = Resources::new();
        resources.insert(EntityAllocator::new());
        resources.insert(ComponentRegistry::new());
        resources.insert(ArchetypeRegistry::new());
        resources.insert(AccessTracker::new());
        resources.insert(Commands::new());
        resources
    }

    #[test]
    fn round_trip_save_load() {
        let doc = SceneDocument {
            name: "Test Scene".into(),
            version: "0.1.0".into(),
            entities: vec![
                EntityDescription {
                    name: "Player".into(),
                    parent: None,
                    components: vec![ComponentDescription {
                        type_name: "ome_ecs::transform::Transform".into(),
                        fields: vec![
                            ("position".into(), ReflectValue::Vec3(Vec3::new(1.0, 2.0, 3.0))),
                            (
                                "rotation".into(),
                                ReflectValue::Quat(glam::Quat::IDENTITY),
                            ),
                            ("scale".into(), ReflectValue::Vec3(Vec3::ONE)),
                        ],
                    }],
                },
                EntityDescription {
                    name: "Enemy".into(),
                    parent: None,
                    components: vec![ComponentDescription {
                        type_name: "test::Health".into(),
                        fields: vec![
                            ("hp".into(), ReflectValue::U32(42)),
                            ("max_hp".into(), ReflectValue::U32(100)),
                        ],
                    }],
                },
            ],
        };

        let dir = std::env::temp_dir().join("ome_scene_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("round_trip.ome_scene");

        doc.save(&path).unwrap();
        let loaded = SceneDocument::load(&path).unwrap();
        assert_eq!(doc, loaded);

        // Cleanup.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn from_ecs_captures_entities() {
        let mut resources = setup_resources();

        // Spawn two entities with reflected components.
        {
            let mut commands = resources.remove::<Commands>().unwrap();
            commands
                .spawn(&mut resources)
                .insert_reflected(Health { hp: 42, max_hp: 100 })
                .insert_reflected(Transform::from_position(Vec3::new(1.0, 2.0, 3.0)));
            commands
                .spawn(&mut resources)
                .insert_reflected(Health { hp: 10, max_hp: 50 });
            commands.apply(&mut resources);
            resources.insert(commands);
        }

        let doc = SceneDocument::from_ecs(&resources);

        assert_eq!(doc.entities.len(), 2);

        // First entity should have 2 components (Health + Transform).
        let e0 = &doc.entities[0];
        assert_eq!(e0.components.len(), 2);

        // Second entity should have 1 component (Health).
        let e1 = &doc.entities[1];
        assert_eq!(e1.components.len(), 1);

        // Verify Health field values on the first entity.
        let health_comp = e0
            .components
            .iter()
            .find(|c| c.type_name.contains("Health"))
            .expect("Health component not found");
        assert!(health_comp.fields.contains(&("hp".into(), ReflectValue::U32(42))));
        assert!(health_comp.fields.contains(&("max_hp".into(), ReflectValue::U32(100))));
    }

    // -- Marker component used by the ephemeral-filter tests ---------------

    /// Zero-sized marker registered as ephemeral in the tests below.
    struct TestEphemeral;
    impl Component for TestEphemeral {}

    #[test]
    fn from_ecs_skips_ephemeral_entities() {
        use crate::ephemeral::EphemeralComponents;

        let mut resources = setup_resources();
        let mut ephemeral = EphemeralComponents::new();
        ephemeral.insert(std::any::TypeId::of::<TestEphemeral>());
        resources.insert(ephemeral);

        // Spawn one persistent entity and one ephemeral entity.
        {
            let mut commands = resources.remove::<Commands>().unwrap();
            commands
                .spawn(&mut resources)
                .insert_reflected(Health { hp: 1, max_hp: 1 });
            commands
                .spawn(&mut resources)
                .insert_reflected(Health { hp: 99, max_hp: 99 })
                .insert(TestEphemeral);
            commands.apply(&mut resources);
            resources.insert(commands);
        }

        let doc = SceneDocument::from_ecs(&resources);

        // Only the non-ephemeral entity should be serialized.
        assert_eq!(doc.entities.len(), 1);
        let health = &doc.entities[0].components[0];
        assert!(health.fields.contains(&("hp".into(), ReflectValue::U32(1))));
    }

    #[test]
    fn despawn_all_preserves_ephemeral_entities() {
        use crate::ephemeral::EphemeralComponents;

        let mut resources = setup_resources();
        let mut ephemeral = EphemeralComponents::new();
        ephemeral.insert(std::any::TypeId::of::<TestEphemeral>());
        resources.insert(ephemeral);

        // Register Health so it can be looked up by name during sync.
        resources
            .get_mut::<ComponentRegistry>()
            .unwrap()
            .register_cpu_reflected::<Health>();

        // Spawn one persistent and one ephemeral entity.
        {
            let mut commands = resources.remove::<Commands>().unwrap();
            commands
                .spawn(&mut resources)
                .insert_reflected(Health { hp: 1, max_hp: 1 });
            commands
                .spawn(&mut resources)
                .insert_reflected(Health { hp: 99, max_hp: 99 })
                .insert(TestEphemeral);
            commands.apply(&mut resources);
            resources.insert(commands);
        }

        // Loading an empty scene should wipe the persistent entity but
        // keep the ephemeral one alive.
        let empty = SceneDocument {
            name: "empty".into(),
            version: "0.1.0".into(),
            entities: vec![],
        };
        sync_scene_to_ecs(&empty, &mut resources).unwrap();

        let query = Query::<&Health>::new(&resources);
        let healths: Vec<u32> = query.iter().map(|h| h.hp).collect();
        assert_eq!(healths, vec![99], "ephemeral entity must survive scene load");
    }

    #[test]
    fn sync_scene_to_ecs_rebuilds_entities() {
        let mut resources = setup_resources();

        // Register component types so they can be looked up by name.
        {
            let registry = resources.get_mut::<ComponentRegistry>().unwrap();
            registry.register_cpu_reflected::<Health>();
            registry.register_cpu_reflected::<Transform>();
        }

        let doc = SceneDocument {
            name: "Test".into(),
            version: "0.1.0".into(),
            entities: vec![EntityDescription {
                name: "Hero".into(),
                parent: None,
                components: vec![ComponentDescription {
                    type_name: std::any::type_name::<Health>().to_owned(),
                    fields: vec![
                        ("hp".into(), ReflectValue::U32(77)),
                        ("max_hp".into(), ReflectValue::U32(200)),
                    ],
                }],
            }],
        };

        sync_scene_to_ecs(&doc, &mut resources).unwrap();

        // Verify the entity exists with correct field values.
        let query = Query::<&Health>::new(&resources);
        let results: Vec<_> = query.iter().collect();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].hp, 77);
        assert_eq!(results[0].max_hp, 200);
    }

    // -- AssetRef round-trip ---------------------------------------------

    /// Component with the new typed asset reference fields. Mirrors the
    /// shape of `MeshRenderer.mesh` / `.material` without pulling
    /// `ome_render` into the test scope (would create a dep cycle).
    #[derive(Default, Clone, Debug, PartialEq, ome_ecs_macros::Reflect)]
    struct TestAssetHolder {
        #[reflect(asset = "test::FakeMesh")]
        pub mesh: Option<ome_core::Guid>,
        #[reflect(asset = "test::FakeMaterial")]
        pub material: Option<ome_core::Guid>,
    }

    impl Component for TestAssetHolder {}

    /// End-to-end: spawn an entity with both AssetRef fields populated,
    /// snapshot the world to a `SceneDocument`, save to RON, load it
    /// back, and sync into a fresh world. Both GUIDs must survive the
    /// round-trip — otherwise scenes that reference engine assets
    /// silently lose their bindings on reload.
    #[test]
    fn assetref_fields_round_trip_through_scene_save_load() {
        use ome_core::Guid;
        use std::path::PathBuf;

        let mesh_guid = Guid::new_v4();
        let material_guid = Guid::new_v4();

        // 1. Build source world + spawn one entity with both fields.
        let mut src = setup_resources();
        src.get_mut::<ComponentRegistry>()
            .unwrap()
            .register_cpu_reflected::<TestAssetHolder>();
        {
            let mut commands = src.remove::<Commands>().unwrap();
            commands
                .spawn(&mut src)
                .insert_reflected(TestAssetHolder {
                    mesh: Some(mesh_guid),
                    material: Some(material_guid),
                });
            commands.apply(&mut src);
            src.insert(commands);
        }

        // 2. Snapshot + RON round-trip via on-disk file.
        let doc = SceneDocument::from_ecs(&src);
        let tmp_dir = std::env::temp_dir();
        let scene_path: PathBuf = tmp_dir.join(format!(
            "ome_assetref_round_trip_{}.ron",
            std::process::id(),
        ));
        doc.save(&scene_path).expect("scene save");
        let reloaded = SceneDocument::load(&scene_path).expect("scene load");
        let _ = std::fs::remove_file(&scene_path);

        // 3. Sync into a fresh world.
        let mut dst = setup_resources();
        dst.get_mut::<ComponentRegistry>()
            .unwrap()
            .register_cpu_reflected::<TestAssetHolder>();
        sync_scene_to_ecs(&reloaded, &mut dst).expect("sync");

        // 4. Assert the GUIDs round-trip into the new component.
        let query = Query::<&TestAssetHolder>::new(&dst);
        let results: Vec<_> = query.iter().collect();
        assert_eq!(results.len(), 1, "exactly one entity must be reconstructed");
        assert_eq!(
            results[0].mesh,
            Some(mesh_guid),
            "mesh GUID must survive scene round-trip",
        );
        assert_eq!(
            results[0].material,
            Some(material_guid),
            "material GUID must survive scene round-trip",
        );
    }

    /// `None` AssetRefs must round-trip too — defensive against a
    /// serializer that conflates `Some(uuid)` and `None`.
    #[test]
    fn assetref_none_round_trip() {
        let mut src = setup_resources();
        src.get_mut::<ComponentRegistry>()
            .unwrap()
            .register_cpu_reflected::<TestAssetHolder>();
        {
            let mut commands = src.remove::<Commands>().unwrap();
            commands
                .spawn(&mut src)
                .insert_reflected(TestAssetHolder::default());
            commands.apply(&mut src);
            src.insert(commands);
        }

        let doc = SceneDocument::from_ecs(&src);
        // Direct in-memory round-trip via RON to avoid the temp file.
        let serialized = ron::ser::to_string(&doc).expect("serialize");
        let reloaded: SceneDocument = ron::from_str(&serialized).expect("deserialize");

        let mut dst = setup_resources();
        dst.get_mut::<ComponentRegistry>()
            .unwrap()
            .register_cpu_reflected::<TestAssetHolder>();
        sync_scene_to_ecs(&reloaded, &mut dst).expect("sync");

        let query = Query::<&TestAssetHolder>::new(&dst);
        let results: Vec<_> = query.iter().collect();
        assert_eq!(results.len(), 1);
        assert!(results[0].mesh.is_none(), "None mesh must survive round-trip");
        assert!(results[0].material.is_none(), "None material must survive round-trip");
    }
}
