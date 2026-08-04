//! Importing files into the project, and editing a material in place.

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::resource::Resources;
use kooch_render::material::Material;

/// surface in the Asset Browser + pickers next frame.
pub(super) fn handle_import_assets(
    resources: &mut Resources,
    files: &[std::path::PathBuf],
    dest: &std::path::Path,
) {
    if let Err(e) = std::fs::create_dir_all(dest) {
        tracing::error!(dest = %dest.display(), error = %e, "import: cannot create destination");
        return;
    }
    let mut copied = Vec::new();
    for src in files {
        let Some(name) = src.file_name() else {
            continue;
        };
        let target = crate::actions::asset_ops::unique_target(dest, name);
        match std::fs::copy(src, &target) {
            Ok(_) => {
                tracing::info!(from = %src.display(), to = %target.display(), "asset imported");
                copied.push(target);
            }
            Err(e) => {
                tracing::error!(from = %src.display(), error = %e, "asset import failed");
            }
        }
    }
    if copied.is_empty() {
        return;
    }
    // The rescan is what gives the new files a `.meta`, so it comes first
    // and there is nothing to register from before it.
    crate::actions::asset_ops::force_rescan(resources);
    // The rescan is local to this process. Without this the project can be
    // handed a guid for a file it has no entry for, which fails as an
    // unknown asset rather than as anything that names the import.
    for target in &copied {
        crate::actions::handlers::asset_saved(resources, target);
    }
}

/// Applies a Material asset edit: writes the material back to its source
/// `.ron`, then lets the save path put it everywhere it has to go.
///
/// The live update used to be done here by hand, resolving the guid and
/// overwriting the slot. It worked in this window and nowhere else — the
/// project runs in its own process with its own `Assets<Material>`, and
/// nothing told it. Editing a material while connected changed the
/// Inspector and left the running game rendering the old one.
///
/// Writing first and refreshing from disk keeps one direction of travel:
/// the file is the material, and both processes read it the same way.
pub(super) fn handle_edit_material(resources: &mut Resources, guid: Guid, material: &Material) {
    let Some(path) = resources
        .get::<AssetDatabase>()
        .and_then(|db| db.entry(guid).map(|e| e.path.clone()))
    else {
        tracing::warn!(guid = %guid, "EditMaterial: no path in AssetDatabase; not persisted");
        return;
    };
    let text = match ron::ser::to_string_pretty(material, ron::ser::PrettyConfig::default()) {
        Ok(text) => text,
        Err(e) => {
            tracing::error!(guid = %guid, error = %e, "failed to serialise material");
            return;
        }
    };
    if let Err(e) = std::fs::write(&path, text) {
        tracing::error!(path = %path.display(), error = %e, "failed to write material");
        return;
    }
    crate::actions::handlers::asset_saved(resources, &path);
    tracing::info!(path = %path.display(), "material saved");
}
