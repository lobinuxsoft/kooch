//! Proving a plugin was built against this engine by this compiler: a trait object's vtable layout
//! is only stable within one compiler, so [`BuildStamp`] is compared before anything is called.

/// Current plugin API version; increment on any breaking change to [`Engine`](crate::Engine),
/// [`KoochPlugin`](crate::KoochPlugin) or the schema types.
pub const API_VERSION: u32 = 5;

/// The engine's version, shared through `version.workspace = true`. 🔴 Separate from
/// [`API_VERSION`]: field layouts change with no signature change, and vendored engines drift
/// (#754).
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The compiler that built this crate, as `rustc -V -v` reported it.
///
/// Captured by the build script; newlines flattened to `|`.
pub const RUSTC_IDENT: &str = env!("KOOCH_RUSTC_IDENT");

/// Identity of the API and compiler a binary was built with — `#[repr(C)]`, since it is read before
/// compatibility is known.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildStamp {
    /// Value of [`API_VERSION`] at build time.
    pub api_version: u32,
    /// Hash of [`RUSTC_IDENT`] at build time.
    pub rustc_hash: u64,
    /// Hash of [`ENGINE_VERSION`] at build time, fixed-size because this struct crosses the
    /// boundary before compatibility is known.
    pub engine_hash: u64,
}

impl BuildStamp {
    /// The stamp of the binary calling this.
    pub const fn current() -> Self {
        Self {
            api_version: API_VERSION,
            rustc_hash: fnv1a(RUSTC_IDENT.as_bytes()),
            engine_hash: fnv1a(ENGINE_VERSION.as_bytes()),
        }
    }

    #[cfg(test)]
    /// Whether a plugin carrying this stamp may be loaded here.
    pub const fn is_compatible_with_current(&self) -> bool {
        let current = Self::current();
        self.api_version == current.api_version
            && self.rustc_hash == current.rustc_hash
            && self.engine_hash == current.engine_hash
    }

    /// Why it is incompatible, or `None`: an API mismatch means rebuild against this engine, a
    /// compiler mismatch means rebuild with this toolchain.
    pub fn incompatibility(&self) -> Option<Incompatibility> {
        let current = Self::current();
        if self.api_version != current.api_version {
            return Some(Incompatibility::ApiVersion {
                plugin: self.api_version,
                engine: current.api_version,
            });
        }
        if self.rustc_hash != current.rustc_hash {
            return Some(Incompatibility::Compiler);
        }
        if self.engine_hash != current.engine_hash {
            return Some(Incompatibility::EngineVersion {
                engine: ENGINE_VERSION,
            });
        }
        None
    }
}

/// How a plugin's [`BuildStamp`] failed to match the host's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Incompatibility {
    /// Built against a different API version.
    ApiVersion {
        /// What the plugin was built against.
        plugin: u32,
        /// What this engine expects.
        engine: u32,
    },
    /// Built by a different compiler.
    Compiler,
    /// Built against a different engine version, with the same API and
    /// compiler — so it links, and the layouts underneath may not match.
    EngineVersion {
        /// What this engine is. The plugin's is unrecoverable: the stamp
        /// carries a hash so it can stay fixed-size.
        engine: &'static str,
    },
}

impl std::fmt::Display for Incompatibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiVersion { plugin, engine } => write!(
                f,
                "plugin was built against API version {plugin}, this engine speaks {engine} — \
                 rebuild the plugin"
            ),
            Self::Compiler => write!(
                f,
                "plugin was built by a different compiler than the engine — a Rust trait object \
                 cannot safely cross that boundary; rebuild both with the same toolchain"
            ),
            Self::EngineVersion { engine } => write!(
                f,
                "plugin was built against a different engine version; this one is {engine}. The \
                 API matches, so it would load and read every shared structure at whatever \
                 layout it was compiled with — rebuild the project against the engine the \
                 editor vendored"
            ),
        }
    }
}

/// FNV-1a over bytes, `const` so a stamp builds at compile time and stays a fixed-size `#[repr(C)]`
/// value.
const fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        i += 1;
    }
    hash
}

#[cfg(test)]
mod tests;
