//! Importing files into the project, and editing a material in place.

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::asset_loader::AssetServer;
use kooch_core::assets::Assets;
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

/// Rewrites a texture's `[import]` table and queues the re-upload.
pub(super) fn handle_set_image_import(
    resources: &mut Resources,
    guid: Guid,
    import: kooch_render::texture::ImageImport,
) {
    let Some(path) = resources
        .get::<AssetDatabase>()
        .and_then(|db| db.entry(guid).map(|e| e.path.clone()))
    else {
        tracing::warn!(guid = %guid, "SetImageImport: no path in AssetDatabase; not persisted");
        return;
    };

    let mut meta = match kooch_core::asset_meta::read_meta(&path) {
        Ok(meta) => meta,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "SetImageImport: no readable .meta beside the texture",
            );
            return;
        }
    };
    meta.import = match kooch_core::toml::Table::try_from(import) {
        Ok(table) => Some(table),
        Err(error) => {
            tracing::warn!(%error, "SetImageImport: import settings did not serialise");
            return;
        }
    };
    if let Err(error) = kooch_core::asset_meta::write_meta(&path, &meta) {
        tracing::warn!(path = %path.display(), %error, "SetImageImport: could not write .meta");
        return;
    }

    // The bytes did not change, so the mtime-driven reload will not fire
    // on its own. This is the re-import.
    crate::actions::handlers::asset_saved(resources, &path);
    if let Some(reimports) = resources.get_mut::<kooch_render::material::TextureReimports>() {
        reimports.queue(guid);
    } else {
        let mut reimports = kooch_render::material::TextureReimports::default();
        reimports.queue(guid);
        resources.insert(reimports);
    }
}

/// Applies a Material asset edit.
pub(super) fn handle_edit_material(
    resources: &mut Resources,
    guid: Guid,
    material: &Material,
    commit: bool,
) {
    let Some(path) = material_path(resources, guid) else {
        tracing::warn!(guid = %guid, "EditMaterial: not a material asset; not persisted");
        return;
    };
    let material = &keep_declared(resources, guid, material);

    // Before the edit, and on the preview too: the preview is the first frame of a drag, so
    // recording only on commit would snapshot the value the drag already reached. The merge key is
    // what keeps the rest of the drag from filing sixty more.
    crate::history::documents::record(
        resources,
        &crate::history::Document::Asset(guid),
        "Edit Material",
        Some(crate::history::MergeKey::of(("material", guid))),
    );

    if !commit {
        preview_material(resources, guid, material);
        return;
    }

    persist_material(resources, guid, material, &path);
}

/// A shader switch keeps the values both shaders declare and drops the rest (#1158). Recorded like
/// any edit, so undo brings the dropped values back.
fn keep_declared(resources: &mut Resources, guid: Guid, edited: &Material) -> Material {
    let mut edited = edited.clone();
    let before = load_material(resources, guid).and_then(|m| m.shader);
    if before == edited.shader {
        return edited;
    }
    let declared = edited
        .shader
        .and_then(|shader| crate::systems::asset_detail::shader_params(shader, resources))
        .unwrap_or_default();
    kooch_render::material::retain_declared(&mut edited.values, &declared);
    edited
}

/// The material as the world currently holds it.
fn load_material(resources: &mut Resources, guid: Guid) -> Option<Material> {
    let mut server = resources.remove::<AssetServer>()?;
    let handle = server.load_by_guid::<Material>(guid, resources);
    resources.insert(server);
    resources
        .get::<Assets<Material>>()?
        .get(handle.ok()?)
        .cloned()
}

/// Puts a material into the world *and* onto disk.
pub(crate) fn write_material(resources: &mut Resources, guid: Guid, material: &Material) {
    let Some(path) = material_path(resources, guid) else {
        return;
    };
    preview_material(resources, guid, material);
    persist_material(resources, guid, material, &path);
}

/// The file behind `guid`, only when it is a material: a stale guid would otherwise write
/// material RON over whatever asset it names (#1189).
fn material_path(resources: &Resources, guid: Guid) -> Option<std::path::PathBuf> {
    let entry = resources.get::<AssetDatabase>()?.entry(guid)?;
    (entry.type_name.as_deref() == Some(kooch_render::material::MATERIAL_TYPE_NAME))
        .then(|| entry.path.clone())
}

fn persist_material(
    resources: &mut Resources,
    guid: Guid,
    material: &Material,
    path: &std::path::Path,
) {
    let text = match ron::ser::to_string_pretty(material, ron::ser::PrettyConfig::default()) {
        Ok(text) => text,
        Err(e) => {
            tracing::error!(guid = %guid, error = %e, "failed to serialise material");
            return;
        }
    };
    if let Err(e) = std::fs::write(path, text) {
        tracing::error!(path = %path.display(), error = %e, "failed to write material");
        return;
    }
    crate::actions::handlers::asset_saved(resources, path);
    tracing::info!(path = %path.display(), "material saved");
}

/// Shows an in-flight edit without recording it: overwrites the slot the
/// render sync already reads, and touches neither the disk nor the wire.
fn preview_material(resources: &mut Resources, guid: Guid, material: &Material) {
    let Some(mut server) = resources.remove::<AssetServer>() else {
        return;
    };
    let handle = server.load_by_guid::<Material>(guid, resources);
    resources.insert(server);
    if let Ok(handle) = handle
        && let Some(assets) = resources.get_mut::<Assets<Material>>()
        && let Some(slot) = assets.get_mut(handle)
    {
        *slot = material.clone();
    }
}

