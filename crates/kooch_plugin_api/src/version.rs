//! Proving a plugin was built against this engine, by this compiler.
//!
//! The plugin hands the host a `Box<dyn KoochPlugin>` — a Rust trait
//! object, whose vtable layout Rust does not guarantee between compiler
//! versions. Passing one across a library boundary is sound *only* when
//! both sides were built by the same compiler against the same API.
//!
//! Nothing enforces that on its own, so [`BuildStamp`] records it and
//! the loader compares before calling anything. A mismatch is a refusal
//! with a message, which is the alternative to a jump through a vtable
//! that means something else.

/// Current plugin API version.
///
/// Increment on any breaking change to [`Engine`](crate::Engine),
/// [`KoochPlugin`](crate::KoochPlugin), or the schema types.
pub const API_VERSION: u32 = 5;

/// The engine's version, which this crate shares through
/// `version.workspace = true`.
///
/// 🔴 Separate from [`API_VERSION`] on purpose. That one is bumped by
/// hand when the plugin *interface* changes; this one moves with every
/// engine release, including the ones that change a component's fields
/// without touching a single signature in this crate. A plugin built
/// against those still links, still passes the API check, and hands over
/// structures whose layout the host reads differently.
///
/// It matters more since the engine is vendored into projects (#754):
/// the editor's engine and a project's copy are now two directories that
/// can drift apart.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The compiler that built this crate, as `rustc -V -v` reported it.
///
/// Captured by the build script; newlines flattened to `|`.
pub const RUSTC_IDENT: &str = env!("KOOCH_RUSTC_IDENT");

/// Identity of the API and compiler a binary was built with.
///
/// `#[repr(C)]` because it is returned across the boundary by the one
/// symbol that is read *before* compatibility is known — it has to be
/// decodable even when nothing else would be.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildStamp {
    /// Value of [`API_VERSION`] at build time.
    pub api_version: u32,
    /// Hash of [`RUSTC_IDENT`] at build time.
    pub rustc_hash: u64,
    /// Hash of [`ENGINE_VERSION`] at build time.
    ///
    /// A hash and not the string for the same reason as `rustc_hash`:
    /// this struct crosses the boundary *before* compatibility is known,
    /// so it has to be fixed-size and `#[repr(C)]`.
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

    /// Whether a plugin carrying this stamp may be loaded here.
    pub const fn is_compatible_with_current(&self) -> bool {
        let current = Self::current();
        self.api_version == current.api_version
            && self.rustc_hash == current.rustc_hash
            && self.engine_hash == current.engine_hash
    }

    /// Why it is incompatible, or `None` if it is fine.
    ///
    /// Separated from the predicate so the loader can say which half
    /// failed: a version mismatch means rebuild against this engine, a
    /// compiler mismatch means rebuild with this toolchain. They look
    /// identical from a boolean and have different fixes.
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

/// FNV-1a over bytes, `const` so a stamp can be built at compile time.
///
/// A hash rather than the string itself keeps [`BuildStamp`] a fixed-size
/// `#[repr(C)]` value, which matters because it crosses the boundary
/// before compatibility has been established.
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
