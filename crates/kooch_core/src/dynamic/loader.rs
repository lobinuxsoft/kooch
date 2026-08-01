//! Loading plugin libraries, and refusing the ones that would be unsound.
//!
//! [`PluginLoader`] opens `.so`/`.dll` files with `libloading`, checks the
//! build stamp, constructs the plugin, and keeps the library alive.
//!
//! # Why the stamp is read first
//!
//! The constructor hands back a `Box<dyn KoochPlugin>` — a Rust trait
//! object, whose vtable layout Rust does not guarantee across compiler
//! versions. Calling it when the plugin was built by a different
//! compiler is undefined behaviour, and it would look like a working
//! load right up until a method call jumps somewhere else.
//!
//! So the stamp symbol is looked up and compared **before** the
//! constructor is even resolved. It is a `#[repr(C)]` struct of two
//! integers, which is decodable regardless of whether anything else
//! would have been.

use std::path::{Path, PathBuf};

use libloading::Library;

use kooch_plugin_api::KoochPlugin;
use kooch_plugin_api::plugin::{CREATE_SYMBOL, CreatePluginFn, STAMP_SYMBOL};
use kooch_plugin_api::version::{BuildStamp, Incompatibility};

/// Signature of the build-stamp symbol every plugin exports.
type BuildStampFn = unsafe extern "C" fn() -> BuildStamp;

/// Holds loaded libraries and keeps them alive.
///
/// A library must outlive every plugin that came out of it: dropping it
/// unmaps the code the plugin's vtable points into.
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
        Self { loaded: Vec::new() }
    }

    /// Loads a plugin from the library at `path`.
    ///
    /// The build stamp is verified before the constructor is called; an
    /// incompatible library is refused without executing any of its
    /// Rust code.
    ///
    /// # Safety
    ///
    /// Opening a library runs its initialisers, which is arbitrary code.
    /// Only load libraries you built.
    ///
    /// # Errors
    ///
    /// If the library cannot be opened, either symbol is missing, or the
    /// stamp does not match this engine's.
    pub unsafe fn load(&mut self, path: &Path) -> Result<Box<dyn KoochPlugin>, PluginLoadError> {
        let library = unsafe {
            Library::new(path).map_err(|e| PluginLoadError::LibraryOpen {
                path: path.to_path_buf(),
                source: e.to_string(),
            })?
        };

        // Before anything else: prove this library is safe to call into.
        let stamp = unsafe {
            let symbol =
                library
                    .get::<BuildStampFn>(STAMP_SYMBOL)
                    .map_err(|e| PluginLoadError::Symbol {
                        path: path.to_path_buf(),
                        symbol: String::from_utf8_lossy(STAMP_SYMBOL).into_owned(),
                        source: e.to_string(),
                    })?;
            symbol()
        };

        if let Some(reason) = stamp.incompatibility() {
            return Err(PluginLoadError::Incompatible {
                path: path.to_path_buf(),
                reason,
            });
        }

        let plugin = unsafe {
            let constructor = library.get::<CreatePluginFn>(CREATE_SYMBOL).map_err(|e| {
                PluginLoadError::Symbol {
                    path: path.to_path_buf(),
                    symbol: String::from_utf8_lossy(CREATE_SYMBOL).into_owned(),
                    source: e.to_string(),
                }
            })?;
            constructor()
        };

        tracing::info!(
            plugin = plugin.name(),
            path = %path.display(),
            "loaded plugin"
        );

        self.loaded.push(LoadedPlugin {
            path: path.to_path_buf(),
            _library: library,
        });

        Ok(plugin)
    }

    /// Number of libraries currently held open.
    #[inline]
    pub fn count(&self) -> usize {
        self.loaded.len()
    }

    /// Paths of the libraries currently held open.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.loaded.iter().map(|lp| lp.path.as_path())
    }
}

impl Default for PluginLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// Why a plugin could not be loaded.
#[derive(Debug)]
pub enum PluginLoadError {
    /// The library could not be opened.
    LibraryOpen {
        /// Library that failed to open.
        path: PathBuf,
        /// What the loader reported.
        source: String,
    },
    /// A required symbol was missing.
    Symbol {
        /// Library that lacked it.
        path: PathBuf,
        /// Symbol that was looked up.
        symbol: String,
        /// What the loader reported.
        source: String,
    },
    /// The library was not built for this engine.
    Incompatible {
        /// Library that was refused.
        path: PathBuf,
        /// Which half of the stamp failed to match.
        reason: Incompatibility,
    },
}

impl std::fmt::Display for PluginLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LibraryOpen { path, source } => {
                write!(f, "failed to open library {}: {source}", path.display())
            }
            Self::Symbol {
                path,
                symbol,
                source,
            } => write!(
                f,
                "{} does not export {symbol}: {source} — is it built with export_plugin!?",
                path.display()
            ),
            Self::Incompatible { path, reason } => {
                write!(f, "refused to load {}: {reason}", path.display())
            }
        }
    }
}

impl std::error::Error for PluginLoadError {}
