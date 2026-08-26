//! Saving an entity as a prefab, and stamping one back into the scene.
//!
//! A prefab is a scene file — see
//! [`SceneDocument::from_ecs_subtree`](kooch_ecs::scene::SceneDocument::from_ecs_subtree)
//! for why there is no second format. What lives here is only the editor's
//! side of it: choosing where the file goes and what it is called.

use std::path::{Path, PathBuf};

use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;
use kooch_ecs::scene::SceneDocument;

use crate::project::PREFAB_EXTENSION;
use crate::project_state::ProjectState;

/// The root of the project currently open.
pub(crate) fn project_root(resources: &Resources) -> Option<PathBuf> {
    resources
        .get::<ProjectState>()?
        .active_project
        .as_ref()
        .map(|project| project.root_path.clone())
}

/// The name to call an entity's prefab file.
///
/// Read from the entity's `Name`, falling back to its index — the same
/// fallback the World panel shows, so the file matches the row the user
/// right-clicked.
pub(crate) fn entity_name(resources: &Resources, entity: Entity) -> String {
    resources
        .get::<kooch_ecs::component::ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<kooch_ecs::name::Name>())
        .and_then(|storage| storage.get(entity))
        .map(|name| name.value.clone())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("Entity {}", entity.index()))
}

/// Turns an entity's display name into something a filesystem accepts.
///
/// Entity names are free text — "Player (spare)", "enemy/variant b" — and
/// a slash in one would silently write outside the folder that was chosen.
/// Kept deliberately narrow rather than trying to preserve as much as
/// possible: the file is renameable afterwards, so a conservative
/// starting point costs the user nothing.
pub(crate) fn sanitize_file_stem(name: &str) -> String {
    // Runs collapse: "Player (spare)" replaces three characters in a row
    // and would otherwise come out "Player__spare".
    let mut cleaned = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            c if c.is_alphanumeric() || c == '-' => cleaned.push(c),
            _ if cleaned.ends_with('_') => {}
            _ => cleaned.push('_'),
        }
    }
    // Leading dots would make it a hidden file, and a name of nothing but
    // separators leaves nothing to call it.
    let trimmed = cleaned.trim_matches(['_', '-', '.']);
    match trimmed.is_empty() {
        true => "Prefab".to_owned(),
        false => trimmed.to_owned(),
    }
}

/// Where a prefab named `name` gets written.
///
/// `dest` is the folder a drag was dropped on; without one the project's
/// `assets/` is used, which is where the user asked prefabs to live.
///
/// The same entity always resolves to the same file, so saving a prefab
/// again after editing the entity updates it. It used to suffix — `Enemy`,
/// `Enemy_1`, `Enemy_2` — which never destroyed anything and made
/// iterating on a prefab impossible. Replacement is guarded by a
/// confirmation prompt instead; see `actions::intercept_prefab_overwrites`.
pub(crate) fn prefab_path(root: &Path, name: &str, dest: Option<&Path>) -> PathBuf {
    let folder = match dest {
        // A folder outside the project would put the file somewhere the
        // asset catalog does not scan, so it would never appear.
        Some(dest) if dest.starts_with(root) => dest.to_path_buf(),
        _ => root.join("assets"),
    };
    folder.join(format!("{}.{PREFAB_EXTENSION}", sanitize_file_stem(name)))
}

/// What every write of an asset file goes through, whichever panel or
/// action did the writing.
///
/// Three things have to happen and none of them are optional:
///
/// 1. **The database learns the file exists.** The project's asset scan
///    only runs when the active project changes, so a file created
///    mid-session is invisible until the editor restarts: its `.meta`
///    gives it a guid, which is why it can be selected, but the catalog
///    the Inspector looks it up in has never heard of it. The result was
///    an asset you could click and an Inspector that showed nothing.
/// 2. **What is already loaded stops being the old bytes.** Overwritten
///    in place, so the handles components are holding stay valid — see
///    [`kooch_core::asset_loader::asset_written`].
/// 3. **The project is told.** It has its own copy of both, in another
///    process, and it did not write this file.
///
/// Doing the one file that changed, at the moment it changes, is narrower
/// than rescanning the tree and costs nothing per frame.
pub(crate) fn asset_saved(resources: &mut Resources, path: &Path) {
    let written = kooch_core::asset_loader::asset_written(path, resources);
    if written.guid.is_none() {
        tracing::warn!("no asset identity beside {}", path.display());
    }
    announce_to_host(resources, path);
}

