use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::archetype_registry::ArchetypeRegistry;
use crate::component::ComponentRegistry;
use crate::reflect::ReflectValue;
use ome_core::resource::Resources;

use super::error::SceneError;

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
