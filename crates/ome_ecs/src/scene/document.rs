use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::archetype_registry::ArchetypeRegistry;
use crate::component::ComponentRegistry;
use crate::dynamic_components::DynamicComponents;
use crate::reflect::ReflectValue;
use ome_core::Guid;
use ome_core::resource::Resources;

use super::error::SceneError;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Top-level scene container, serialized as RON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SceneDocument {
    /// Identity of this scene, stable across sessions.
    ///
    /// This is what [`EntityRef::Persistent`](crate::reflect::EntityRef) has
    /// addressed since #607: a reference leaving its own file names the
    /// scene it points into. A path could not serve — moving or renaming a
    /// file would break every reference into it — for the same reason an
    /// asset is addressed by [`Guid`] rather than by where it happens to
    /// sit on disk.
    ///
    /// Files written before scenes had identity get one assigned on load;
    /// [`SceneManager`](crate::scene_manager::SceneManager) marks them dirty
    /// so it persists on the next save.
    #[serde(default = "Guid::new_v4")]
    pub id: Guid,
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

/// The last segment of a full type path.
///
/// Component descriptions store the full path, which differs between a
/// type moving crates and the same type staying put; the tail is what
/// stays stable and is already what the capture pass matches `Name` on.
fn short_name(type_name: &str) -> &str {
    type_name.rsplit("::").next().unwrap_or(type_name)
}

/// What a capture takes out of the live world.
///
/// Three answers to one question, named rather than encoded as an
/// `Option<Guid>` that meant "one scene, or else everything". A prefab
/// is a third answer, and bolting it on as a second flag would let the
/// two disagree about what "everything" excludes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Capture {
    /// The whole world. What Play and the remote protocol mirror — they
    /// reflect what is running, not what one file owns.
    Everything,
    /// One scene's members. With several scenes open, saving one must not
    /// drag in another's entities.
    Scene(Guid),
    /// One entity and everything under it — a prefab (#611).
    Subtree(crate::entity::Entity),
}

// ---------------------------------------------------------------------------
// SceneDocument methods
// ---------------------------------------------------------------------------

