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

/// Applies a Material asset edit.
///
/// Two speeds, because a slider produces one of these per frame:
///
/// - **Not committed** — overwrite the local `Assets<Material>` and stop.
///   The viewport follows the drag and nothing touches the disk.
/// - **Committed** — write the `.ron`, then let [`asset_saved`] refresh
///   from it and tell the project.
///
/// The live update used to be unconditional, and the write with it. That
/// worked in this window and nowhere else: the project runs in its own
/// process with its own `Assets<Material>` and nothing told it, so
/// editing a material while connected changed the Inspector and left the
/// running game rendering the old one. Committing writes first and
/// refreshes from disk, which keeps one direction of travel — the file is
/// the material, and both processes read it the same way.
///
/// [`asset_saved`]: crate::actions::handlers::asset_saved
pub(super) fn handle_edit_material(
    resources: &mut Resources,
    guid: Guid,
    material: &Material,
    commit: bool,
) {
    let Some(path) = resources
        .get::<AssetDatabase>()
        .and_then(|db| db.entry(guid).map(|e| e.path.clone()))
    else {
        tracing::warn!(guid = %guid, "EditMaterial: no path in AssetDatabase; not persisted");
        return;
    };

    // Before the edit, and on the preview too: the preview is the first
    // frame of a drag, so recording only on commit would snapshot the
    // value the drag already reached. The merge key is what keeps the
    // rest of the drag from filing sixty more.
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

/// Puts a material into the world *and* onto disk.
///
/// The write is what an undo has to reach: a committed edit wrote the
/// file, so restoring only the in-memory copy would leave the value the
/// user undid sitting in the asset both processes read.
pub(crate) fn write_material(resources: &mut Resources, guid: Guid, material: &Material) {
    let Some(path) = resources
        .get::<AssetDatabase>()
        .and_then(|db| db.entry(guid).map(|e| e.path.clone()))
    else {
        return;
    };
    preview_material(resources, guid, material);
    persist_material(resources, guid, material, &path);
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
///
/// The generic counterpart to [`handle_edit_material`], and the same
/// two-speed shape:
///
/// - **Live** — set the field on the in-memory asset. The viewport
///   follows the drag and nothing touches the disk.
/// - **Committed** — write the file, then let [`asset_saved`] refresh
///   from it and tell the running project.
///
/// Committing writes the file *first* and refreshes from it, keeping one
/// direction of travel: the file is the asset, and both processes read
/// it the same way. Skipping that is how editing a material used to
/// change the Inspector and leave the running game rendering the old one.
///
/// [`asset_saved`]: crate::actions::handlers::asset_saved
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
///
/// Shared with the undo path, which has to persist for the same reason
/// the commit does — the file is the asset.
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
    if let Err(e) = std::fs::write(path, text) {
        tracing::error!(path = %path.display(), error = %e, "failed to write asset");
        return;
    }
    super::asset_saved(resources, path);
}
