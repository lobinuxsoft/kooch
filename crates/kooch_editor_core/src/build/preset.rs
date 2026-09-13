//! `.buildpreset` — what "make a build" means for one target (#758).

use kooch_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};
use kooch_ecs::Reflect;
use kooch_ecs::reflect::FieldChoice;
use serde::{Deserialize, Serialize};

use super::platform::Platform;

/// Extension a build preset carries.
pub const BUILD_PRESET_EXTENSION: &str = "buildpreset";

/// The feature that compiles the profiler into a game.
const PROFILING_FEATURE: &str = "kooch/profiling";

/// What someone types into the features field meaning the same thing.
const PROFILING_SHORTHAND: &str = "profiling";

/// The build that ships: fully optimised, no profiler, no open socket.
pub const MODE_RELEASE: u32 = 0;

/// The build that gets measured: the same optimisations, plus the
/// profiler.
pub const MODE_PROFILING: u32 = 1;

/// Labels for the `mode` dropdown.
pub static BUILD_MODE_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Release",
        value: MODE_RELEASE as i64,
    },
    FieldChoice {
        label: "Profiling",
        value: MODE_PROFILING as i64,
    },
];

/// One way of building this project.
#[derive(Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(category = "Build")]
pub struct BuildPreset {
    /// Build for Linux.
    #[serde(default)]
    #[reflect(group = "Platforms")]
    pub linux: bool,

    /// Build for Windows.
    #[serde(default)]
    #[reflect(group = "Platforms")]
    pub windows: bool,

    /// Folder the builds are written to, relative to the project.
    #[serde(default = "default_output_dir")]
    pub output_dir: String,

    /// Name of the produced executable, without an extension.
    #[serde(default)]
    pub executable_name: String,

    /// What this build is for.
    #[serde(default)]
    #[reflect(choices = BUILD_MODE_CHOICES)]
    pub mode: u32,

    /// Extra cargo features, comma separated.
    #[serde(default)]
    pub features: String,

    /// Whether assets are packed into an encrypted `.kpack` rather than copied as loose files.
    #[serde(default = "default_true")]
    pub pack_assets: bool,

    /// Oldest glibc the build has to run on, e.g. `2.28`.
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
            // The machine in front of the author, which is what "make a
            // build" means before anyone has opinions about platforms.
            linux: matches!(Platform::host(), Some(Platform::Linux)),
            windows: matches!(Platform::host(), Some(Platform::Windows)),
            output_dir: default_output_dir(),
            executable_name: String::new(),
            // The one field whose default is a shipping decision rather
            // than a convenience: a build made without thinking about it
            // must not listen on a port.
            mode: MODE_RELEASE,
            features: String::new(),
            pack_assets: true,
            min_glibc: String::new(),
        }
    }
}

impl BuildPreset {
    /// Whether this preset compiles the profiler in.
    pub fn is_profiling(&self) -> bool {
        self.mode == MODE_PROFILING
    }

    /// The label this preset's mode carries in the UI.
    pub fn mode_label(&self) -> &'static str {
        BUILD_MODE_CHOICES
            .iter()
            .find(|choice| choice.value == self.mode as i64)
            .map(|choice| choice.label)
            .unwrap_or("Release")
    }

    /// The cargo profile directory this preset's output lands in.
    pub fn profile_dir(&self) -> &'static str {
        "release"
    }

    /// The features to pass, split and trimmed.
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
        if self.is_profiling() {
            features.push(PROFILING_FEATURE.to_owned());
        }
        features
    }

    /// The executable's file name for this preset's target, extension included.
    pub fn binary_name(&self, crate_name: &str, platform: Platform) -> String {
        let stem = match self.executable_name.trim() {
            "" => crate_name,
            name => name,
        };
        format!("{stem}{}", platform.extension())
    }

    /// The platforms this preset builds, in the order they are built.
    pub fn targets(&self) -> Vec<Platform> {
        Platform::ALL
            .into_iter()
            .filter(|platform| match platform {
                Platform::Linux => self.linux,
                Platform::Windows => self.windows,
            })
            .collect()
    }

    /// The glibc version this build must not go above, if one was asked for and `platform` is one
    /// it means anything for.
    pub fn glibc_floor(&self, platform: Platform) -> Option<&str> {
        let floor = self.min_glibc.trim();
        match !floor.is_empty() && platform.takes_glibc_floor() {
            true => Some(floor),
            false => None,
        }
    }

    /// Whether any platform this preset builds needs `cargo zigbuild`.
    pub fn needs_zig(&self) -> bool {
        self.targets()
            .into_iter()
            .any(|platform| self.glibc_floor(platform).is_some())
    }
}

