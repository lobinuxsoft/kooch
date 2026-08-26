//! `.buildpreset` — what "make a build" means for one target (#758).
//!
//! Modelled on Godot's export presets, read from
//! `editor/export/editor_export_preset.h` rather than from memory. What
//! was worth taking:
//!
//! - **Named presets, several per project**, saved with the project. Not
//!   one global configuration: a project has "Windows release", "Linux
//!   debug", "handheld", and they differ in more than one field.
//! - An output path per preset.
//!
//! What was not worth taking: Godot's `runnable` flag. The panel's list
//! is the selector, so a field marking one preset as "the" one decided
//! nothing except which row drew a different icon.
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

use std::path::{Path, PathBuf};

use kooch_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};
use kooch_ecs::Reflect;
use kooch_ecs::reflect::FieldChoice;
use serde::{Deserialize, Serialize};

use super::platform::Platform;

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

/// The build that ships: fully optimised, no profiler, no open socket.
pub const MODE_RELEASE: u32 = 0;

/// The build that gets measured: the same optimisations, plus the
/// profiler.
pub const MODE_PROFILING: u32 = 1;

/// Labels for the `mode` dropdown.
///
/// 🔴 **Both modes are optimised, and that is the point.** An earlier
/// design had "Debug" here, compiled without `--release`. Measuring that
/// binary describes that binary: the editor's own debug build ran at
/// 14.31 ms against 4.94 ms for its release build, and the handheld's
/// whole budget is 13.9 ms. A mode that answers "how slow is my game"
/// with a number three times too large is worse than no mode.
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
    ///
    /// Enabling both platforms builds both, one after the other, from
    /// one press — each into its own folder.
    #[serde(default)]
    #[reflect(group = "Platforms")]
    pub linux: bool,

    /// Build for Windows.
    ///
    /// ⚠️ Cross-compiling to Windows needs the `x86_64-pc-windows-gnu`
    /// target and mingw-w64. Both are checked before cargo starts, and
    /// the check says what to install.
    #[serde(default)]
    #[reflect(group = "Platforms")]
    pub windows: bool,

    /// Folder the builds are written to, relative to the project.
    ///
    /// Each platform gets a subfolder of its own — `build/linux`,
    /// `build/windows` — so building both does not have the second
    /// overwrite the first. Everything one game needs lands in its own
    /// folder together: the executable, `scenes/`, and the asset pack.
    #[serde(default = "default_output_dir")]
    pub output_dir: String,

    /// Name of the produced executable, without an extension.
    ///
    /// Empty uses the crate's own name. The extension is added for you
    /// and follows the platform: `.exe` on Windows, `.x86_64` on Linux
    /// the way Unity and Godot name theirs.
    #[serde(default)]
    pub executable_name: String,

    /// What this build is for.
    ///
    /// **Release** — the build you give people. Optimised as far as
    /// cargo goes: LTO across every crate and a single codegen unit.
    /// Carries no profiler and opens no port.
    ///
    /// **Profiling** — the same build, plus the profiler. It streams
    /// every frame to the editor's Profiler panel over
    /// `0.0.0.0:8585`, CPU and GPU alike. Use it to find out where a
    /// frame goes on the machine the game has to run on.
    ///
    /// Both are optimised on purpose. A build compiled without
    /// optimisations runs several times slower, so measuring one tells
    /// you about that build and not about your game.
    ///
    /// 🔴 **Never hand out a Profiling build** (#558): it listens on a
    /// socket. Release is not "the profiler switched off" — with the
    /// feature absent the instrumentation is not in the executable at
    /// all.
    #[serde(default)]
    #[reflect(choices = BUILD_MODE_CHOICES)]
    pub mode: u32,

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
    /// by every platform that has no glibc, so a preset building both
    /// Linux and Windows carries the floor into the Linux half only.
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
    ///
    /// Always `release`: both modes are optimised, and the profiler is a
    /// feature rather than a profile.
    pub fn profile_dir(&self) -> &'static str {
        "release"
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
        if self.is_profiling() {
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
    pub fn binary_name(&self, crate_name: &str, platform: Platform) -> String {
        let stem = match self.executable_name.trim() {
            "" => crate_name,
            name => name,
        };
        format!("{stem}{}", platform.extension())
    }

    /// The platforms this preset builds, in the order they are built.
    ///
    /// Empty is a preset that builds nothing, which is a state the panel
    /// reports rather than one the build silently treats as "the host" —
    /// guessing there would make an unticked box behave like a ticked
    /// one.
    pub fn targets(&self) -> Vec<Platform> {
        Platform::ALL
            .into_iter()
            .filter(|platform| match platform {
                Platform::Linux => self.linux,
                Platform::Windows => self.windows,
            })
            .collect()
    }

    /// Where `platform`'s build is written, relative to the project.
    pub fn platform_dir(&self, platform: Platform) -> PathBuf {
        Path::new(&self.output_dir).join(platform.folder())
    }

    /// The glibc version this build must not go above, if one was asked
    /// for and `platform` is one it means anything for.
    ///
    /// Per platform rather than per preset: one preset can build both,
    /// and `cargo zigbuild` rejects `x86_64-pc-windows-gnu.2.28` — so a
    /// floor that followed the build onto Windows would fail it on an
    /// argument nobody typed.
    pub fn glibc_floor(&self, platform: Platform) -> Option<&str> {
        let floor = self.min_glibc.trim();
        match !floor.is_empty() && platform.takes_glibc_floor() {
            true => Some(floor),
            false => None,
        }
    }

    /// Whether any platform this preset builds needs `cargo zigbuild`.
    ///
    /// The toolchain check runs once for the whole preset, before the
    /// first compile — so it asks about the set, not about one member.
    pub fn needs_zig(&self) -> bool {
        self.targets()
            .into_iter()
            .any(|platform| self.glibc_floor(platform).is_some())
    }
}