impl SceneDocument {
    /// Saves the scene as pretty-printed RON to `path`.
    ///
    /// Refuses a document still holding a live entity handle — see
    /// [`Self::live_reference`].
    pub fn save(&self, path: &Path) -> Result<(), SceneError> {
        if let Some(error) = self.live_reference() {
            return Err(error);
        }
        let config = ron::ser::PrettyConfig::default()
            .struct_names(false)
            .enumerate_arrays(false);
        let data = ron::ser::to_string_pretty(self, config)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    /// The first field still holding a [`EntityRef::Live`], if any.
    ///
    /// An index and a generation are meaningless once reloaded, so writing
    /// one produces a scene whose references point at whatever occupies
    /// those slots next time. `entity_refs::to_persistent` resolves every
    /// reference on the way into the document, so anything left here means
    /// a document that did not come through it.
    ///
    /// The check lives at the file boundary rather than in
    /// [`EntityRef`]'s serialiser because the editor protocol carries the
    /// same values legitimately — and because a document knows *which*
    /// entity, component and field, where a serialiser sees a bare pair of
    /// numbers.
    ///
    /// [`EntityRef::Live`]: crate::reflect::EntityRef::Live
    fn live_reference(&self) -> Option<SceneError> {
        for entity in &self.entities {
            for component in &entity.components {
                for (field, value) in &component.fields {
                    if let ReflectValue::EntityRef(Some(reference)) = value
                        && reference.entity().is_some()
                    {
                        return Some(SceneError::UnresolvedReference {
                            entity: entity.name.clone(),
                            component: component.type_name.clone(),
                            field: field.clone(),
                        });
                    }
                }
            }
        }
        None
    }

    /// The index of the one entity with no parent inside this document.
    ///
    /// Every non-root of a captured subtree carries a `Parent` pointing
    /// inside the file, so the root is what is left. Returns
    /// [`SceneError::NotASingleRoot`] for zero or several — see there for
    /// why instancing needs exactly one.
    pub fn root_index(&self) -> Result<usize, SceneError> {
        let mut roots = self.entities.iter().enumerate().filter(|(_, entity)| {
            !entity
                .components
                .iter()
                .any(|component| short_name(&component.type_name) == "Parent")
        });
        match (roots.next(), roots.next()) {
            (Some((index, _)), None) => Ok(index),
            (first, second) => Err(SceneError::NotASingleRoot {
                roots: first.is_some() as usize + second.is_some() as usize + roots.count(),
            }),
        }
    }

    /// A copy of this document ready to be spawned as an instance living
    /// inside the scene `into`.
    ///
    /// # What has to change, and what deliberately does not
    ///
    /// An [`EntityGuid`] is unique within its scene, not globally — which
    /// is what makes instancing possible at all. Stamp the same prefab out
    /// twice without remapping and both copies claim to be "entity 4", so
    /// a reference to one resolves to whichever was loaded last.
    ///
    /// Only the **ids** are remapped. Internal references are already
    /// written as `scene: None`, meaning "the same scene as the reference
    /// itself" — a shape that exists precisely so a scene can be relocated
    /// without rewriting every reference in it. References that name
    /// another scene are left alone: they point outside this prefab and
    /// remapping them would break them.
    ///
    /// The copy takes `into` as its own id, which is what tags the spawned
    /// entities as members of the scene that now contains them. A Phase A
    /// instance is *baked* into that scene: it keeps no link back to the
    /// file it came from, so editing the prefab later does not update it
    /// (#611 Phase B).
    pub fn as_instance_of(
        &self,
        into: Guid,
        allocator: &mut crate::persistent_id::PersistentIdAllocator,
    ) -> Self {
        use crate::persistent_id::EntityGuid;
        use std::collections::HashMap;

        // Every id the file mentions, mapped once, so two fields pointing
        // at the same entity still point at the same entity afterwards.
        let mut remap: HashMap<EntityGuid, EntityGuid> = HashMap::new();
        let mut fresh = |old: EntityGuid, allocator: &mut _| -> EntityGuid {
            *remap
                .entry(old)
                .or_insert_with(|| crate::persistent_id::PersistentIdAllocator::allocate(allocator))
        };

        // Two passes: identity first, so a reference reaching a field
        // before its target's `PersistentId` still lands on the same new
        // id rather than allocating a second one.
        let mut entities = self.entities.clone();
        for entity in &mut entities {
            for component in &mut entity.components {
                if short_name(&component.type_name) != "PersistentId" {
                    continue;
                }
                for (name, value) in &mut component.fields {
                    // Reflected as a bare `u64`, so the component it
                    // belongs to is what identifies it. Zero is the
                    // `EntityGuid` niche rather than an id, and a file
                    // carrying one is corrupt — left alone here so the load
                    // path is the one place that rejects it.
                    if name == "id"
                        && let ReflectValue::U64(raw) = value
                        && let Some(old) = EntityGuid::new(*raw)
                    {
                        *value = ReflectValue::U64(fresh(old, allocator).get());
                    }
                }
            }
        }
        for entity in &mut entities {
            for component in &mut entity.components {
                for (_, value) in &mut component.fields {
                    let ReflectValue::EntityRef(Some(reference)) = value else {
                        continue;
                    };
                    if let crate::reflect::EntityRef::Persistent { scene, id } = reference {
                        // `None` is an internal reference; `Some(self.id)`
                        // is the same thing spelled out. Anything else
                        // points outside the prefab.
                        let internal = scene.is_none() || *scene == Some(self.id);
                        if internal {
                            *scene = None;
                            *id = fresh(*id, allocator);
                        }
                    }
                }
            }
        }

        Self {
            id: into,
            name: self.name.clone(),
            version: self.version.clone(),
            entities,
        }
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
    pub fn from_ecs(resources: &mut Resources) -> Self {
        Self::capture(resources, Capture::Everything, Guid::new_v4())
    }

    /// Snapshots only the entities belonging to `scene`.
    ///
    /// With several scenes open, saving one must not drag in another's
    /// entities — that would duplicate them into both files and make the
    /// next load spawn each twice.
    pub fn from_ecs_scene(resources: &mut Resources, scene: Guid) -> Self {
        Self::capture(resources, Capture::Scene(scene), scene)
    }

    /// Snapshots one entity and its descendants as a standalone scene — a
    /// prefab.
    ///
    /// # Why this is a scene and not a new format
    ///
    /// A prefab *is* a serialised scene; Unity's is, and Godot makes it
    /// explicit with `PackedScene`. A second format would mean a second
    /// serialiser to keep in step with this one, and the one that is not
    /// exercised by every save is the one that drifts (#611).
    ///
    /// # What the capture drops
    ///
    /// The root's [`Parent`](crate::hierarchy::Parent) — it points at
    /// whatever the entity happened to be attached to while being
    /// authored, which is not part of the prefab. Keeping it would leave
    /// the file holding a reference to an entity the file does not
    /// contain, and every instance would try to resolve it.
    ///
    /// The document takes a fresh [`Guid`]: it is a new scene, not another
    /// view of the one it was captured from.
    pub fn from_ecs_subtree(resources: &mut Resources, root: crate::entity::Entity) -> Self {
        Self::capture(resources, Capture::Subtree(root), Guid::new_v4())
    }

    /// Shared body of every `from_ecs*` constructor; see [`Capture`] for
    /// what each one takes.
    fn capture(resources: &mut Resources, what: Capture, id: Guid) -> Self {
        // Saving assigns identity: `PersistentId` is opt-in, and whether
        // something is referenced is only known once references are
        // written. See `scene::entity_refs`.
        let ids = super::entity_refs::assign_ids_to_referenced(resources);
        let resources: &Resources = resources;
        use crate::ephemeral::EphemeralComponents;
        use crate::hierarchy::{Children, GlobalTransform};

        // Children and GlobalTransform are derived from Parent and the
        // transform hierarchy, so saving them would store the same fact
        // twice and let the copies disagree. `Parent` itself is now an
        // ordinary component carrying an entity reference.
        let skip_types = [
            std::any::TypeId::of::<Children>(),
            std::any::TypeId::of::<GlobalTransform>(),
        ];
        let parent_tid = std::any::TypeId::of::<crate::hierarchy::Parent>();
        let instance_tid = std::any::TypeId::of::<crate::prefab_instance::PrefabInstance>();
        let member_tid_prefab = std::any::TypeId::of::<crate::prefab_instance::PrefabMember>();

        // A subtree's membership is resolved once, up front: the walk below
        // visits archetypes in whatever order they were created, so
        // "is this entity under the root" cannot be answered as it goes.
        let subtree = match what {
            Capture::Subtree(root) => resources
                .get::<ComponentRegistry>()
                .map(|components| {
                    crate::hierarchy::collect_descendants(root, components)
                        .into_iter()
                        .collect::<std::collections::HashSet<_>>()
                })
                .unwrap_or_default(),
            _ => std::collections::HashSet::new(),
        };

        // Membership is derived on load, never written — see
        // `SceneMember`. It is read here only to decide what belongs.
        let member_tid = std::any::TypeId::of::<crate::scene_member::SceneMember>();

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
                    // A prefab instance is written as a *reference* — the
                    // link and what the user changed — not as the entities
                    // it built. Those come back from the prefab on load.
                    //
                    // This is the whole point of the model: a value held in
                    // two places drifts, and every prefab bug found while
                    // building this was that. Storing it once removes the
                    // class rather than propagating around it (#611).
                    let membership = components
                        .get_cpu::<crate::prefab_instance::PrefabMember>()
                        .and_then(|storage| storage.get(entity))
                        .map(|member| member.root);
                    // Only the root survives into the file; the rest of the
                    // instance is not the scene's to describe.
                    if membership.is_some_and(|root| root != entity) {
                        continue;
                    }
                    let is_instance_root = membership == Some(entity);

                    // Restrict the walk to what was asked for.
                    let belongs = match what {
                        Capture::Everything => true,
                        Capture::Scene(only) => components
                            .get_cpu::<crate::scene_member::SceneMember>()
                            .and_then(|storage| storage.get(entity))
                            .is_some_and(|member| member.scene == only),
                        Capture::Subtree(_) => subtree.contains(&entity),
                    };
                    if !belongs {
                        continue;
                    }

                    let mut comp_descs = Vec::new();

                    for &type_id in archetype.components() {
                        // Skip hierarchy components and membership.
                        if skip_types.contains(&type_id) || type_id == member_tid {
                            continue;
                        }

                        // The prefab's root has no parent inside the file —
                        // see `from_ecs_subtree`.
                        if type_id == parent_tid && what == Capture::Subtree(entity) {
                            continue;
                        }

                        // An instance root writes what makes it an instance
                        // and where it sits, and nothing else: its
                        // components are the prefab's, and any that differ
                        // are already in the override list.
                        //
                        // `PrefabMember` is rebuilt by `attach` on load, so
                        // writing it would be storing a fact twice — the
                        // mistake this whole change exists to stop making.
                        if is_instance_root && type_id != instance_tid && type_id != parent_tid {
                            continue;
                        }
                        if type_id == member_tid_prefab {
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

                        // Live handles mean nothing after a reload; swap
                        // them for the persistent ids assigned above.
                        let fields = fields
                            .into_iter()
                            .map(|(name, value)| {
                                (name, super::entity_refs::to_persistent(value, &ids))
                            })
                            .collect();

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

            // Sorted for a stable file order; nothing depends on the
            // positions any more. `parent_index` used to point into this
            // list, which made the sort load-bearing — see #607.
            indexed_entities.sort_by_key(|(idx, _, _)| *idx);
        }

        // A prefab is named after the entity it was captured from, so the
        // file and the thing inside it say the same name.
        let name = match what {
            Capture::Subtree(root) => indexed_entities
                .iter()
                .find(|(_, entity, _)| *entity == root)
                .map(|(_, _, desc)| desc.name.clone())
                .unwrap_or_else(|| "Untitled Scene".into()),
            _ => "Untitled Scene".into(),
        };

        let entities: Vec<EntityDescription> = indexed_entities
            .into_iter()
            .map(|(_, _, desc)| desc)
            .collect();

        SceneDocument {
            id,
            name,
            version: "0.1.0".into(),
            entities,
        }
    }
}
