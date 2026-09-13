//! What a build that asked for DLSS needs, before cargo and after (#536).

use std::path::{Path, PathBuf};
use std::process::Command;

use super::BuildPreset;

/// The cargo feature a project turns on to get DLSS.
pub const FEATURE: &str = "kooch/dlss";

/// The spelling a project uses if it declared a passthrough of its own.
pub const BARE_FEATURE: &str = "dlss";

/// What the notices land as, beside the executable.
pub const NOTICES_NAME: &str = "DLSS_NOTICES.pdf";

/// Whether this preset asked for DLSS.
pub fn wanted(preset: &BuildPreset) -> bool {
    preset
        .feature_list()
        .iter()
        .any(|feature| feature == FEATURE || feature == BARE_FEATURE)
}

/// Rewrites a bare `dlss` into the spelling cargo accepts.
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
pub fn missing_sdk(preset: &BuildPreset) -> Option<String> {
    if !wanted(preset) {
        return None;
    }
    match crate::dlss_sdk::sdk_dir() {
        Some(dir) if crate::dlss_sdk::is_installed(&dir) => {}
        _ => {
            return Some(format!(
                "this preset asks for the `{FEATURE}` feature and NVIDIA's DLSS SDK is not \
                 installed — install it from Settings, which downloads {} after you accept \
                 NVIDIA's terms",
                crate::dlss_sdk::VERSION
            ));
        }
    }
    missing_vulkan_headers()
}

/// The Vulkan headers, checked the way the build will look for them.
fn missing_vulkan_headers() -> Option<String> {
    let root = std::env::var_os("VULKAN_SDK")
        .map(PathBuf::from)
        .unwrap_or_else(vulkan_sdk);
    if header_in(&root).is_file() {
        return None;
    }
    let mut message = format!(
        "this preset asks for the `{FEATURE}` feature, which compiles NVIDIA's headers, \
         and there is no {} on this machine",
        header_in(&root).display()
    );
    match crate::preflight::Installer::detect().command(&[crate::preflight::VULKAN_HEADERS]) {
        Some(command) => message.push_str(&format!("\n\n{command}")),
        None => message.push_str(&format!("\n\n{}", crate::preflight::VULKAN_HEADERS.hint)),
    }
    Some(message)
}

/// Where the header sits under a Vulkan root, on every platform the
/// build script knows: `include` on unix, `Include` on Windows.
fn header_in(root: &Path) -> PathBuf {
    let include = if cfg!(windows) { "Include" } else { "include" };
    root.join(include).join("vulkan").join("vulkan.h")
}

/// Puts the SDK where `dlss_wgpu`'s build script looks.
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
    if std::env::var_os(BINDGEN_ARGS).is_none()
        && let Some(include) = clang_include()
    {
        command.env(BINDGEN_ARGS, format!("-I{}", include.display()));
    }
}

/// Where bindgen is told to find clang's own headers.
const BINDGEN_ARGS: &str = "BINDGEN_EXTRA_CLANG_ARGS";

/// Clang's resource headers, when they are somewhere bindgen will not look on its own.
fn clang_include() -> Option<PathBuf> {
    let mut best: Option<(u32, PathBuf)> = None;
    for root in ["/usr/lib/clang", "/usr/lib64/clang"] {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let include = entry.path().join("include");
            if !include.join("stdbool.h").is_file() {
                continue;
            }
            let version = entry
                .file_name()
                .to_string_lossy()
                .split('.')
                .next()
                .and_then(|major| major.parse::<u32>().ok())
                .unwrap_or(0);
            if best.as_ref().is_none_or(|(seen, _)| version > *seen) {
                best = Some((version, include));
            }
        }
    }
    best.map(|(_, include)| include)
}

/// Where the Vulkan headers are, on a machine that installed them the
/// way its distribution ships them.
fn vulkan_sdk() -> PathBuf {
    PathBuf::from("/usr")
}

/// Copies the runtime and the notices beside the executable.
pub fn ship(
    preset: &BuildPreset,
    platform: super::platform::Platform,
    dir: &Path,
) -> std::io::Result<Vec<PathBuf>> {
    if !wanted(preset) {
        return Ok(Vec::new());
    }
    let Some(sdk) = crate::dlss_sdk::sdk_dir() else {
        return Ok(Vec::new());
    };
    let mut written = Vec::new();
    let runtime = crate::dlss_sdk::runtime_for(&sdk, platform);
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
