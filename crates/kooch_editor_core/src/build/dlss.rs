//! What a build that asked for DLSS needs, before cargo and after (#536).
//!
//! Two obligations, and the engine meets both rather than reminding
//! anyone about them:
//!
//! - **Before cargo**, `DLSS_SDK` and `VULKAN_SDK` have to be in the
//!   environment or `dlss_wgpu`'s build script panics — ten minutes in,
//!   with a message about bindgen.
//! - **After cargo**, the runtime blob and NVIDIA's notices have to sit
//!   beside the executable. The blob because NGX `dlopen`s it from the
//!   application's own directory; the notices because section 9.5 of the
//!   Programming Guide requires it of anyone who ships the blob.
//!
//! 🔴 The second one is why this is code and not documentation. A
//! licence file nobody remembered to copy is a licence breach, and
//! "tell the author to remember" is the design that produces one.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::BuildPreset;

/// The cargo feature a project turns on to get DLSS.
///
/// 🔴 Namespaced, like `PROFILING_FEATURE`. `dlss` on its own is a
/// feature of the GAME's crate, which no project declares, and cargo
/// answers that with *"the package does not contain this feature"*.
/// `kooch/dlss` reaches the engine's without the project declaring
/// anything.
pub const FEATURE: &str = "kooch/dlss";

/// The spelling a project uses if it declared a passthrough of its own.
///
/// Accepted because a project is entitled to wrap the engine's feature
/// in one of its own — and because it is what anyone types first.
pub const BARE_FEATURE: &str = "dlss";

/// What the notices land as, beside the executable.
///
/// 🔴 The Programming Guide itself rather than an extract. Section 9.5
/// lives inside a PDF, and slicing text out of one needs a tool the
/// machine doing the build may not have — on Windows it almost
/// certainly has not. Copying the document is six megabytes and cannot
/// go stale against the SDK it came from, which an extract can.
pub const NOTICES_NAME: &str = "DLSS_NOTICES.pdf";

/// Whether this preset asked for DLSS.
pub fn wanted(preset: &BuildPreset) -> bool {
    preset
        .feature_list()
        .iter()
        .any(|feature| feature == FEATURE || feature == BARE_FEATURE)
}

/// Rewrites a bare `dlss` into the spelling cargo accepts.
///
/// 🔴 The papercut this removes: `--features dlss` names a feature of
/// the GAME's crate, and cargo refuses a build that asks for one the
/// project never declared. Every project would have to write the same
/// three-line passthrough, and the failure until it did was
/// *"the package does not contain this feature"* — which says nothing
/// about the engine.
///
/// ⚠️ Guarded on the project's own manifest rather than applied
/// blindly. A project is entitled to declare `dlss` as a feature that
/// means more than the engine's, and rewriting that would silently
/// build something else.
pub fn normalise(features: Vec<String>, project_root: &Path) -> Vec<String> {
    if !features.iter().any(|f| f == BARE_FEATURE) {
        return features;
    }
    let manifest = std::fs::read_to_string(project_root.join("Cargo.toml")).unwrap_or_default();
    if declares_feature(&manifest, BARE_FEATURE) {
        return features;
    }
    features
        .into_iter()
        .map(|f| {
            if f == BARE_FEATURE {
                FEATURE.to_owned()
            } else {
                f
            }
        })
        .collect()
}

/// Whether `manifest`'s `[features]` table has an entry called `name`.
///
/// Scanned rather than parsed: the question is one key in one table, and
/// a manifest parser is a dependency this crate does not otherwise need.
fn declares_feature(manifest: &str, name: &str) -> bool {
    let mut in_features = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_features = line == "[features]";
            continue;
        }
        if !in_features {
            continue;
        }
        if let Some((key, _)) = line.split_once('=')
            && key.trim().trim_matches('"') == name
        {
            return true;
        }
    }
    false
}

/// What is missing before cargo can be started, if anything.
///
/// The same job [`missing_toolchain`](super::compile) does for a target
/// triple, for the same reason: the failure without it arrives late and
/// names the wrong thing.
pub fn missing_sdk(preset: &BuildPreset) -> Option<String> {
    if !wanted(preset) {
        return None;
    }
    match crate::dlss_sdk::sdk_dir() {
        Some(dir) if crate::dlss_sdk::is_installed(&dir) => None,
        _ => Some(format!(
            "this preset asks for the `{FEATURE}` feature and NVIDIA's DLSS SDK is not \
             installed — install it from Settings, which downloads {} after you accept \
             NVIDIA's terms",
            crate::dlss_sdk::VERSION
        )),
    }
}

/// Puts the SDK where `dlss_wgpu`'s build script looks.
///
/// A value already in the environment wins, the way the optimisation
/// settings do: someone who exported `VULKAN_SDK` meant it.
pub fn build_env(command: &mut Command, preset: &BuildPreset) {
    if !wanted(preset) {
        return;
    }
    let Some(sdk) = crate::dlss_sdk::sdk_dir() else {
        return;
    };
    if std::env::var_os("DLSS_SDK").is_none() {
        command.env("DLSS_SDK", &sdk);
    }
    if std::env::var_os("VULKAN_SDK").is_none() {
        // 🔴 `/usr`, not a LunarG install. The build script only wants
        // `$VULKAN_SDK/include/vulkan/vulkan.h` for bindgen, and on
        // every distro that ships `vulkan-headers` that is where it is.
        command.env("VULKAN_SDK", vulkan_sdk());
    }
}

/// Where the Vulkan headers are, on a machine that installed them the
/// way its distribution ships them.
fn vulkan_sdk() -> PathBuf {
    PathBuf::from("/usr")
}

/// Copies the runtime and the notices beside the executable.
///
/// Returns what it wrote, so the build log can say it — a file that
/// appears without being mentioned is a file the author deletes.
pub fn ship(preset: &BuildPreset, dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    if !wanted(preset) {
        return Ok(Vec::new());
    }
    let Some(sdk) = crate::dlss_sdk::sdk_dir() else {
        return Ok(Vec::new());
    };
    let mut written = Vec::new();
    let runtime = crate::dlss_sdk::runtime_for(&sdk, &preset.target_triple);
    if runtime.is_file() {
        let dest = dir.join(file_name(&runtime));
        std::fs::copy(&runtime, &dest)?;
        written.push(dest);
    }
    let notices = crate::dlss_sdk::notices_path(&sdk);
    if notices.is_file() {
        let dest = dir.join(NOTICES_NAME);
        std::fs::copy(&notices, &dest)?;
        written.push(dest);
    }
    Ok(written)
}

/// The file's own name, which NGX looks for verbatim.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
