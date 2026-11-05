//! Editing a project's data files (#1796) with no type behind them: the project declared their
//! fields and defaults, and the file is the only copy of the values on this side.

use std::path::Path;
use std::time::SystemTime;

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::dynamic::DataAssetTypes;
use kooch_core::resource::Resources;
use kooch_ecs::reflect::{DataFields, ReflectValue, data_text};

/// The file the Inspector has open: it asks every frame, and an edit lives here until it commits.
struct OpenDataAsset {
    guid: Guid,
    modified: Option<SystemTime>,
    fields: DataFields,
}

fn modified(path: &Path) -> Option<SystemTime> {
    path.metadata().and_then(|meta| meta.modified()).ok()
}

/// Whether the loaded project declared `type_name`.
pub(crate) fn is_data(resources: &Resources, type_name: &str) -> bool {
    resources
        .get::<DataAssetTypes>()
        .is_some_and(|types| types.get(type_name).is_some())
}

/// Each declared `(extension, type name)`, for the asset scan.
pub(crate) fn extensions(resources: &Resources) -> Vec<(String, String)> {
    let types = resources.get::<DataAssetTypes>();
    let schemas = types.map(DataAssetTypes::all).unwrap_or_default();
    schemas
        .iter()
        .map(|schema| (schema.extension.clone(), schema.type_name.clone()))
        .collect()
}

/// The type's fields in its order, each holding the file's value when it still fits.
fn merged(defaults: DataFields, file: DataFields) -> DataFields {
    let found = |name: &str, value: &ReflectValue| {
        let own = file.iter().find(|(own, _)| own == name)?;
        let same = std::mem::discriminant(&own.1) == std::mem::discriminant(value);
        same.then(|| own.1.clone())
    };
    defaults
        .iter()
        .map(|(name, value)| {
            let value = found(name, value).unwrap_or_else(|| value.clone());
            (name.clone(), value)
        })
        .collect()
}

/// The fields of the file `guid`, read again only when it changed on disk.
pub(crate) fn fields_of(resources: &mut Resources, guid: Guid) -> Option<DataFields> {
    let entry = resources.get::<AssetDatabase>()?.entry(guid)?;
    let (path, type_name) = (entry.path.clone(), entry.type_name.clone()?);
    let stamp = modified(&path);
    if let Some(open) = resources.get::<OpenDataAsset>()
        && open.guid == guid
        && open.modified == stamp
    {
        return Some(open.fields.clone());
    }
    let schema = resources.get::<DataAssetTypes>()?.get(&type_name)?;
    let defaults: DataFields = ron::from_str(&schema.defaults).ok()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let fields = merged(defaults, ron::from_str(&text).ok()?);
    resources.insert(OpenDataAsset {
        guid,
        modified: stamp,
        fields: fields.clone(),
    });
    Some(fields)
}

/// Sets one field of the open file. `false` when it has no such field or the kind differs.
pub(crate) fn write(
    resources: &mut Resources,
    guid: Guid,
    field: &str,
    value: ReflectValue,
) -> bool {
    if fields_of(resources, guid).is_none() {
        return false;
    }
    let Some(open) = resources.get_mut::<OpenDataAsset>() else {
        return false;
    };
    let Some((_, own)) = open.fields.iter_mut().find(|(name, _)| name == field) else {
        return false;
    };
    if std::mem::discriminant(own) != std::mem::discriminant(&value) {
        return false;
    }
    *own = value;
    true
}

/// Writes the open file out and tells the running game.
pub(crate) fn persist(resources: &mut Resources, path: &Path) {
    let Some(open) = resources.get::<OpenDataAsset>() else {
        return;
    };
    if let Err(error) = std::fs::write(path, data_text(&open.fields)) {
        tracing::error!(path = %path.display(), %error, "failed to write the data asset");
        return;
    }
    let stamp = modified(path);
    if let Some(open) = resources.get_mut::<OpenDataAsset>() {
        open.modified = stamp;
    }
    crate::actions::handlers::asset_saved(resources, path);
}

#[cfg(test)]
mod tests;