/// Writes one field of any reflected asset (#744).
pub(super) fn handle_edit_asset_field(
    resources: &mut Resources,
    guid: Guid,
    field: &str,
    value: kooch_ecs::reflect::ReflectValue,
    commit: bool,
) {
    let Some((path, type_name)) = resources.get::<AssetDatabase>().and_then(|db| {
        let entry = db.entry(guid)?;
        Some((entry.path.clone(), entry.type_name.clone()?))
    }) else {
        tracing::warn!(guid = %guid, "EditAssetField: no path or type in AssetDatabase");
        return;
    };

    let Some(registration) = kooch_ecs::reflect::reflected_asset(&type_name) else {
        // The Inspector only offers these widgets for a registered type,
        // so reaching here means the registry and the panel disagree —
        // worth a line rather than a silent no-op.
        tracing::warn!(%type_name, "EditAssetField: type is not a reflected asset");
        return;
    };

    // Same shape as the material path: recorded on the way in, merged by
    // field so a drag is one step.
    crate::history::documents::record(
        resources,
        &crate::history::Document::Asset(guid),
        &format!("Set {field}"),
        Some(crate::history::MergeKey::of((guid, field))),
    );

    if !(registration.write)(resources, guid, field, value) {
        tracing::warn!(%type_name, field, "EditAssetField: the asset refused the value");
        return;
    }
    if !commit {
        return;
    }

    persist_asset(resources, guid, registration, &path);
}

/// Serialises a reflected asset to its file and refreshes from it.
pub(crate) fn persist_asset(
    resources: &mut Resources,
    guid: Guid,
    registration: &kooch_ecs::reflect::ReflectedAssetRegistration,
    path: &std::path::Path,
) {
    let Some(text) = (registration.to_ron)(resources, guid) else {
        tracing::error!(guid = %guid, "failed to serialise the asset; not persisted");
        return;
    };
    // 🔴 A write that changes nothing is not a write.
    if !needs_write(path, &text) {
        return;
    }
    if let Err(e) = std::fs::write(path, text) {
        tracing::error!(path = %path.display(), error = %e, "failed to write asset");
        return;
    }
    super::asset_saved(resources, path);
}

/// Whether `text` differs from what `path` already holds.
pub(crate) fn needs_write(path: &std::path::Path, text: &str) -> bool {
    !std::fs::read_to_string(path).is_ok_and(|on_disk| on_disk == text)
}

#[cfg(test)]
mod write_guard_tests;

#[cfg(test)]
mod material_path_tests;

/// Turns one pair of the project's collision matrix on or off and writes the table back (#1302).
/// Both halves, since a relationship between two layers is one fact.
pub(super) fn handle_set_layer_pair(
    resources: &mut Resources,
    guid: Option<Guid>,
    a: usize,
    b: usize,
    collide: bool,
) {
    let Some(guid) = guid else {
        tracing::warn!("SetLayerPair: the project has no .layers file to write");
        return;
    };
    let Some(path) = resources
        .get::<AssetDatabase>()
        .and_then(|db| Some(db.entry(guid)?.path.clone()))
    else {
        tracing::warn!(guid = %guid, "SetLayerPair: no path in AssetDatabase");
        return;
    };
    let mut names = resources
        .get::<kooch_core::layers::LayerNames>()
        .cloned()
        .unwrap_or_default();
    names.set_collide(a, b, collide);
    let Ok(text) = ron::ser::to_string_pretty(&names, ron::ser::PrettyConfig::default()) else {
        tracing::error!("SetLayerPair: the table did not serialise");
        return;
    };
    if let Err(e) = std::fs::write(&path, text) {
        tracing::error!(path = %path.display(), error = %e, "failed to write the collision matrix");
        return;
    }
    // Published now rather than next frame, so the grid under the cursor answers with it.
    resources.insert(names);
}

/// Renames one of the project's layers and writes the table back (#1218). The names are not
/// reflected — a list of strings is not a field grid — so this walks the file itself.
pub(super) fn handle_rename_layer(
    resources: &mut Resources,
    guid: Option<Guid>,
    index: usize,
    name: &str,
) {
    let Some(guid) = guid else {
        tracing::warn!("RenameLayer: the project has no .layers file to write");
        return;
    };
    let Some(path) = resources
        .get::<AssetDatabase>()
        .and_then(|db| Some(db.entry(guid)?.path.clone()))
    else {
        tracing::warn!(guid = %guid, "RenameLayer: no path in AssetDatabase");
        return;
    };
    let mut names = resources
        .get::<kooch_core::layers::LayerNames>()
        .cloned()
        .unwrap_or_default();
    names.set(index, name);
    let Ok(text) = ron::ser::to_string_pretty(&names, ron::ser::PrettyConfig::default()) else {
        tracing::error!("RenameLayer: the table did not serialise");
        return;
    };
    // 🔴 A write that changes nothing is not a write: the field reports an edit every frame it has
    // focus, and each write round-trips to the running project.
    if !needs_write(&path, &text) {
        return;
    }
    if let Err(e) = std::fs::write(&path, text) {
        tracing::error!(path = %path.display(), error = %e, "failed to write the layer names");
        return;
    }
    // Published now rather than next frame, so the checklist under the cursor renames with it.
    resources.insert(names);
    super::asset_saved(resources, &path);
}
