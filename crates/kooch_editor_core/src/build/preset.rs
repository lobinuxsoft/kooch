//! `.buildpreset` — what "make a build" means for one target (#758).
//!
//! Modelled on Godot's export presets, read from
//! `editor/export/editor_export_preset.h` rather than from memory. What
//! was worth taking:
//!
//! - **Named presets, several per project**, saved with the project. Not
//!   one global configuration: a project has "Windows release", "Linux
//!   debug", "handheld", and they differ in more than one field.
//! - **A `runnable` one**, so one click has something to deploy.
//! - An output path per preset.
//!
//! # Why it is an asset
//!
//! Because `register_reflected_asset!` exists (#744): a `.buildpreset`
//! is edited in the Inspector with tooltips generated from these doc
//! comments, and the panel only has to provide what the Inspector cannot
//! — the list, the button, and cargo's output.
//!
//! Godot's own answer to the platform-specific half is a dynamic property
//! map. Ours is flat fields, for the same reason `RenderSettings` is
//! flat: the generic editor draws them, and each one states its unit.
//!
//! # 🔴 The encryption key is not here
//!
//! A preset is configuration and belongs in version control. The key does
//! not — a repository carrying it has published it. It lives in
//! [`super::key`], outside the preset and outside git, which is the same
//! line Godot draws between `export_presets.cfg` and its encryption key.

use kooch_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};
use kooch_ecs::Reflect;
use serde::{Deserialize, Serialize};

/// Extension a build preset carries.
pub const BUILD_PRESET_EXTENSION: &str = "buildpreset";

/// The feature that compiles the profiler into a game.
///
/// 🔴 Reached **through the dependency**, not as a feature of the
/// project's own crate. `cargo build --features kooch/profiling` works
/// against any project that depends on the engine; a bare `profiling`
/// would need every project's `Cargo.toml` to declare a forwarding
/// feature, so ticking the box in a project made last month would fail
/// with "does not have feature `profiling`" ten minutes into a build.
///
/// The manifest the editor generates is only written when a project is
/// created (`generate_cargo_toml`), so anything that requires a new one
/// silently excludes every project that already exists.
const PROFILING_FEATURE: &str = "kooch/profiling";

/// What someone types into the features field meaning the same thing.
///
/// Dropped alongside the qualified name so ticking the box and typing it
/// do not ask cargo for the feature twice.
const PROFILING_SHORTHAND: &str = "profiling";

/// One way of building this project.
#[derive(Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(category = "Build")]
pub struct BuildPreset {
    /// Rust target triple to compile for, e.g.
    /// `x86_64-unknown-linux-gnu` or `x86_64-pc-windows-gnu`.
    ///
    /// Empty builds for this machine. A triple that is not installed
    /// fails before compiling, with what to run to install it — cargo's
    /// own error arrives ten minutes in and names a linker.
    #[serde(default)]
    pub target_triple: String,

    /// Folder the build is written to, relative to the project.
    ///
    /// Everything the game needs lands here together: the executable,
    /// `scenes/`, and the asset pack.
    #[serde(default = "default_output_dir")]
    pub output_dir: String,

    /// Name of the produced executable, without an extension.
    ///
    /// Empty uses the crate's own name. The extension is added for you
    /// and follows the target: `.exe` on Windows, the architecture on
    /// Linux (`.x86_64`, `.aarch64`) the way Unity and Godot name theirs.
    #[serde(default)]
    pub executable_name: String,

    /// Whether to build with optimisations.
    ///
    /// Off produces a debug build: faster to compile, several times
    /// larger, and far slower to run. On is what ships.
    #[serde(default = "default_true")]
    pub release: bool,

    /// Extra cargo features, comma separated.
    ///
    /// ⚠️ `editor` is never one of them: a shipped build must not contain
    /// the authoring surface, and the game binary's target cannot link it
    /// (#558).
    #[serde(default)]
    pub features: String,

    /// Whether assets are packed into an encrypted `.kpack` rather than
    /// copied as loose files.
    ///
    /// Off is useful while working out why a build behaves differently
    /// from the editor: the files are right there to look at. On is what
    /// ships.
    ///
    /// ⚠️ The key has to be inside the binary for the binary to read the
    /// pack, so this raises the cost of taking your assets — it does not
    /// make it impossible. See `kooch_pack`.
    #[serde(default = "default_true")]
    pub pack_assets: bool,

    /// Whether this is the preset the toolbar's one-click build uses.
    ///
    /// Exactly one should be. The panel picks the first when several say
    /// yes, and says so.
    #[serde(default)]
    pub runnable: bool,

    /// Whether the game is built with the profiler compiled in.
    ///
    /// On, the binary opens a socket on `0.0.0.0:8585` and streams every
    /// frame to the editor's Profiler panel. That is the only way to
    /// measure a game on the hardware it has to run on — a capture taken
    /// on the desktop describes the desktop (#769).
    ///
    /// 🔴 **Never on for a build anyone else receives** (#558). It is a
    /// listening socket and a thread, and off is not "switched off": with
    /// the feature absent every `profiling::scope!` in the engine expands
    /// to nothing at compile time.
    ///
    /// Its own preset rather than a checkbox on the release one, so
    /// "make a build" and "make a build I can measure" stay different
    /// actions and the fast path cannot acquire a socket by accident.
    #[serde(default)]
    pub profiling: bool,

