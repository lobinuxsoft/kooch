//! Importing files into the project, and editing a material in place.

use ome_core::Guid;
use ome_core::asset_database::AssetDatabase;
use ome_core::asset_loader::AssetServer;
use ome_core::assets::Assets;
use ome_core::resource::Resources;
use ome_render::material::Material;

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
    let mut copied = 0usize;
    for src in files {
        let Some(name) = src.file_name() else {
            continue;
        };
        let target = crate::actions::asset_ops::unique_target(dest, name);
        match std::fs::copy(src, &target) {
            Ok(_) => {
                copied += 1;
                tracing::info!(from = %src.display(), to = %target.display(), "asset imported");
            }
            Err(e) => {
                tracing::error!(from = %src.display(), error = %e, "asset import failed");
            }
        }
    }
    if copied > 0 {
        crate::actions::asset_ops::force_rescan(resources);
    }
}

/// Applies a Material asset edit: updates `Assets<Material>` in place so
/// the render sync uploads the new params live, then serialises the
/// material back to its source `.ron` so the change survives a reload.
pub(super) fn handle_edit_material(resources: &mut Resources, guid: Guid, material: &Material) {
    // 1. Live update: resolve the GUID to a handle (loading if needed)
    //    and overwrite the stored asset.
    let Some(mut server) = resources.remove::<AssetServer>() else {
        tracing::warn!("EditMaterial: AssetServer missing; edit dropped");
        return;
    };
    let handle = server.load_by_guid::<Material>(guid, resources);
    resources.insert(server);
    match handle {
        Ok(h) => {
            if let Some(assets) = resources.get_mut::<Assets<Material>>()
                && let Some(slot) = assets.get_mut(h)
            {
                *slot = material.clone();
            }
        }
        Err(e) => {
            tracing::warn!(guid = %guid, error = %e, "EditMaterial: failed to resolve material")
        }
    }

    // 2. Persist to disk at the asset's registered path.
    let Some(path) = resources
        .get::<AssetDatabase>()
        .and_then(|db| db.entry(guid).map(|e| e.path.clone()))
    else {
        tracing::warn!(guid = %guid, "EditMaterial: no path in AssetDatabase; not persisted");
        return;
    };
    match ron::ser::to_string_pretty(material, ron::ser::PrettyConfig::default()) {
        Ok(text) => match std::fs::write(&path, text) {
            Ok(()) => tracing::info!(path = %path.display(), "material saved"),
            Err(e) => {
                tracing::error!(path = %path.display(), error = %e, "failed to write material")
            }
        },
        Err(e) => tracing::error!(guid = %guid, error = %e, "failed to serialise material"),
    }
}
