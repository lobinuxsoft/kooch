use super::error::{AssetError, AssetResult};
use super::trait_def::{AssetLoader, LoadContext, TypedLoader, UntypedLoader};
use crate::asset_database::{AssetDatabase, AssetEntry};
use crate::asset_meta;
use crate::assets::{Asset, Assets, Handle};
use crate::guid::Guid;
use crate::resource::Resources;
use std::any::{TypeId, type_name};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Type-erased registry of loaders + path-cache resource.
pub struct AssetServer {
    loaders: HashMap<TypeId, Box<dyn UntypedLoader>>,
    cache: HashMap<(TypeId, PathBuf), slotmap::DefaultKey>,
    /// Directory paths are resolved relative to. `None` keeps paths raw.
    asset_root: Option<PathBuf>,
    /// Packs to read through before touching the disk (#758). Empty in
    /// the editor and in any `cargo run`, so development is unaffected.
    packs: super::packs::Packs,
}

impl AssetServer {
    /// Empty server with no loaders and no asset root.
    pub fn new() -> Self {
        Self {
            loaders: HashMap::new(),
            cache: HashMap::new(),
            asset_root: None,
            packs: super::packs::Packs::default(),
        }
    }

    /// Sets the directory all relative load paths are resolved against.
    /// Absolute paths bypass this entirely.
    pub fn with_asset_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.asset_root = Some(root.into());
        self
    }

    /// Returns the configured asset root, if any.
    pub fn asset_root(&self) -> Option<&Path> {
        self.asset_root.as_deref()
    }

    /// Registers a loader for asset type `T`. Replaces any prior loader for the same type silently
    /// — last-write-wins.
    pub fn register_loader<T, L>(&mut self, loader: L)
    where
        T: Asset,
        L: AssetLoader<T>,
    {
        let typed = TypedLoader::<L, T> {
            inner: loader,
            _marker: PhantomData,
        };
        self.loaders.insert(TypeId::of::<T>(), Box::new(typed));
    }

    /// Returns `true` when a loader is registered for `T`.
    pub fn has_loader<T: Asset>(&self) -> bool {
        self.loaders.contains_key(&TypeId::of::<T>())
    }

    /// Every `(extension, asset type name)` pair any registered loader claims.
    pub fn known_extensions(&self) -> Vec<(&'static str, &'static str)> {
        self.loaders
            .values()
            .flat_map(|loader| {
                let type_name = loader.asset_type_name();
                loader.extensions().iter().map(move |ext| (*ext, type_name))
            })
            .collect()
    }

    /// Returns the extensions claimed by `T`'s loader, or `&[]` if none.
    pub fn extensions_for<T: Asset>(&self) -> &[&'static str] {
        self.loaders
            .get(&TypeId::of::<T>())
            .map(|loader| loader.extensions())
            .unwrap_or(&[])
    }

    /// Loads an asset of type `T` from disk, inserts it into the matching `Assets<T>` resource, and
    /// returns its handle.
    pub fn load<T: Asset>(
        &mut self,
        path: impl AsRef<Path>,
        resources: &mut Resources,
    ) -> AssetResult<Handle<T>> {
        let path = self.resolve_path(path.as_ref());
        let cache_key = (TypeId::of::<T>(), path.clone());
        if let Some(key) = self.cache.get(&cache_key) {
            return Ok(Handle::<T>::from_key(*key));
        }

        // First-time load: ensure the asset has a `.meta` sidecar (one is generated on the spot if
        // missing) and register the resulting GUID in the `AssetDatabase` resource if it exists.
        Self::ensure_guid_identity(&path, resources, type_name::<T>());

        let loader = self
            .loaders
            .get(&TypeId::of::<T>())
            .ok_or_else(|| AssetError::NoLoaderForType(type_name::<T>()))?;

        // Match the file name's lowercased basename against every suffix the loader claims.
        let file_name_lower = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|s| s.to_ascii_lowercase());
        let supports = match file_name_lower.as_deref() {
            Some(name) => loader.extensions().iter().any(|ext| {
                let suffix = format!(".{}", ext.to_ascii_lowercase());
                name.ends_with(&suffix)
            }),
            None => false,
        };
        if !supports {
            return Err(AssetError::UnsupportedExtension {
                path: path.clone(),
                registered: loader.extensions().to_vec(),
            });
        }

        let bytes = self.packs.read_or_disk(&path)?;
        // The sidecar is read for its `[import]` table, and its absence is not an error: a file
        // with no `.meta` yet — one dropped into the folder a moment ago — still loads, on the
        // engine's defaults.
        let meta = asset_meta::read_meta(&path).ok();
        let mut ctx =
            LoadContext::with_import(&path, meta.as_ref().and_then(|m| m.import.as_ref()));
        let boxed = loader.load_boxed(&bytes, &mut ctx)?;

        // Downcast back to T. Safe by construction — registry is keyed by
        // TypeId<T>, and the boxed value comes from a loader registered
        // for that exact TypeId.
        let asset = *boxed
            .downcast::<T>()
            .expect("loader produced wrong concrete asset type — registry corrupted");

        let assets = resources
            .get_mut::<Assets<T>>()
            .ok_or_else(|| AssetError::MissingAssetStorage(type_name::<T>()))?;
        let handle = assets.insert(asset);
        self.cache.insert(cache_key, handle.key());
        Ok(handle)
    }

    /// Loads an asset of type `T` referenced by [`Guid`]. The [`AssetDatabase`] resource must hold
    /// an entry for the GUID (typically populated by [`AssetDatabase::scan_directory`] at startup,
    /// or by a prior [`AssetServer::load`] call that triggered `.meta` registration).
    pub fn load_by_guid<T: Asset>(
        &mut self,
        guid: Guid,
        resources: &mut Resources,
    ) -> AssetResult<Handle<T>> {
        let path = {
            let db = resources
                .get::<AssetDatabase>()
                .ok_or(AssetError::MissingAssetStorage("AssetDatabase"))?;
            let entry = db.entry(guid).ok_or(AssetError::UnknownGuid(guid))?;
            entry.path.clone()
        };
        self.load::<T>(path, resources)
    }

    /// The asset's bytes, from a mounted pack or from disk, with no loader involved.
    pub fn read_bytes(&mut self, path: impl AsRef<Path>) -> AssetResult<Vec<u8>> {
        let path = self.resolve_path(path.as_ref());
        self.packs.read_or_disk(&path)
    }

    /// Returns the cached handle for `path` if `T` was loaded already,
    /// otherwise `None`. Does NOT trigger a load — read-only lookup.
    pub fn get_cached<T: Asset>(&self, path: impl AsRef<Path>) -> Option<Handle<T>> {
        let path = self.resolve_path(path.as_ref());
        self.cache
            .get(&(TypeId::of::<T>(), path))
            .map(|key| Handle::<T>::from_key(*key))
    }

    /// Drops every cached path → handle association without touching the
    /// `Assets<T>` storage. Use after a hot-reload pass that re-inserted
    /// fresh assets.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Forgets the cached handle for one path, so the next load re-reads it from disk.
    pub fn forget<T: Asset>(&mut self, path: impl AsRef<Path>) {
        let path = self.resolve_path(path.as_ref());
        self.cache.remove(&(TypeId::of::<T>(), path));
    }

    /// Re-reads `path` from disk and overwrites the assets already loaded from it, keeping their
    /// handles valid. Returns how many were refreshed.
    pub fn reload_path(
        &mut self,
        path: impl AsRef<Path>,
        resources: &mut Resources,
    ) -> AssetResult<usize> {
        let path = self.resolve_path(path.as_ref());
        // Collected first: the loop borrows `self.loaders` and mutates the
        // cache, neither of which can happen while iterating it.
        let cached: Vec<(TypeId, slotmap::DefaultKey)> = self
            .cache
            .iter()
            .filter(|((_, cached_path), _)| *cached_path == path)
            .map(|((type_id, _), key)| (*type_id, *key))
            .collect();
        if cached.is_empty() {
            return Ok(0);
        }

        // Read once even when several types share the file.
        let bytes = self.packs.read_or_disk(&path)?;
        let mut reloaded = 0usize;
        let mut stale = Vec::new();
        for (type_id, key) in cached {
            let Some(loader) = self.loaders.get(&type_id) else {
                // The type was loaded by a build that had this loader and
                // this one does not — nothing to refresh it with.
                continue;
            };
            // Hot reload takes the sidecar as it is on disk right now:
            // editing the `[import]` table and saving is a reload, and
            // the point of the table is to see the answer change.
            let meta = asset_meta::read_meta(&path).ok();
            let mut ctx =
                LoadContext::with_import(&path, meta.as_ref().and_then(|m| m.import.as_ref()));
            match loader.reload_into(&bytes, &mut ctx, key, resources)? {
                true => reloaded += 1,
                false => stale.push(type_id),
            }
        }
        for type_id in stale {
            self.cache.remove(&(type_id, path.clone()));
        }
        Ok(reloaded)
    }

    /// Guarantees that `path` has a `.meta` sidecar with a stable [`Guid`] and the recorded
    /// `asset_type` set to `type_name`, and that, if an `AssetDatabase` resource is present, the
    /// resulting `(guid, path, type_name)` triple is registered.
    fn ensure_guid_identity(path: &Path, resources: &mut Resources, type_name: &'static str) {
        if !path.exists() {
            return;
        }
        let meta = match asset_meta::read_or_create_typed(path, type_name) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(
                    target: "kooch_core::asset_loader",
                    path = %path.display(),
                    error = %e,
                    "failed to read or create .meta sidecar; continuing without GUID identity"
                );
                return;
            }
        };
        let Some(db) = resources.get_mut::<AssetDatabase>() else {
            return;
        };
        let mtime = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        // The sidecar's recorded type wins.
        db.register(
            meta.guid,
            AssetEntry {
                path: path.to_path_buf(),
                mtime,
                type_name: meta.asset_type.clone(),
            },
        );
        // Re-entrant safety: if the entry already existed under the same GUID with no type yet
        // (scanned at startup before any `load::<T>`), `register` keeps the freshly-typed entry; if
        // it already had a type, the new entry's type matches what the sidecar carries.
    }

    /// Resolves a caller-provided path against the configured asset root. Absolute paths bypass the
    /// root and pass through unchanged; relative paths are joined onto `asset_root`.
    pub fn resolve_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else if let Some(root) = &self.asset_root {
            root.join(path)
        } else {
            path.to_path_buf()
        }
    }

    /// Mounts a `.kpack` over `root`, so assets under that directory come out of the pack instead
    /// of the filesystem (#758).
    pub fn mount_pack(
        &mut self,
        root: impl Into<PathBuf>,
        pack: &Path,
        key: &kooch_pack::PackKey,
    ) -> Result<usize, kooch_pack::PackError> {
        let mounted = super::packs::MountedPack::open(root.into(), pack, key)?;
        let entries = mounted.len();
        self.packs.push(mounted);
        tracing::info!(
            target: "kooch_core::assets",
            path = %pack.display(),
            entries,
            "asset pack mounted",
        );
        Ok(entries)
    }

    #[cfg(test)]
    /// Whether anything is mounted — i.e. whether this is a packaged game
    /// rather than a project being edited.
    pub fn has_packs(&self) -> bool {
        !self.packs.is_empty()
    }

    /// Every path the mounted packs hold, as the engine names them.
    pub fn packed_paths(&self) -> Vec<PathBuf> {
        self.packs.paths()
    }

    /// Reads `path` out of a mounted pack, or `None` when no pack holds it.
    pub fn read_packed(&mut self, path: &Path) -> Option<Vec<u8>> {
        self.packs.read_packed(path)
    }
}

impl Default for AssetServer {
    fn default() -> Self {
        Self::new()
    }
}