/// The `release` / `profiling` booleans `mode` replaced.
///
/// 🔴 **A missing field is not a missing decision.** `mode` defaults to
/// `Release`, so without this a preset written last week — the one that
/// says `profiling: true` — would load as Release and produce a binary
/// with no instrumentation in it. Nothing would fail: the build would
/// succeed, the panel would offer to connect, and the connection would
/// time out against a game that never opened the port.
///
/// Serde drops unknown fields rather than reporting them, which is what
/// makes reading them deliberately the only way to see them at all.
#[derive(Deserialize)]
struct LegacyMode {
    #[serde(default, deserialize_with = "present_bool")]
    release: Option<bool>,
    #[serde(default, deserialize_with = "present_bool")]
    profiling: Option<bool>,
}

/// The `target_triple` the platform toggles replaced.
///
/// 🔴 **A missing field is not a missing decision.** The toggles default
/// to `false`, so without this a preset written last week would open
/// with no platform ticked and build nothing — and the first save would
/// write that emptiness back over the only record of what it was for.
///
/// An absent field and an empty one mean different things and are kept
/// apart: absent is a file that already had toggles, empty is one that
/// said "this machine".
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
            // An empty triple was "this machine", which is the host —
            // not an unreadable triple. A triple that names neither
            // platform is one this editor cannot build for, and taking
            // the host instead would build something the file never
            // asked for.
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

/// Reads a plain `true` / `false` into `Some`, leaving `None` to mean
/// the field was absent.
///
/// ⚠️ `Option<bool>` alone does not do this in RON: it writes an option
/// as `Some(true)` and rejects a bare `true` with `ExpectedOption`. The
/// files being migrated were written by the old struct, where the field
/// was a plain `bool`.
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
            // `release: false` was a debug build, and there is no debug
            // mode any more. It wanted to be run and looked at, which is
            // what Profiling is for — and it says so in the log rather
            // than quietly building something else.
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
