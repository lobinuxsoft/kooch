//! A `.gltf` names its buffers and images by relative URI, not by GUID, so neither the loader
//! allowlist nor the GUID walk reaches them. They travel because the document that names them
//! does (#981).

use std::path::PathBuf;

/// Adds every file a kept `.gltf` names beside it and that is not already in `files`.
pub(super) fn carry_uris(files: &mut Vec<(String, PathBuf)>) {
    let mut carried: Vec<(String, PathBuf)> = Vec::new();
    for (name, path) in files.iter() {
        if !name.to_ascii_lowercase().ends_with(".gltf") {
            continue;
        }
        let folder = name.rsplit_once('/').map_or("", |(folder, _)| folder);
        let dir = path.parent().unwrap_or(path);
        for uri in uris_of(path) {
            let travelling = format!("{folder}/{uri}");
            let source = dir.join(&uri);
            let present = |list: &[(String, PathBuf)]| list.iter().any(|(n, _)| *n == travelling);
            if present(files) || present(&carried) {
                continue;
            }
            if source.is_file() {
                carried.push((travelling, source));
            } else {
                tracing::warn!(
                    target: "kooch_editor_core::build",
                    gltf = %path.display(),
                    %uri,
                    "a .gltf names a file that is not beside it; the built game will not load it",
                );
            }
        }
    }
    files.extend(carried);
}

/// The relative file URIs in a `.gltf`'s `buffers` and `images`. Inline `data:` URIs and anything
/// the loader's hygiene gate would refuse are left out.
pub(crate) fn uris_of(path: &std::path::Path) -> Vec<String> {
    let Ok(text) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok(document) = serde_json::from_slice::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    ["buffers", "images"]
        .iter()
        .filter_map(|key| document.get(key)?.as_array())
        .flatten()
        .filter_map(|entry| entry.get("uri")?.as_str())
        .map(|uri| uri.replace('\\', "/"))
        .filter(|uri| is_relative_file(uri))
        .collect()
}

fn is_relative_file(uri: &str) -> bool {
    !uri.is_empty()
        && !uri.starts_with('/')
        && !uri.contains(':')
        && !uri.split('/').any(|part| part == "..")
}