/// The part of saving a prefab that is *about* prefabs, after
/// [`asset_saved`] has dealt with the file.
///
/// A prefab has instances, which no other asset does: the document in
/// memory is now ahead of every entity stamped from it, and those have to
/// catch up. Nothing here re-reads the file — that already happened.
///
/// Queued rather than applied: this runs in the middle of handling an
/// action, and the writes have to travel the same way an edit does.
pub(crate) fn prefab_saved(resources: &mut Resources, path: &Path) {
    let Ok(meta) = kooch_core::asset_meta::read_meta(path) else {
        return;
    };
    // The file and the cache agree again, so there is nothing outstanding.
    if let Some(dirty) = resources.get_mut::<crate::actions::DirtyPrefabs>() {
        dirty.clear(meta.guid);
    }
    crate::actions::prefab_propagate::queue(resources, meta.guid);
}

/// Queues the message that tells the project its cached copy is stale.
///
/// The project caches the documents it instances from, and the editor is
/// what writes those files. Without this the project keeps rebuilding
/// instances from the version it read first — so a component removed from
/// a prefab came back the next time the scene loaded.
fn announce_to_host(resources: &mut Resources, path: &Path) {
    if resources.get::<PendingHostReloads>().is_none() {
        resources.insert(PendingHostReloads::default());
    }
    if let Some(pending) = resources.get_mut::<PendingHostReloads>() {
        // Telling the project twice about the same file in one frame costs
        // a synchronous round trip and re-reads the same bytes. The list
        // is short enough that scanning it beats keeping a set beside it.
        if pending.0.iter().any(|queued| queued == path) {
            return;
        }
        pending.0.push(path.to_path_buf());
    }
}

/// Prefab files the project has not been told about yet.
#[derive(Default)]
pub(crate) struct PendingHostReloads(pub(crate) Vec<std::path::PathBuf>);

/// Writes `entity` and its descendants to a prefab file.
///
/// The local path — used when the editor is driving its own world. With a
/// project connected this goes over the wire instead, because the world
/// being captured is the project's; see `remote_edit`.
pub(super) fn handle_save_prefab(resources: &mut Resources, entity: Entity, dest: Option<&Path>) {
    let Some(root) = project_root(resources) else {
        tracing::error!("cannot save a prefab without a project open");
        return;
    };
    let path = prefab_path(&root, &entity_name(resources, entity), dest);
    let document = SceneDocument::from_ecs_subtree(resources, entity);
    // The extension promises one root; checked here so the promise holds
    // for everything on disk rather than being discovered at the click
    // that instances it. A subtree has one by construction — this catches
    // an entity with nothing capturable on it, which yields no root at all.
    if let Err(e) = document.root_index() {
        tracing::error!("refusing to write a prefab that cannot be instanced: {e}");
        return;
    }
    // `prefab::save`, not `document.save`: it also writes the `.meta` that
    // makes the file a registered asset. Without one it is invisible to the
    // picker and cannot be spawned.
    match kooch_ecs::scene::prefab::save(&document, &path) {
        Ok(guid) => {
            asset_saved(resources, &path);
            prefab_saved(resources, &path);
            tracing::info!("prefab saved to {} as {guid}", path.display());
        }
        Err(e) => tracing::error!("failed to save prefab: {e}"),
    }
}

