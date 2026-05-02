//! Asset loader trait + `AssetServer` resource.
//!
//! Defines the contract every asset loader implements ([`AssetLoader<T>`]),
//! a load-time context the loader uses to side-load dependencies
//! ([`LoadContext`]), and the central registry the engine consults when
//! game code requests an asset by path ([`AssetServer`]).
//!
//! # Pipeline
//!
//! ```text
//! game code ──load("mesh.glb")──▶ AssetServer
//!                                      │
//!                                      ▼
//!                          read bytes from filesystem
//!                                      │
//!                                      ▼
//!                       lookup AssetLoader<Mesh> by TypeId
//!                                      │
//!                                      ▼
//!                          loader.load(bytes, ctx) ─▶ Mesh
//!                                      │
//!                                      ▼
//!                  Assets<Mesh>.insert(mesh) ─▶ Handle<Mesh>
//!                                      │
//!                                      ▼
//!                       cache (path -> handle) for next load
//! ```
//!
//! # Out of scope (other issues)
//!
//! - Concrete loaders (glTF #129, image #131, RON scene): each lives with
//!   its asset type's load issue.
//! - Async / background loading: arrives when streaming demands it.
//! - Hot-reload: file-watcher integration is its own feature.

use crate::assets::{Asset, Assets, Handle};
use crate::resource::Resources;
use std::any::{Any, TypeId, type_name};
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

/// Result alias for asset operations.
pub type AssetResult<T> = Result<T, AssetError>;

/// Errors produced during load. Loader implementations wrap their domain
/// errors in [`AssetError::Loader`].
#[derive(Debug)]
pub enum AssetError {
    /// File could not be opened or read.
    Io(std::io::Error),
    /// No loader registered for the requested asset type.
    NoLoaderForType(&'static str),
    /// File extension is not supported by the registered loader.
    UnsupportedExtension {
        path: PathBuf,
        registered: Vec<&'static str>,
    },
    /// `Assets<T>` storage was missing from the resource set when the
    /// load completed. Caller forgot to insert it before driving a load.
    MissingAssetStorage(&'static str),
    /// Domain error returned by the loader itself (parser failure, etc.).
    Loader(Box<dyn std::error::Error + Send + Sync>),
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetError::Io(e) => write!(f, "asset I/O failed: {e}"),
            AssetError::NoLoaderForType(name) => {
                write!(f, "no AssetLoader registered for type `{name}`")
            }
            AssetError::UnsupportedExtension { path, registered } => write!(
                f,
                "loader does not support extension of {path:?}; registered = {registered:?}",
            ),
            AssetError::MissingAssetStorage(name) => {
                write!(f, "Assets<{name}> resource missing — insert it first")
            }
            AssetError::Loader(e) => write!(f, "loader error: {e}"),
        }
    }
}

impl std::error::Error for AssetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AssetError::Io(e) => Some(e),
            AssetError::Loader(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<std::io::Error> for AssetError {
    fn from(err: std::io::Error) -> Self {
        AssetError::Io(err)
    }
}

/// Per-load context handed to the loader's `load` call.
///
/// Carries the source path (for error messages, relative path resolution)
/// and a slot the loader can stash sub-assets into. PR-1 keeps this minimal;
/// future iterations grow it (texture loader returning a `Handle<Image>`,
/// glTF loader producing a `Handle<Material>` per primitive, etc.).
pub struct LoadContext<'a> {
    /// Absolute path the bytes came from.
    pub path: &'a Path,
}

