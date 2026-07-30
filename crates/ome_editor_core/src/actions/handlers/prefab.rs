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

use crate::project::SCENE_EXTENSION;
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
/// A name already taken gets a numeric suffix. Saving a prefab must never
/// overwrite a file the user did not name — two entities called "Enemy" is
/// ordinary, and the second one silently replacing the first would be data
/// loss triggered by a menu item that reads as additive.
pub(crate) fn prefab_path(root: &Path, name: &str, dest: Option<&Path>) -> PathBuf {
    let folder = match dest {
        // A folder outside the project would put the file somewhere the
        // asset catalog does not scan, so it would never appear.
        Some(dest) if dest.starts_with(root) => dest.to_path_buf(),
        _ => root.join("assets"),
    };
    let stem = sanitize_file_stem(name);

    let first = folder.join(format!("{stem}.{SCENE_EXTENSION}"));
    if !first.exists() {
        return first;
    }
    // Bounded rather than `loop`: a thousand identically named prefabs in
    // one folder is a different problem, and an unbounded search on a
    // filesystem that is answering "yes" to everything would hang the
    // editor.
    for suffix in 1..1000 {
        let candidate = folder.join(format!("{stem}_{suffix}.{SCENE_EXTENSION}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    first
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
    match SceneDocument::from_ecs_subtree(resources, entity).save(&path) {
        Ok(()) => tracing::info!("prefab saved to {}", path.display()),
        Err(e) => tracing::error!("failed to save prefab: {e}"),
    }
}

/// Stamps a prefab file into the open scene.
pub(super) fn handle_instantiate_prefab(resources: &mut Resources, path: &Path) {
    let prefab = match SceneDocument::load(path) {
        Ok(prefab) => prefab,
        Err(e) => {
            tracing::error!("failed to read prefab {}: {e}", path.display());
            return;
        }
    };
    let into = resources
        .get::<ome_ecs::SceneManager>()
        .and_then(|scenes| scenes.active_id())
        .unwrap_or_else(ome_core::Guid::new_v4);

    match ome_ecs::scene::instantiate(&prefab, resources, into) {
        Ok(_) => tracing::info!("instanced {}", path.display()),
        Err(e) => tracing::error!("failed to instance {}: {e}", path.display()),
    }
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
            PathBuf::from("/p/assets/Ball.ome_scene"),
        );
    }

    /// A folder outside the project is not somewhere the catalog scans, so
    /// the file would be written and never appear.
    #[test]
    fn a_destination_outside_the_project_falls_back_to_assets() {
        let root = Path::new("/p");
        assert_eq!(
            prefab_path(root, "Ball", Some(Path::new("/somewhere/else"))),
            PathBuf::from("/p/assets/Ball.ome_scene"),
        );
    }

    #[test]
    fn a_drop_folder_inside_the_project_is_used() {
        let root = Path::new("/p");
        assert_eq!(
            prefab_path(root, "Ball", Some(Path::new("/p/assets/enemies"))),
            PathBuf::from("/p/assets/enemies/Ball.ome_scene"),
        );
    }

    /// Two entities with one name is ordinary; the second must not replace
    /// the first.
    ///
    /// `std::env::temp_dir` rather than a crate — there is no `tempfile` in
    /// this workspace, and the rest of the editor's file tests do the same.
    #[test]
    fn an_existing_name_is_suffixed_rather_than_overwritten() {
        let root = std::env::temp_dir().join("ome_prefab_name_clash");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("assets/Enemy.ome_scene"), "").unwrap();

        let next = prefab_path(&root, "Enemy", None);
        assert_eq!(next, root.join("assets/Enemy_1.ome_scene"));

        std::fs::write(&next, "").unwrap();
        assert_eq!(
            prefab_path(&root, "Enemy", None),
            root.join("assets/Enemy_2.ome_scene"),
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
