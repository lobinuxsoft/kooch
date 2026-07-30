//! Saving an entity as a prefab, and stamping one back into the scene.
//!
//! A prefab is a scene file — see
//! [`SceneDocument::from_ecs_subtree`](ome_ecs::scene::SceneDocument::from_ecs_subtree)
//! for why there is no second format. What lives here is only the editor's
//! side of it: choosing where the file goes and what it is called.

use std::path::{Path, PathBuf};

use ome_core::resource::Resources;
use ome_ecs::entity::Entity;
use ome_ecs::scene::SceneDocument;

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
        .get::<ome_ecs::component::ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<ome_ecs::name::Name>())
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
    match ome_ecs::scene::prefab::save(&document, &path) {
        Ok(guid) => tracing::info!("prefab saved to {} as {guid}", path.display()),
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
    prefab: ome_core::Guid,
    at: crate::viewport_pick::DropPoint,
) {
    // Resolved before the spawn: it reads the camera, and the borrow ends
    // before the world is mutated.
    let at = crate::viewport_pick::resolve(resources, at);

    let root = match ome_ecs::scene::spawn_prefab(prefab, resources) {
        Ok(root) => root,
        Err(e) => {
            tracing::error!("failed to instance prefab {prefab}: {e}");
            return;
        }
    };
    // A drop into the viewport names a place; the World panel and the
    // context menu do not, and leave the prefab where it was authored.
    if let Some(at) = at
        && let Some(registry) = resources.get_mut::<ome_ecs::component::ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<ome_ecs::transform::Transform>()
        && let Some(transform) = storage.get_mut(root)
    {
        transform.position = at;
    }
    tracing::info!("instanced prefab {prefab}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_becomes_a_filename() {
        assert_eq!(sanitize_file_stem("Ball"), "Ball");
        assert_eq!(sanitize_file_stem("Player (spare)"), "Player_spare");
    }

    /// A slash would write outside the folder that was chosen, which is
    /// the whole reason this function exists.
    #[test]
    fn a_path_cannot_be_smuggled_through_a_name() {
        assert_eq!(sanitize_file_stem("enemy/variant"), "enemy_variant");
        assert_eq!(sanitize_file_stem("../../etc/passwd"), "etc_passwd");
    }

    #[test]
    fn a_name_with_nothing_usable_in_it_still_gets_a_file() {
        assert_eq!(sanitize_file_stem(""), "Prefab");
        assert_eq!(sanitize_file_stem("///"), "Prefab");
        assert_eq!(sanitize_file_stem("..."), "Prefab");
    }

    #[test]
    fn without_a_drop_folder_a_prefab_goes_to_assets() {
        let root = Path::new("/p");
        assert_eq!(
            prefab_path(root, "Ball", None),
            PathBuf::from("/p/assets/Ball.prefab"),
        );
    }

    /// A folder outside the project is not somewhere the catalog scans, so
    /// the file would be written and never appear.
    #[test]
    fn a_destination_outside_the_project_falls_back_to_assets() {
        let root = Path::new("/p");
        assert_eq!(
            prefab_path(root, "Ball", Some(Path::new("/somewhere/else"))),
            PathBuf::from("/p/assets/Ball.prefab"),
        );
    }

    #[test]
    fn a_drop_folder_inside_the_project_is_used() {
        let root = Path::new("/p");
        assert_eq!(
            prefab_path(root, "Ball", Some(Path::new("/p/assets/enemies"))),
            PathBuf::from("/p/assets/enemies/Ball.prefab"),
        );
    }

    /// The same entity resolves to the same file every time, which is what
    /// makes re-saving update a prefab rather than litter the folder. The
    /// prompt is what keeps that from being destructive.
    ///
    /// `std::env::temp_dir` rather than a crate — there is no `tempfile` in
    /// this workspace, and the rest of the editor's file tests do the same.
    #[test]
    fn re_saving_resolves_to_the_same_file() {
        let root = std::env::temp_dir().join("ome_prefab_name_clash");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("assets")).unwrap();

        let first = prefab_path(&root, "Enemy", None);
        std::fs::write(&first, "").unwrap();
        assert_eq!(
            prefab_path(&root, "Enemy", None),
            first,
            "an existing file must resolve to itself, not to a new name",
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

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
    prefab: ome_core::Guid,
    edit: impl FnOnce(&mut SceneDocument) -> R,
) -> Option<R> {
    let mut server = resources.remove::<ome_core::asset_loader::AssetServer>()?;
    let handle = server.load_by_guid::<SceneDocument>(prefab, resources).ok();
    resources.insert(server);

    let handle = handle?;
    let assets = resources.get_mut::<ome_core::assets::Assets<SceneDocument>>()?;
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
    prefab: ome_core::Guid,
    entity_index: usize,
    component: &str,
    field: &str,
    value: ome_ecs::reflect::ReflectValue,
) {
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
    prefab: ome_core::Guid,
    entity_index: usize,
    component: ome_ecs::component::ComponentId,
    add: bool,
) {
    // The document stores a type name, the menu speaks `ComponentId`, and
    // the registry is the only thing that knows both.
    let Some(type_name) = resources
        .get::<ome_ecs::component::ComponentNames>()
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
                        .push(ome_ecs::scene::ComponentDescription {
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
fn default_fields(
    resources: &Resources,
    type_name: &str,
) -> Option<Vec<(String, ome_ecs::reflect::ReflectValue)>> {
    let registry = resources.get::<ome_ecs::component::ComponentRegistry>()?;
    let type_id = registry.type_id_by_name(type_name)?;
    registry.reflect_default_fields(&type_id)
}

/// Writes a prefab's edited document back to its file.
pub(super) fn handle_save_prefab_asset(resources: &mut Resources, prefab: ome_core::Guid) {
    let Some(path) = resources
        .get::<ome_core::asset_database::AssetDatabase>()
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
    match ome_ecs::scene::prefab::save(&document, &path) {
        Ok(_) => {
            if let Some(dirty) = resources.get_mut::<crate::actions::DirtyPrefabs>() {
                dirty.clear(prefab);
            }
            tracing::info!("prefab saved to {}", path.display());
        }
        Err(e) => tracing::error!("failed to save prefab: {e}"),
    }
}
