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
    /// Empty uses the crate's own name. Windows gets `.exe` added
    /// whatever this says.
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
    pub fn feature_list(&self) -> Vec<String> {
        self.features
            .split(',')
            .map(str::trim)
            .filter(|f| !f.is_empty() && *f != crate::cargo_args::AUTHORING)
            .map(str::to_owned)
            .collect()
    }

    /// The executable's file name for `target`, extension included.
    pub fn binary_name(&self, crate_name: &str) -> String {
        let stem = match self.executable_name.trim() {
            "" => crate_name,
            name => name,
        };
        match self.target_triple.contains("windows") {
            true => format!("{stem}.exe"),
            false => stem.to_owned(),
        }
    }

    /// Whether this preset builds for the machine running the editor.
    pub fn is_host(&self) -> bool {
        self.target_triple.trim().is_empty()
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
