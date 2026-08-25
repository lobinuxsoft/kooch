//! What a build preset targets, as a platform rather than a triple.
//!
//! # Why not the triple
//!
//! A Rust target triple — `x86_64-pc-windows-gnu` — is a string somebody
//! has to know exists and spell exactly. It was never really the
//! question being asked: the pipeline read it back into a platform
//! everywhere it mattered, deciding the `.exe` extension with
//! `contains("windows")`, the mingw `CFLAGS` with
//! `contains("windows-gnu")`, and whether a glibc floor applies with
//! `contains("linux-gnu")`. Three string searches for one fact the
//! author already knew when they picked the preset.
//!
//! So the fact is stored, and the triple is derived from it.
//!
//! # Why architecture is not part of it
//!
//! Every variant here is x86_64. An `aarch64` toggle would be a
//! checkbox that fails: nothing in the engine builds for it today, and
//! offering it would make the preset promise something it cannot do.
//! When there is a device to build for, this enum gains a variant and
//! every preset on disk keeps loading — which is the point of storing a
//! platform rather than a string.

/// A platform a preset can build for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Platform {
    Linux,
    Windows,
}

impl Platform {
    /// Every platform, in the order a build runs them.
    pub const ALL: [Platform; 2] = [Platform::Linux, Platform::Windows];

    /// The triple cargo is told to build for.
    pub fn triple(self) -> &'static str {
        match self {
            Platform::Linux => "x86_64-unknown-linux-gnu",
            Platform::Windows => "x86_64-pc-windows-gnu",
        }
    }

    /// The subfolder of the preset's output directory this lands in.
    ///
    /// Lowercase and one word: it becomes a path, and a build folder
    /// copied between machines should not depend on case sensitivity.
    pub fn folder(self) -> &'static str {
        match self {
            Platform::Linux => "linux",
            Platform::Windows => "windows",
        }
    }

    /// What the platform is called in the editor.
    pub fn label(self) -> &'static str {
        match self {
            Platform::Linux => "Linux",
            Platform::Windows => "Windows",
        }
    }

    /// The extension the executable takes, `""` for none.
    ///
    /// # Why Linux gets one at all
    ///
    /// It does not need one — an ELF is executable because of its mode
    /// and its header, not its name. `.x86_64` is the convention Unity
    /// and Godot use in their Linux exports, and it earns its place the
    /// moment a folder holds more than one platform: `game.exe` beside
    /// a bare `game` is ambiguous about which is which, and about
    /// whether the second one is a script.
    ///
    /// ⚠️ Never `.sh`. That is a text script the shell interprets; this
    /// is a compiled binary, and the extension would make some file
    /// managers offer to open it in an editor.
    pub fn extension(self) -> &'static str {
        match self {
            // Fixed rather than read from the triple: every variant here
            // is x86_64 by construction, so there is nothing to read.
            Platform::Linux => ".x86_64",
            Platform::Windows => ".exe",
        }
    }

    /// Whether a glibc floor means anything for this platform.
    ///
    /// Windows has no glibc, and `cargo zigbuild` rejects
    /// `x86_64-pc-windows-gnu.2.28` outright — so a preset that sets a
    /// floor and builds both platforms must not carry it into the
    /// Windows half.
    pub fn takes_glibc_floor(self) -> bool {
        matches!(self, Platform::Linux)
    }

    /// The platform the editor is running on, `None` on one this cannot
    /// build for at all.
    pub fn host() -> Option<Self> {
        match () {
            _ if cfg!(target_os = "linux") => Some(Platform::Linux),
            _ if cfg!(target_os = "windows") => Some(Platform::Windows),
            _ => None,
        }
    }

    /// The platform a target triple names.
    ///
    /// Only used to read presets written before this existed — see
    /// `LegacyTarget`. An empty triple meant "this machine", which is
    /// [`Self::host`] rather than anything found here.
    pub fn from_triple(triple: &str) -> Option<Self> {
        let triple = triple.trim();
        match () {
            _ if triple.contains("windows") => Some(Platform::Windows),
            _ if triple.contains("linux") => Some(Platform::Linux),
            _ => None,
        }
    }
}

#[cfg(test)]
mod platform_tests;
