use super::error::{AssetError, AssetResult};
use crate::assets::{Asset, Assets, Handle};
use crate::resource::Resources;
use std::any::{Any, type_name};
use std::marker::PhantomData;
use std::path::Path;

/// Per-load context handed to the loader's `load` call.
pub struct LoadContext<'a> {
    /// Absolute path the bytes came from.
    pub path: &'a Path,
    /// The `[import]` table of the asset's `.meta`, when it has one.
    import: Option<&'a toml::Table>,
    /// Reads a file beside the asset; without one, the disk. A packaged game has nothing on disk,
    /// so a file the asset names by relative path (a `.gltf`'s `.bin`) must come through the pack.
    reader: Option<&'a mut dyn FnMut(&Path) -> AssetResult<Vec<u8>>>,
}

impl<'a> LoadContext<'a> {
    /// A context with no import settings, which is what a test and a
    /// `.meta`-less file both get.
    pub fn new(path: &'a Path) -> Self {
        Self {
            path,
            import: None,
            reader: None,
        }
    }

    /// The same, carrying the sidecar's `[import]` table.
    pub fn with_import(path: &'a Path, import: Option<&'a toml::Table>) -> Self {
        Self {
            path,
            import,
            reader: None,
        }
    }

    /// The same, reading neighbours through `reader` instead of the disk.
    pub fn with_reader(mut self, reader: &'a mut dyn FnMut(&Path) -> AssetResult<Vec<u8>>) -> Self {
        self.reader = Some(reader);
        self
    }

    /// The bytes of `relative`, resolved against the asset's own directory.
    pub fn read_sibling(&mut self, relative: &str) -> AssetResult<Vec<u8>> {
        let path = self.path.parent().unwrap_or(Path::new("")).join(relative);
        match self.reader.as_mut() {
            Some(read) => read(&path),
            None => Ok(std::fs::read(&path)?),
        }
    }

    /// The loader's own import settings, or their defaults.
    pub fn import<T: serde::de::DeserializeOwned + Default>(&self) -> T {
        let Some(table) = self.import else {
            return T::default();
        };
        match table.clone().try_into() {
            Ok(settings) => settings,
            Err(error) => {
                tracing::warn!(
                    target: "kooch_core::asset_loader",
                    path = %self.path.display(),
                    %error,
                    "the .meta's [import] table did not parse; using defaults",
                );
                T::default()
            }
        }
    }
}

/// Trait every asset loader implements.
pub trait AssetLoader<T: Asset>: Send + Sync + 'static {
    /// Lower-case extensions handled by this loader (no leading dot).
    /// `["glb", "gltf"]`, `["png", "jpg", "jpeg"]`, etc.
    fn extensions(&self) -> &[&'static str];

    /// Parse `bytes` into an asset of type `T`.
    fn load(&self, bytes: &[u8], ctx: &mut LoadContext<'_>) -> AssetResult<T>;
}

/// Type-erased loader interface. The registry stores `Box<dyn UntypedLoader>`
/// so loaders for any `T` fit in the same `HashMap`. A typed downcast on
/// load brings `T` back at the call site.
pub(crate) trait UntypedLoader: Send + Sync {
    fn extensions(&self) -> &[&'static str];
    fn load_boxed(
        &self,
        bytes: &[u8],
        ctx: &mut LoadContext<'_>,
    ) -> AssetResult<Box<dyn Any + Send + Sync>>;
    fn asset_type_name(&self) -> &'static str;

    /// Parses `bytes` and writes the result **over the slot `key` already points at**, rather than
    /// storing it somewhere new.
    fn reload_into(
        &self,
        bytes: &[u8],
        ctx: &mut LoadContext<'_>,
        key: slotmap::DefaultKey,
        resources: &mut Resources,
    ) -> AssetResult<bool>;
}

/// Adapter that bridges a concrete `AssetLoader<T>` into the type-erased
/// `UntypedLoader` storage. Owns the loader and erases `T` for storage.
pub(crate) struct TypedLoader<L, T>
where
    L: AssetLoader<T>,
    T: Asset,
{
    pub(crate) inner: L,
    pub(crate) _marker: PhantomData<fn() -> T>,
}

impl<L, T> UntypedLoader for TypedLoader<L, T>
where
    L: AssetLoader<T>,
    T: Asset,
{
    fn extensions(&self) -> &[&'static str] {
        self.inner.extensions()
    }

    fn load_boxed(
        &self,
        bytes: &[u8],
        ctx: &mut LoadContext<'_>,
    ) -> AssetResult<Box<dyn Any + Send + Sync>> {
        let asset = self.inner.load(bytes, ctx)?;
        Ok(Box::new(asset))
    }

    fn asset_type_name(&self) -> &'static str {
        type_name::<T>()
    }

    fn reload_into(
        &self,
        bytes: &[u8],
        ctx: &mut LoadContext<'_>,
        key: slotmap::DefaultKey,
        resources: &mut Resources,
    ) -> AssetResult<bool> {
        // Parsed before the storage is borrowed, so a file that no longer
        // parses leaves the previous asset in place instead of blanking it.
        let asset = self.inner.load(bytes, ctx)?;
        let assets = resources
            .get_mut::<Assets<T>>()
            .ok_or_else(|| AssetError::MissingAssetStorage(type_name::<T>()))?;
        match assets.get_mut(Handle::<T>::from_key(key)) {
            Some(slot) => {
                *slot = asset;
                Ok(true)
            }
            None => Ok(false),
        }
    }
}