/// Stamps a prefab into the open scene, optionally placing its root.
///
/// Goes through `spawn_prefab`, which is the same entry point a game's own
/// spawner uses — so the editor exercises the runtime path rather than a
/// parallel one that could rot.
pub(super) fn handle_instantiate_prefab(
    resources: &mut Resources,
    prefab: kooch_core::Guid,
    at: crate::viewport_pick::DropPoint,
) {
    // Resolved before the spawn: it reads the camera, and the borrow ends
    // before the world is mutated.
    let at = crate::viewport_pick::resolve(resources, at);

    let (root, members) = match kooch_ecs::scene::spawn_prefab_members(prefab, resources) {
        Ok(spawned) => spawned,
        Err(e) => {
            tracing::error!("failed to instance prefab {prefab}: {e}");
            return;
        }
    };
    // Placing a prefab in the editor links the instance to it; spawning
    // one from a game does not. Same spawn, different intent.
    kooch_ecs::prefab_instance::attach(resources, root, &members, prefab);
    // A drop into the viewport names a place; the World panel and the
    // context menu do not, and leave the prefab where it was authored.
    if let Some(at) = at
        && let Some(registry) = resources.get_mut::<kooch_ecs::component::ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<kooch_ecs::transform::Transform>()
        && let Some(transform) = storage.get_mut(root)
    {
        transform.position = at;
    }
    tracing::info!("instanced prefab {prefab}");
}

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// Editing a prefab as a document
// ---------------------------------------------------------------------------

/// Runs `edit` against the cached document for `prefab` and marks it dirty.
///
/// Edits land in `Assets<SceneDocument>` rather than on disk, so they are
/// live for anything spawning the prefab and the file is behind until the
/// user saves. That is the whole shape of an explicit save; the Inspector's
/// button is what makes it visible.
fn with_cached_document<R>(
    resources: &mut Resources,
    prefab: kooch_core::Guid,
    edit: impl FnOnce(&mut SceneDocument) -> R,
) -> Option<R> {
    let mut server = resources.remove::<kooch_core::asset_loader::AssetServer>()?;
    let handle = server.load_by_guid::<SceneDocument>(prefab, resources).ok();
    resources.insert(server);

    let handle = handle?;
    let assets = resources.get_mut::<kooch_core::assets::Assets<SceneDocument>>()?;
    let result = edit(assets.get_mut(handle)?);

    // Created on first edit rather than at startup: a session that never
    // touches a prefab never grows the set.
    if resources.get::<crate::actions::DirtyPrefabs>().is_none() {
        resources.insert(crate::actions::DirtyPrefabs::default());
    }
    if let Some(dirty) = resources.get_mut::<crate::actions::DirtyPrefabs>() {
        dirty.mark(prefab);
    }
    Some(result)
}

pub(super) fn handle_edit_prefab_field(
    resources: &mut Resources,
    prefab: kooch_core::Guid,
    entity_index: usize,
    component: &str,
    field: &str,
    value: kooch_ecs::reflect::ReflectValue,
) {
    // The document as it stands, before this write. Merged by field, so
    // typing into a prefab's name is one step rather than one per letter.
    crate::history::documents::record(
        resources,
        &crate::history::Document::Prefab(prefab),
        &format!("Set {field}"),
        Some(crate::history::MergeKey::of((
            prefab,
            entity_index,
            component,
            field,
        ))),
    );
    let applied = with_cached_document(resources, prefab, |document| {
        let Some(entity) = document.entities.get_mut(entity_index) else {
            return false;
        };
        let Some(component) = entity
            .components
            .iter_mut()
            .find(|c| c.type_name == component)
        else {
            return false;
        };
        // Replaces in place rather than appending: a field written twice
        // would round-trip as two entries and the loader would take
        // whichever came last.
        match component.fields.iter_mut().find(|(name, _)| name == field) {
            Some(slot) => slot.1 = value,
            None => component.fields.push((field.to_owned(), value)),
        }
        true
    });
    if applied != Some(true) {
        tracing::warn!(
            "prefab edit did not land: {prefab} entity {entity_index} {component}.{field}"
        );
    }
}

