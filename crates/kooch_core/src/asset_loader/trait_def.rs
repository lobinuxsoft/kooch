use super::error::{AssetError, AssetResult};
use crate::assets::{Asset, Assets, Handle};
use crate::resource::Resources;
use std::any::{Any, TypeId, type_name};
use std::marker::PhantomData;
use std::path::Path;

/// Per-load context handed to the loader's `load` call.
///
/// Carries the source path (for error messages, relative path resolution)
/// and a slot the loader can stash sub-assets into. PR-1 keeps this minimal;
/// future iterations grow it (texture loader returning a `Handle<Image>`,
/// glTF loader producing a `Handle<Material>` per primitive, etc.).
pub struct LoadContext<'a> {
    /// Absolute path the bytes came from.
    pub path: &'a Path,
    /// The `[import]` table of the asset's `.meta`, when it has one.
    ///
    /// Private and read through [`Self::import`]: the settings are
    /// per-type — a texture's chain, a mesh's scale — and `kooch_core`
    /// has no business knowing what any of them mean. It carries the
    /// table; the loader that owns the type deserialises it.
    import: Option<&'a toml::Table>,
}

impl<'a> LoadContext<'a> {
    /// A context with no import settings, which is what a test and a
    /// `.meta`-less file both get.
    pub fn new(path: &'a Path) -> Self {
        Self { path, import: None }
    }

    /// The same, carrying the sidecar's `[import]` table.
    pub fn with_import(path: &'a Path, import: Option<&'a toml::Table>) -> Self {
        Self { path, import }
    }

    /// The loader's own import settings, or their defaults.
    ///
    /// 🔴 Defaults rather than an error on a malformed table, and it is
    /// deliberate: a settings file should never stop an asset from
    /// loading. A `.meta` written by a newer engine, or hand-edited with
    /// a typo, has to leave the texture on screen — the failure belongs
    /// in the log, not in a missing mesh.
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
///
/// Generic over the asset type `T` so a `GltfMeshLoader` returns `Mesh`,
/// an `ImageLoader` returns `Image`, etc. `Send + Sync + 'static` lets
/// the `AssetServer` store them across threads when async/streaming
/// arrives.
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
    fn asset_type_id(&self) -> TypeId;

    /// Parses `bytes` and writes the result **over the slot `key` already
    /// points at**, rather than storing it somewhere new.
    ///
    /// This is what makes a reload visible. Loading again would call
    /// `Assets::insert`, which mints a fresh key — every `Handle<T>`
    /// already held by a component, a field or an instance would keep
    /// resolving to the copy from before the edit, and the reload would
    /// change nothing anyone can see. Overwriting in place leaves the
    /// key untouched, so every existing handle reads the new bytes
    /// without knowing a reload happened.
    ///
    /// Only the typed adapter can do this: the slot lives in
    /// `Assets<T>`, and `T` is exactly what the type-erased registry
    /// threw away.
    ///
    /// Returns `false` when `key` no longer points at a live slot — the
    /// asset was removed after it was cached. That is a stale cache
    /// entry rather than a failure, and the caller drops it.
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

    fn asset_type_id(&self) -> TypeId {
        TypeId::of::<T>()
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