/// Trait every asset loader implements.
///
/// Generic over the asset type `T` so a `MeshLoader` returns `Mesh`, an
/// `ImageLoader` returns `Image`, etc. `Send + Sync + 'static` lets the
/// `AssetServer` store them across threads when async/streaming arrives.
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
trait UntypedLoader: Send + Sync {
    fn extensions(&self) -> &[&'static str];
    fn load_boxed(
        &self,
        bytes: &[u8],
        ctx: &mut LoadContext<'_>,
    ) -> AssetResult<Box<dyn Any + Send + Sync>>;
    fn asset_type_name(&self) -> &'static str;
    fn asset_type_id(&self) -> TypeId;
}

/// Adapter that bridges a concrete `AssetLoader<T>` into the type-erased
/// `UntypedLoader` storage. Owns the loader and erases `T` for storage.
struct TypedLoader<L, T>
where
    L: AssetLoader<T>,
    T: Asset,
{
    inner: L,
    _marker: PhantomData<fn() -> T>,
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
}

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

        let loader = self
            .loaders
            .get(&TypeId::of::<T>())
            .ok_or_else(|| AssetError::NoLoaderForType(type_name::<T>()))?;

        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase());
        let supports = match extension.as_deref() {
            Some(ext) => loader
                .extensions()
                .iter()
                .any(|e| e.eq_ignore_ascii_case(ext)),
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

    fn resolve_path(&self, path: &Path) -> PathBuf {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[derive(Debug, PartialEq)]
    struct PlainText(String);

    /// Loader that copies bytes into a UTF-8 string.
    struct TextLoader;
    impl AssetLoader<PlainText> for TextLoader {
        fn extensions(&self) -> &[&'static str] {
            &["txt", "log"]
        }

        fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<PlainText> {
            let s = std::str::from_utf8(bytes)
                .map_err(|e| AssetError::Loader(Box::new(e)))?
                .to_string();
            Ok(PlainText(s))
        }
    }

    #[derive(Debug, PartialEq)]
    struct Other(u32);

    fn temp_file_with(content: &str, ext: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let filename = format!(
            "ome_assetloader_test_{}.{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            ext,
        );
        path.push(filename);
        let mut f = std::fs::File::create(&path).expect("temp file");
        f.write_all(content.as_bytes()).expect("write temp");
        path
    }

    #[test]
    fn register_then_has_loader() {
        let mut server = AssetServer::new();
        assert!(!server.has_loader::<PlainText>());
        server.register_loader::<PlainText, _>(TextLoader);
        assert!(server.has_loader::<PlainText>());
    }

    #[test]
    fn extensions_reflect_registered_loader() {
        let mut server = AssetServer::new();
        server.register_loader::<PlainText, _>(TextLoader);
        let exts = server.extensions_for::<PlainText>();
        assert_eq!(exts, &["txt", "log"]);
    }

    #[test]
    fn extensions_empty_when_no_loader() {
        let server = AssetServer::new();
        assert!(server.extensions_for::<PlainText>().is_empty());
    }

    #[test]
    fn load_without_registered_loader_errs() {
        let mut server = AssetServer::new();
        let mut resources = Resources::new();
        resources.insert(Assets::<PlainText>::new());
        let err = server
            .load::<PlainText>("anything.txt", &mut resources)
            .unwrap_err();
        match err {
            AssetError::NoLoaderForType(name) => {
                assert!(name.contains("PlainText"));
            }
            other => panic!("expected NoLoaderForType, got {other:?}"),
        }
    }

    #[test]
    fn load_with_unsupported_extension_errs() {
        let mut server = AssetServer::new();
        server.register_loader::<PlainText, _>(TextLoader);
        let mut resources = Resources::new();
        resources.insert(Assets::<PlainText>::new());
        let path = temp_file_with("data", "bin");
        let err = server.load::<PlainText>(&path, &mut resources).unwrap_err();
        match err {
            AssetError::UnsupportedExtension { registered, .. } => {
                assert_eq!(registered, vec!["txt", "log"]);
            }
            other => panic!("expected UnsupportedExtension, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_succeeds_and_caches_handle() {
        let mut server = AssetServer::new();
        server.register_loader::<PlainText, _>(TextLoader);
        let mut resources = Resources::new();
        resources.insert(Assets::<PlainText>::new());
        let path = temp_file_with("hello world", "txt");

        let h1 = server.load::<PlainText>(&path, &mut resources).unwrap();
        let h2 = server.load::<PlainText>(&path, &mut resources).unwrap();
        assert_eq!(h1, h2, "second load should hit cache, not re-insert");

        let assets = resources.get::<Assets<PlainText>>().unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets.get(h1).map(|t| t.0.as_str()), Some("hello world"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn get_cached_returns_none_before_load() {
        let server = AssetServer::new();
        let path = std::env::temp_dir().join("never_loaded.txt");
        assert!(server.get_cached::<PlainText>(&path).is_none());
    }

    #[test]
    fn get_cached_returns_handle_after_load() {
        let mut server = AssetServer::new();
        server.register_loader::<PlainText, _>(TextLoader);
        let mut resources = Resources::new();
        resources.insert(Assets::<PlainText>::new());
        let path = temp_file_with("cached", "log");

        let handle = server.load::<PlainText>(&path, &mut resources).unwrap();
        let cached = server.get_cached::<PlainText>(&path).unwrap();
        assert_eq!(handle, cached);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_assets_storage_errs() {
        let mut server = AssetServer::new();
        server.register_loader::<PlainText, _>(TextLoader);
        let mut resources = Resources::new(); // no Assets<PlainText> inserted
        let path = temp_file_with("data", "txt");

        let err = server.load::<PlainText>(&path, &mut resources).unwrap_err();
        match err {
            AssetError::MissingAssetStorage(name) => {
                assert!(name.contains("PlainText"));
            }
            other => panic!("expected MissingAssetStorage, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn loader_per_type_is_independent() {
        struct OtherLoader;
        impl AssetLoader<Other> for OtherLoader {
            fn extensions(&self) -> &[&'static str] {
                &["bin"]
            }
            fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<Other> {
                if bytes.len() < 4 {
                    return Err(AssetError::Loader("need 4 bytes".into()));
                }
                let value = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                Ok(Other(value))
            }
        }

        let mut server = AssetServer::new();
        server.register_loader::<PlainText, _>(TextLoader);
        server.register_loader::<Other, _>(OtherLoader);

        assert!(server.has_loader::<PlainText>());
        assert!(server.has_loader::<Other>());
        assert_eq!(server.extensions_for::<PlainText>(), &["txt", "log"]);
        assert_eq!(server.extensions_for::<Other>(), &["bin"]);
    }

    #[test]
    fn relative_path_resolves_against_root() {
        let server = AssetServer::new().with_asset_root("/tmp/some/root");
        let resolved = server.resolve_path(Path::new("models/cube.glb"));
        assert_eq!(resolved, PathBuf::from("/tmp/some/root/models/cube.glb"));
    }

    #[test]
    fn absolute_path_bypasses_root() {
        let server = AssetServer::new().with_asset_root("/tmp/some/root");
        let resolved = server.resolve_path(Path::new("/etc/passwd"));
        assert_eq!(resolved, PathBuf::from("/etc/passwd"));
    }

    #[test]
    fn no_root_keeps_path_raw() {
        let server = AssetServer::new();
        let resolved = server.resolve_path(Path::new("models/cube.glb"));
        assert_eq!(resolved, PathBuf::from("models/cube.glb"));
    }

    #[test]
    fn clear_cache_drops_path_lookup_only() {
        let mut server = AssetServer::new();
        server.register_loader::<PlainText, _>(TextLoader);
        let mut resources = Resources::new();
        resources.insert(Assets::<PlainText>::new());
        let path = temp_file_with("data", "txt");

        let _h = server.load::<PlainText>(&path, &mut resources).unwrap();
        assert!(server.get_cached::<PlainText>(&path).is_some());
        server.clear_cache();
        assert!(server.get_cached::<PlainText>(&path).is_none());

        // Storage untouched
        let assets = resources.get::<Assets<PlainText>>().unwrap();
        assert_eq!(assets.len(), 1);
        let _ = std::fs::remove_file(&path);
    }
}
