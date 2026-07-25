use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::archetype_registry::ArchetypeRegistry;
use crate::component::ComponentRegistry;
use crate::dynamic_components::DynamicComponents;
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
    /// Parent's index into [`SceneDocument::entities`].
    ///
    /// An index, not a name. Entity names are not unique — a scene with five
    /// meshes called "Mesh" is normal — so resolving a parent by name
    /// collapses them all onto one key and attaches every child to whichever
    /// one happened to be inserted last. That is a hierarchy silently
    /// rebuilt wrong on load, and it is what [`Self::parent`] did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_index: Option<usize>,
    /// Parent entity name — **legacy**, read only.
    ///
    /// Kept so scenes written before `parent_index` still load. Never
    /// written any more: two ways to express the same link is two ways for
    /// them to disagree. Resolving through it is ambiguous by construction
    /// and warns when the name it names is not unique.
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
    /// stored as an index into `entities` in
    /// `EntityDescription::parent_index`.
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

        // (entity_index, entity, EntityDescription). The handle is carried
        // along so parents can be resolved *after* the sort — see below.
        let mut indexed_entities: Vec<(u32, crate::entity::Entity, EntityDescription)> = Vec::new();

        let archetypes = resources.get::<ArchetypeRegistry>();
        let components = resources.get::<ComponentRegistry>();
        let dynamic = resources.get::<DynamicComponents>();

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

                    // Write back components this binary could not
                    // resolve on load, so a scene opened by a binary
                    // that only understands half of it survives the
                    // round-trip intact. Sorted by type name: their
                    // original file order is not recoverable, and a
                    // stable order keeps re-saves diff-clean.
                    if let Some(dynamic) = &dynamic {
                        let mut parked: Vec<ComponentDescription> = dynamic
                            .iter_entity(entity)
                            .map(|(type_name, fields)| ComponentDescription {
                                type_name: type_name.to_owned(),
                                fields: fields.to_vec(),
                            })
                            .collect();
                        parked.sort_by(|a, b| a.type_name.cmp(&b.type_name));
                        comp_descs.extend(parked);
                    }

                    if comp_descs.is_empty() {
                        continue;
                    }

                    // Try to extract a display name from a "Name" component's
                    // "value" field, falling back to "Entity <index>".
                    let display_name = comp_descs
                        .iter()
                        .find(|c| c.type_name.rsplit("::").next().unwrap_or(&c.type_name) == "Name")
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

                    indexed_entities.push((
                        entity.index(),
                        entity,
                        EntityDescription {
                            name: display_name,
                            parent_index: None, // Filled in second pass.
                            parent: None,
                            components: comp_descs,
                        },
                    ));
                }
            }

            // Sort BEFORE resolving parents. `parent_index` points into the
            // emitted list, so assigning it first and sorting afterwards
            // leaves every link pointing at whatever moved into that slot —
            // silently, and only when the sort actually reorders anything.
            indexed_entities.sort_by_key(|(idx, _, _)| *idx);

            let entity_to_idx: std::collections::HashMap<crate::entity::Entity, usize> =
                indexed_entities
                    .iter()
                    .enumerate()
                    .map(|(idx, (_, entity, _))| (*entity, idx))
                    .collect();

            if let Some(parent_storage) = components.get_cpu::<Parent>() {
                for idx in 0..indexed_entities.len() {
                    let entity = indexed_entities[idx].1;
                    if let Some(parent_comp) = parent_storage.get(entity)
                        && let Some(&parent_idx) = entity_to_idx.get(&parent_comp.entity)
                    {
                        indexed_entities[idx].2.parent_index = Some(parent_idx);
                    }
                }
            }
        }

        let entities: Vec<EntityDescription> = indexed_entities
            .into_iter()
            .map(|(_, _, desc)| desc)
            .collect();

        SceneDocument {
            name: "Untitled Scene".into(),
            version: "0.1.0".into(),
            entities,
        }
    }
}
