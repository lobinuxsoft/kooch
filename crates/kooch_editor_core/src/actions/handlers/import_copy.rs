//! Copying dropped files into the project. A `.gltf` names its buffers and textures by relative
//! URI, so it brings them along at the same relative paths, or it loads nothing (#1654).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::actions::asset_ops::unique_target;
use crate::build::package::gltf_files::uris_of;

/// Copies `files` into `dest` and returns where each landed.
pub(super) fn copy_dropped(files: &[PathBuf], dest: &Path) -> Vec<PathBuf> {
    let mut copied = Vec::new();
    let mut carried: HashSet<PathBuf> = HashSet::new();

    for gltf in files.iter().filter(|file| is_gltf(file)) {
        let Some(name) = gltf.file_name() else {
            continue;
        };
        let from = gltf.parent().unwrap_or(Path::new(""));
        let mut set = vec![(gltf.clone(), PathBuf::from(name))];
        set.extend(
            uris_of(gltf)
                .into_iter()
                .map(|uri| (from.join(&uri), PathBuf::from(uri)))
                .filter(|(source, _)| source.is_file()),
        );
        // Renaming one file of the set would leave the `.gltf` naming another file, so a clash
        // moves the whole set into a folder of its own instead.
        let clash = set.iter().any(|(_, relative)| dest.join(relative).exists());
        let root = match clash {
            true => unique_target(dest, gltf.file_stem().unwrap_or(name)),
            false => dest.to_path_buf(),
        };
        for (source, relative) in set {
            carried.insert(source.clone());
            copy_one(&source, &root.join(relative), &mut copied);
        }
    }

    for file in files {
        if is_gltf(file) || carried.contains(file) {
            continue;
        }
        let Some(name) = file.file_name() else {
            continue;
        };
        copy_one(file, &unique_target(dest, name), &mut copied);
    }
    copied
}

fn copy_one(source: &Path, target: &Path, copied: &mut Vec<PathBuf>) {
    if let Some(parent) = target.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::error!(dest = %parent.display(), error = %e, "import: cannot create destination");
        return;
    }
    match std::fs::copy(source, target) {
        Ok(_) => {
            tracing::info!(from = %source.display(), to = %target.display(), "asset imported");
            copied.push(target.to_path_buf());
        }
        Err(e) => {
            tracing::error!(from = %source.display(), error = %e, "asset import failed");
        }
    }
}

fn is_gltf(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("gltf"))
}