    /// Oldest glibc the build has to run on, e.g. `2.28`.
    ///
    /// Empty links against this machine's, which is the bug it exists to
    /// fix: glibc is forward compatible and not backward, so a game built
    /// on an up-to-date desktop **refuses to start** on a Steam Deck or a
    /// handheld running an older one — with a message about a missing
    /// symbol version, which says nothing about what to do.
    ///
    /// Set, the build goes through `cargo zigbuild`, which links against
    /// that version's symbols instead. `2.28` covers everything from
    /// Debian 10 and RHEL 8 onward and is what Godot's own Linux exports
    /// target; `2.31` is Ubuntu 20.04.
    ///
    /// ⚠️ Needs `zig` and `cargo-zigbuild` — both install without root,
    /// and the build says so before compiling rather than after. Ignored
    /// for targets that are not `*-linux-gnu`.
    #[serde(default)]
    pub min_glibc: String,
}

fn default_output_dir() -> String {
    "build".to_owned()
}

fn default_true() -> bool {
    true
}

impl Default for BuildPreset {
    /// A release build of this machine's own platform, packed — the
    /// thing someone means by "make a build" before they have opinions.
    fn default() -> Self {
        Self {
            target_triple: String::new(),
            output_dir: default_output_dir(),
            executable_name: String::new(),
            release: true,
            features: String::new(),
            pack_assets: true,
            runnable: true,
            // Off, and it is the one field where the default is a
            // shipping decision rather than a convenience: a build made
            // without thinking about it must not listen on a port.
            profiling: false,
            min_glibc: String::new(),
        }
    }
}

impl BuildPreset {
    /// The cargo profile directory this preset's output lands in.
    pub fn profile_dir(&self) -> &'static str {
        match self.release {
            true => "release",
            false => "debug",
        }
    }

    /// The features to pass, split and trimmed.
    ///
    /// 🔴 `editor` is dropped rather than passed on. It is the one
    /// feature that would put the authoring surface back into a shipped
    /// game, and a preset is a text field somebody can type anything
    /// into (#558).
    ///
    /// `profiling` is the reverse: the checkbox decides, and a copy typed
    /// into the text field is dropped rather than passed twice. Cargo
    /// tolerates the duplicate; a preset that says the feature is off
    /// while the build turns it on does not.
    pub fn feature_list(&self) -> Vec<String> {
        let mut features: Vec<String> = self
            .features
            .split(',')
            .map(str::trim)
            .filter(|f| {
                !f.is_empty()
                    && *f != crate::cargo_args::AUTHORING
                    && *f != PROFILING_FEATURE
                    && *f != PROFILING_SHORTHAND
            })
            .map(str::to_owned)
            .collect();
        if self.profiling {
            features.push(PROFILING_FEATURE.to_owned());
        }
        features
    }

    /// The executable's file name for this preset's target, extension
    /// included.
    ///
    /// # Why Linux gets an extension at all
    ///
    /// It does not need one — an ELF is executable because of its mode
    /// and its header, not its name. `.x86_64` is the convention Unity
    /// and Godot use in their Linux exports, and it earns its place the
    /// moment a folder holds more than one platform: `game.exe` beside a
    /// bare `game` is ambiguous about which is which, and about whether
    /// the second one is a script.
    ///
    /// ⚠️ Never `.sh`. That is a text script the shell interprets; this
    /// is a compiled binary, and the extension would make some file
    /// managers offer to open it in an editor.
    pub fn binary_name(&self, crate_name: &str) -> String {
        let stem = match self.executable_name.trim() {
            "" => crate_name,
            name => name,
        };
        let triple = self.target_triple.trim();
        if triple.contains("windows") {
            return format!("{stem}.exe");
        }
        // Read off the triple rather than assumed: a build for
        // `aarch64-unknown-linux-gnu` is not an `x86_64` one, and a name
        // that says otherwise is worse than no name at all. An empty
        // triple means this machine, so its own architecture answers.
        let arch = match triple.split('-').next().unwrap_or_default() {
            "" => std::env::consts::ARCH,
            arch => arch,
        };
        format!("{stem}.{arch}")
    }

    /// Whether this preset builds for the machine running the editor.
    pub fn is_host(&self) -> bool {
        self.target_triple.trim().is_empty()
    }

    /// The glibc version this build must not go above, if one was asked
    /// for and the target is one it means anything for.
    ///
    /// An empty triple is this machine, and this machine runs Linux
    /// whenever the editor was compiled for it — so the floor applies
    /// there too, which is the common case: someone building for their
    /// own desktop and copying the result to a handheld.
    pub fn glibc_floor(&self) -> Option<&str> {
        let floor = self.min_glibc.trim();
        let triple = self.target_triple.trim();
        let gnu_linux =
            triple.contains("linux-gnu") || (triple.is_empty() && cfg!(target_os = "linux"));
        match !floor.is_empty() && gnu_linux {
            true => Some(floor),
            false => None,
        }
    }
}

/// Reads a `.buildpreset`.
#[derive(Debug, Default, Clone, Copy)]
pub struct BuildPresetLoader;

impl AssetLoader<BuildPreset> for BuildPresetLoader {
    fn extensions(&self) -> &[&'static str] {
        &[BUILD_PRESET_EXTENSION]
    }

    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<BuildPreset> {
        let text = std::str::from_utf8(bytes).map_err(|e| AssetError::Loader(Box::new(e)))?;
        // Every field has a serde default, so a preset written by an
        // older editor still loads and gains the new fields' defaults.
        ron::from_str(text).map_err(|e| AssetError::Loader(Box::new(e)))
    }
}

kooch_ecs::register_reflected_asset!(BuildPreset, BuildPresetLoader);

/// Serialises a preset for writing.
pub fn to_ron(preset: &BuildPreset) -> Result<String, ron::Error> {
    ron::ser::to_string_pretty(preset, ron::ser::PrettyConfig::default())
}

#[cfg(test)]
mod preset_tests;
