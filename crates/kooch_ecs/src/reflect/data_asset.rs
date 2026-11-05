//! A project's own data files (#1796). The file holds reflected fields rather than the type's
//! serde form, so the editor can edit one without the type: it never crosses the dylib boundary.

use std::marker::PhantomData;

use kooch_core::Guid;
use kooch_core::asset_loader::{AssetError, AssetLoader, AssetResult, AssetServer, LoadContext};
use kooch_core::assets::Assets;
use kooch_core::resource::Resources;

use super::asset_registry::load_handle;
use super::{Reflect, ReflectValue};

/// A type a project keeps in its own files: Unity's `ScriptableObject`.
///
/// ```ignore
/// #[derive(Reflect, Default)]
/// pub struct BallConfig { pub max_torque: f32 }
/// impl DataAsset for BallConfig { const EXTENSION: &'static str = "ballconfig"; }
/// ```
pub trait DataAsset: Reflect + Default {
    /// The file's extension, without the dot.
    const EXTENSION: &'static str;
}

/// What a data file holds: each field by name, a field it lacks keeping the type's default.
pub type DataFields = Vec<(String, ReflectValue)>;

/// `asset`'s fields.
pub fn data_fields<T: DataAsset>(asset: &T) -> DataFields {
    let metas = asset.reflect_fields().iter();
    metas
        .filter_map(|meta| Some((meta.name.to_owned(), asset.reflect_get(meta.name)?)))
        .collect()
}

/// `fields` as a file's text.
pub fn data_text(fields: &[(String, ReflectValue)]) -> String {
    ron::ser::to_string_pretty(fields, ron::ser::PrettyConfig::default()).unwrap_or_default()
}

/// Fills `T` from a file's fields. A field the type no longer has is dropped: the file outlives
/// renames, and refusing it would lose every other value.
fn from_fields<T: DataAsset>(fields: DataFields) -> T {
    let mut asset = T::default();
    for (name, value) in fields {
        if let Err(error) = asset.reflect_set(&name, value) {
            tracing::warn!(%name, %error, "a data asset's field does not fit its type");
        }
    }
    asset
}

/// Loads any [`DataAsset`].
struct DataAssetLoader<T> {
    extensions: [&'static str; 1],
    kind: PhantomData<fn() -> T>,
}

impl<T: DataAsset> AssetLoader<T> for DataAssetLoader<T> {
    fn extensions(&self) -> &[&'static str] {
        &self.extensions
    }

    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<T> {
        let text = std::str::from_utf8(bytes).map_err(|e| AssetError::Loader(Box::new(e)))?;
        let fields: DataFields =
            ron::from_str(text).map_err(|e| AssetError::Loader(Box::new(e)))?;
        Ok(from_fields(fields))
    }
}

/// Makes `T` loadable; the project's generated registrations call it.
pub fn install_data_asset<T: DataAsset>(resources: &mut Resources) {
    let Some(server) = resources.get_mut::<AssetServer>() else {
        tracing::warn!(
            extension = T::EXTENSION,
            "no AssetServer to load a data asset"
        );
        return;
    };
    server.register_loader::<T, _>(DataAssetLoader {
        extensions: [T::EXTENSION],
        kind: PhantomData,
    });
    resources.insert(Assets::<T>::new());
}

/// The file `guid` points at, loading it on first use.
pub fn data_asset<T: DataAsset>(resources: &mut Resources, guid: Guid) -> Option<&T> {
    let handle = load_handle::<T>(resources, guid)?;
    resources.get::<Assets<T>>()?.get(handle)
}

#[cfg(test)]
mod tests;