pub(super) fn handle_edit_prefab_component(
    resources: &mut Resources,
    prefab: kooch_core::Guid,
    entity_index: usize,
    component: kooch_ecs::component::ComponentId,
    add: bool,
) {
    // No merge key: adding a component is a discrete thing, and two of
    // them in a row are two steps however fast they were clicked.
    crate::history::documents::record(
        resources,
        &crate::history::Document::Prefab(prefab),
        match add {
            true => "Add Component",
            false => "Remove Component",
        },
        None,
    );
    // The document stores a type name, the menu speaks `ComponentId`, and
    // the registry is the only thing that knows both.
    let Some(type_name) = resources
        .get::<kooch_ecs::component::ComponentNames>()
        .and_then(|names| names.name(component).map(str::to_owned))
    else {
        tracing::warn!("no type name for component {component:?}");
        return;
    };
    let defaults = match add {
        true => match default_fields(resources, &type_name) {
            Some(fields) => fields,
            None => {
                tracing::warn!("no default value for {type_name}; not added");
                return;
            }
        },
        false => Vec::new(),
    };

    with_cached_document(resources, prefab, |document| {
        let Some(entity) = document.entities.get_mut(entity_index) else {
            return;
        };
        match add {
            true => {
                if !entity.components.iter().any(|c| c.type_name == type_name) {
                    entity
                        .components
                        .push(kooch_ecs::scene::ComponentDescription {
                            type_name,
                            fields: defaults,
                        });
                }
            }
            false => entity.components.retain(|c| c.type_name != type_name),
        }
    });
}

/// The fields a freshly-constructed component would have.
///
/// Built from the type's own `Default` rather than from field *kinds*: a
/// component whose default sets `visible: true` must arrive that way, and a
/// zero-per-kind table would silently disagree with what spawning the same
/// component in the World gives.
/// What a freshly added component of `type_name` should hold.
///
/// Two sources, and both are needed. The reflected registry knows the
/// types the editor itself compiled. A project's component was compiled
/// into its dylib, so the editor cannot call `Default` on it and knows it
/// only through [`DynamicTypeRegistry`], which now carries the values the
/// plugin read off its own `Default` when it declared the type.
///
/// Asking only the first is why adding a project component to a prefab
/// answered "no default value; not added" — the fourth time a panel has
/// asked one of these registries when the answer lived in the other
/// (#722 was the third).
///
/// [`DynamicTypeRegistry`]: kooch_ecs::component::DynamicTypeRegistry
fn default_fields(
    resources: &Resources,
    type_name: &str,
) -> Option<Vec<(String, kooch_ecs::reflect::ReflectValue)>> {
    let reflected = resources
        .get::<kooch_ecs::component::ComponentRegistry>()
        .and_then(|registry| {
            let type_id = registry.type_id_by_name(type_name)?;
            registry.reflect_default_fields(&type_id)
        });
    if let Some(fields) = reflected {
        return Some(fields);
    }
    // A marker component has no fields, so an empty list is a real
    // answer here rather than a miss — `Player` is a valid thing to add.
    resources
        .get::<kooch_ecs::component::DynamicTypeRegistry>()
        .and_then(|types| types.get(type_name))
        .map(|ty| ty.defaults.clone())
}

/// Writes a prefab's edited document back to its file.
pub(super) fn handle_save_prefab_asset(resources: &mut Resources, prefab: kooch_core::Guid) {
    let Some(path) = resources
        .get::<kooch_core::asset_database::AssetDatabase>()
        .and_then(|db| db.entry(prefab))
        .map(|entry| entry.path.clone())
    else {
        tracing::error!("prefab {prefab} is not registered; nothing to save to");
        return;
    };
    let Some(document) = with_cached_document(resources, prefab, |document| document.clone())
    else {
        tracing::error!("prefab {prefab} has no cached document to save");
        return;
    };
    match kooch_ecs::scene::prefab::save(&document, &path) {
        Ok(_) => {
            if let Some(dirty) = resources.get_mut::<crate::actions::DirtyPrefabs>() {
                dirty.clear(prefab);
            }
            crate::actions::prefab_propagate::queue(resources, prefab);
            announce_to_host(resources, &path);
            tracing::info!("prefab saved to {}", path.display());
        }
        Err(e) => tracing::error!("failed to save prefab: {e}"),
    }
}
