//! Dynamic plugin loader using `libloading` and `stabby` ABI verification.
//!
//! [`PluginLoader`] loads `.dll`/`.so` files, verifies ABI compatibility via
//! stabby, instantiates the plugin, and keeps the library alive.

use std::path::{Path, PathBuf};

use libloading::Library;
use stabby::libloading::StabbyLibrary;

use ome_plugin_api::plugin::{BoxedPlugin, CreatePluginFn, OmePluginDyn};

/// Holds loaded dynamic libraries and their plugin instances.
///
/// Libraries must outlive their plugins — dropping a `PluginLoader` unloads
/// the libraries which invalidates all function pointers.
pub struct PluginLoader {
    loaded: Vec<LoadedPlugin>,
}

struct LoadedPlugin {
    path: PathBuf,
    _library: Library,
}

impl PluginLoader {
    /// Creates an empty loader.
    pub fn new() -> Self {
        Self {
            loaded: Vec::new(),
        }
    }

    /// Loads a plugin from a shared library at `path`.
    ///
    /// 1. Opens the library with `libloading`
    /// 2. Looks up `ome_create_plugin` with stabby ABI verification
    /// 3. Calls the constructor to get a `Box<dyn OmePlugin>`
    /// 4. Verifies API version compatibility
    /// 5. Returns the plugin instance (library is kept alive internally)
    ///
    /// # Safety
    ///
    /// Loading a shared library executes arbitrary code. Only load trusted plugins.
    ///
    /// # Errors
    ///
    /// Returns an error if the library can't be loaded, the symbol is missing,
    /// ABI verification fails, or the plugin reports an incompatible version.
    pub unsafe fn load(
        &mut self,
        path: &Path,
    ) -> Result<BoxedPlugin, PluginLoadError> {
        let library = unsafe {
            Library::new(path).map_err(|e| PluginLoadError::LibraryOpen {
                path: path.to_path_buf(),
                source: e.to_string(),
            })?
        };

        // stabby verifies ABI compatibility via the companion `_stabbied` symbol.
        let constructor = unsafe {
            library
                .get_stabbied::<CreatePluginFn>(b"ome_create_plugin")
                .map_err(|e| PluginLoadError::Symbol {
                    path: path.to_path_buf(),
                    source: e.to_string(),
                })?
        };

        let plugin = constructor();

        // Version check.
        let version = plugin.api_version();
        if !ome_plugin_api::version::is_compatible(version) {
            return Err(PluginLoadError::IncompatibleVersion {
                path: path.to_path_buf(),
                plugin_version: version,
                engine_version: ome_plugin_api::version::API_VERSION,
            });
        }

        let name: String = plugin.name().into();
        tracing::info!(
            plugin = %name,
            version,
            path = %path.display(),
            "Loaded dynamic plugin"
        );

        self.loaded.push(LoadedPlugin {
            path: path.to_path_buf(),
            _library: library,
        });

        Ok(plugin)
    }

    /// Returns the number of loaded plugins.
    #[inline]
    pub fn count(&self) -> usize {
        self.loaded.len()
    }

    /// Returns the paths of all loaded plugin libraries.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.loaded.iter().map(|lp| lp.path.as_path())
    }
}

impl Default for PluginLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that can occur when loading a dynamic plugin.
#[derive(Debug)]
pub enum PluginLoadError {
    /// Failed to open the shared library.
    LibraryOpen {
        path: PathBuf,
        source: String,
    },
    /// Failed to find or verify the `ome_create_plugin` symbol.
    Symbol {
        path: PathBuf,
        source: String,
    },
    /// Plugin API version is incompatible with the engine.
    IncompatibleVersion {
        path: PathBuf,
        plugin_version: u32,
        engine_version: u32,
    },
}

impl std::fmt::Display for PluginLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LibraryOpen { path, source } => {
                write!(f, "failed to open library {}: {source}", path.display())
            }
            Self::Symbol { path, source } => {
                write!(
                    f,
                    "failed to load ome_create_plugin from {}: {source}",
                    path.display()
                )
            }
            Self::IncompatibleVersion {
                path,
                plugin_version,
                engine_version,
            } => {
                write!(
                    f,
                    "plugin {} has API version {plugin_version}, engine expects {engine_version}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for PluginLoadError {}