/// The `release` / `profiling` booleans `mode` replaced.
#[derive(Deserialize)]
struct LegacyMode {
    #[serde(default, deserialize_with = "present_bool")]
    release: Option<bool>,
    #[serde(default, deserialize_with = "present_bool")]
    profiling: Option<bool>,
}

/// The `target_triple` the platform toggles replaced.
#[derive(Deserialize)]
struct LegacyTarget {
    #[serde(default, deserialize_with = "present_string")]
    target_triple: Option<String>,
}

impl LegacyTarget {
    /// The platform a pre-toggle preset meant, or `None` when the file
    /// was written by an editor that already had the toggles.
    fn platform(&self) -> Option<Platform> {
        let triple = self.target_triple.as_deref()?;
        match Platform::from_triple(triple) {
            Some(platform) => Some(platform),
            // An empty triple was "this machine", which is the host — not an unreadable triple. A
            // triple that names neither platform is one this editor cannot build for, and taking
            // the host instead would build something the file never asked for.
            None if triple.trim().is_empty() => Platform::host(),
            None => {
                tracing::warn!(
                    triple,
                    "build preset: this target has no platform toggle, so the preset \
                     opens with none ticked — pick one before building",
                );
                None
            }
        }
    }
}

/// Reads a plain string into `Some`, leaving `None` to mean the field
/// was absent — the same distinction [`present_bool`] draws.
fn present_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    String::deserialize(deserializer).map(Some)
}

/// Reads a plain `true` / `false` into `Some`, leaving `None` to mean the field was absent.
fn present_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    bool::deserialize(deserializer).map(Some)
}

impl LegacyMode {
    /// The mode a pre-`mode` preset meant, or `None` when the file was
    /// written by an editor that already had the dropdown.
    fn mode(&self) -> Option<u32> {
        match (self.release, self.profiling) {
            (None, None) => None,
            // Whatever it asked for, it asked to be measured.
            (_, Some(true)) => Some(MODE_PROFILING),
            // `release: false` was a debug build, and there is no debug mode any more. It wanted to
            // be run and looked at, which is what Profiling is for — and it says so in the log
            // rather than quietly building something else.
            (Some(false), _) => {
                tracing::info!(
                    "build preset: `release: false` has no equivalent — both modes are \
                     optimised now. Read as Profiling; set it to Release if this preset \
                     was what you handed out."
                );
                Some(MODE_PROFILING)
            }
            _ => Some(MODE_RELEASE),
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
        let mut preset: BuildPreset =
            ron::from_str(text).map_err(|e| AssetError::Loader(Box::new(e)))?;
        // Read a second time for the fields the struct no longer has.
        // A file carrying them predates the dropdown, and its booleans
        // are the only record of what it was for.
        if let Ok(legacy) = ron::from_str::<LegacyMode>(text)
            && let Some(mode) = legacy.mode()
        {
            preset.mode = mode;
        }
        if let Ok(legacy) = ron::from_str::<LegacyTarget>(text)
            && let Some(platform) = legacy.platform()
        {
            match platform {
                Platform::Linux => preset.linux = true,
                Platform::Windows => preset.windows = true,
            }
        }
        Ok(preset)
    }
}

kooch_ecs::register_reflected_asset!(BuildPreset, BuildPresetLoader);

/// Serialises a preset for writing.
pub fn to_ron(preset: &BuildPreset) -> Result<String, ron::Error> {
    ron::ser::to_string_pretty(preset, ron::ser::PrettyConfig::default())
}

#[cfg(test)]
mod preset_tests;
