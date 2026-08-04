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
///
/// Insert as a `Resource` at engine startup. Game code calls
/// [`AssetServer::load`] which:
///
/// 1. Looks up the loader registered for `T` (by `TypeId`).
/// 2. Reads the file from disk (sync for now — async lands later).
/// 3. Validates the extension is one the loader claims.
/// 4. Hands bytes to the loader and inserts the result into `Assets<T>`.
/// 5. Caches `(TypeId, path) -> handle` so subsequent loads of the same
///    asset return the same `Handle<T>` (deduplication).
///
/// # Determinism
///
/// `AssetServer` is single-threaded by design — it owns sync I/O. When a
/// streaming layer arrives, it will ride on top with its own thread pool
/// and call `AssetServer` from the main thread to commit results.
pub struct AssetServer {
    loaders: HashMap<TypeId, Box<dyn UntypedLoader>>,
    cache: HashMap<(TypeId, PathBuf), slotmap::DefaultKey>,
    /// Directory paths are resolved relative to. `None` keeps paths raw.
    asset_root: Option<PathBuf>,
}

impl AssetServer {
    /// Empty server with no loaders and no asset root.
    pub fn new() -> Self {
        Self {
            loaders: HashMap::new(),
            cache: HashMap::new(),
            asset_root: None,
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

    /// Registers a loader for asset type `T`. Replaces any prior loader
    /// for the same type silently — last-write-wins.
    ///
    /// Each `T` has at most one registered loader; multi-loader-per-type
    /// (different extensions handled by different loaders) is deferred —
    /// most concrete loaders advertise multiple extensions internally.
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

    /// Returns the extensions claimed by `T`'s loader, or `&[]` if none.
    pub fn extensions_for<T: Asset>(&self) -> &[&'static str] {
        self.loaders
            .get(&TypeId::of::<T>())
            .map(|loader| loader.extensions())
            .unwrap_or(&[])
    }

    /// Loads an asset of type `T` from disk, inserts it into the matching
    /// `Assets<T>` resource, and returns its handle.
    ///
    /// Subsequent loads of the same path return the cached handle without
    /// re-reading the file.
    ///
    /// # Errors
    ///
    /// - [`AssetError::NoLoaderForType`] when no loader is registered.
    /// - [`AssetError::UnsupportedExtension`] when the path's extension
    ///   isn't in the loader's claim list.
    /// - [`AssetError::Io`] when the file cannot be read.
    /// - [`AssetError::MissingAssetStorage`] when `Assets<T>` is not in
    ///   `resources`.
    /// - [`AssetError::Loader`] for parser failures.
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

        // First-time load: ensure the asset has a `.meta` sidecar (one
        // is generated on the spot if missing) and register the
        // resulting GUID in the `AssetDatabase` resource if it exists.
        // The type-aware path back-fills `asset_type` whenever an
        // existing sidecar predates the field. Failures here only emit
        // warnings — a missing or malformed sidecar must not block
        // byte-level loading.
        Self::ensure_guid_identity(&path, resources, type_name::<T>());

        let loader = self
            .loaders
            .get(&TypeId::of::<T>())
            .ok_or_else(|| AssetError::NoLoaderForType(type_name::<T>()))?;

        // Match the file name's lowercased basename against every
        // suffix the loader claims. Single-segment extensions
        // (`"glb"`, `"png"`) match `Path::extension`; compound
        // extensions (`"kooch_material.ron"`) match the trailing
        // segment of the file name. Both cases compare as a
        // suffix of the lowercased name with a `.` separator
        // prepended, so `"glb"` does not accidentally match
        // `foo.fxglb`.
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

        let bytes = std::fs::read(&path)?;
        let mut ctx = LoadContext { path: &path };
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

    /// Loads an asset of type `T` referenced by [`Guid`]. The
    /// [`AssetDatabase`] resource must hold an entry for the GUID
    /// (typically populated by [`AssetDatabase::scan_directory`] at
    /// startup, or by a prior [`AssetServer::load`] call that triggered
    /// `.meta` registration).
    ///
    /// Internally resolves `guid → path` and delegates to
    /// [`AssetServer::load`], which means the per-path cache short-
    /// circuits repeat calls — `load_by_guid` of an already-loaded
    /// asset returns the cached handle without re-reading bytes.
    ///
    /// # Errors
    ///
    /// - [`AssetError::MissingAssetStorage`] when no `AssetDatabase`
    ///   resource is present.
    /// - [`AssetError::UnknownGuid`] when the database has no entry
    ///   for `guid`.
    /// - Any error [`AssetServer::load`] would surface (loader
    ///   missing, bytes unreadable, etc.).
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

    /// Forgets the cached handle for one path, so the next load re-reads
    /// it from disk.
    ///
    /// Targeted rather than [`Self::clear_cache`] because one file
    /// changing is not a reason to make every other asset load again.
    pub fn forget<T: Asset>(&mut self, path: impl AsRef<Path>) {
        let path = self.resolve_path(path.as_ref());
        self.cache.remove(&(TypeId::of::<T>(), path));
    }

    /// Re-reads `path` from disk and overwrites the assets already loaded
    /// from it, keeping their handles valid. Returns how many were
    /// refreshed.
    ///
    /// # Why not `forget` + `load`
    ///
    /// [`Self::forget`] drops the cache entry so the *next* load re-reads
    /// the file — but that load calls `Assets::insert`, which mints a new
    /// key. Everything already holding a `Handle<T>` (a component field,
    /// an `AssetRef`, a live instance) goes on resolving to the copy from
    /// before the edit. The file would be re-read and nothing on screen
    /// would change. Writing over the existing slot is what makes the
    /// edit visible, and it is why this is a server method rather than
    /// something each caller assembles.
    ///
    /// # Not knowing the type is the point
    ///
    /// The caller is a save handler or a message off the wire; all it has
    /// is a path. The cache is keyed by `(TypeId, path)`, so every type
    /// that ever loaded this file is found here and refreshed — a path
    /// loaded under two types refreshes both.
    ///
    /// A path nothing ever loaded returns `Ok(0)`: not an error, just
    /// nothing cached to update. Handles whose slot has since been
    /// removed are dropped from the cache rather than reported.
    ///
    /// # Errors
    ///
    /// - [`AssetError::Io`] when the file cannot be read.
    /// - [`AssetError::Loader`] when it no longer parses. The previous
    ///   asset stays in place — a broken save does not blank what is
    ///   loaded — and types refreshed before the failure keep their new
    ///   contents.
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
        let bytes = std::fs::read(&path)?;
        let mut reloaded = 0usize;
        let mut stale = Vec::new();
        for (type_id, key) in cached {
            let Some(loader) = self.loaders.get(&type_id) else {
                // The type was loaded by a build that had this loader and
                // this one does not — nothing to refresh it with.
                continue;
            };
            let mut ctx = LoadContext { path: &path };
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

    /// Guarantees that `path` has a `.meta` sidecar with a stable
    /// [`Guid`] and the recorded `asset_type` set to `type_name`, and
    /// that, if an `AssetDatabase` resource is present, the resulting
    /// `(guid, path, type_name)` triple is registered. Best-effort:
    /// missing source file is a no-op; sidecar I/O errors are logged
    /// but not surfaced (callers care about asset bytes, not metadata).
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
        // The sidecar's recorded type wins. If the loader was the
        // first to assign one, `read_or_create_typed` already
        // back-filled it; if a previous load with a different `T`
        // wrote a different type, we honour that here and the
        // caller's `Assets<T>` insert downstream will fail loudly
        // (downcast mismatch) rather than us silently relabel.
        db.register(
            meta.guid,
            AssetEntry {
                path: path.to_path_buf(),
                mtime,
                type_name: meta.asset_type.clone(),
            },
        );
        // Re-entrant safety: if the entry already existed under the
        // same GUID with no type yet (scanned at startup before any
        // `load::<T>`), `register` keeps the freshly-typed entry; if
        // it already had a type, the new entry's type matches what
        // the sidecar carries.
    }

    /// Resolves a caller-provided path against the configured asset
    /// root. Absolute paths bypass the root and pass through unchanged;
    /// relative paths are joined onto `asset_root`. Public so other
    /// systems (scene loaders, asset pickers) can mirror the same
    /// resolution rule when looking up entries in `AssetDatabase`.
    pub fn resolve_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else if let Some(root) = &self.asset_root {
            root.join(path)
        } else {
            path.to_path_buf()
        }
    }
}

impl Default for AssetServer {
    fn default() -> Self {
        Self::new()
    }
}
