use std::path::{Path, PathBuf};

use kooch_core::app::App;
use kooch_core::asset_loader::AssetServer;
use kooch_core::resource::Resources;

use crate::material::Material;
use crate::meshlet::MeshletMesh;
use crate::texture::Image;

pub(super) fn eager_import_typed_assets(app: &mut App, root: &Path) {
    let resources = app.resources_mut();
    eager_import_with(resources, root);
}

/// Walks `root` recursively and loads every file with a recognised
/// typed extension through the `AssetServer`. The load step generates
/// `.meta` sidecars on the fly for assets that do not yet have one,
/// back-fills `asset_type` on legacy sidecars, and registers the
/// entry in the `AssetDatabase` — exactly what the inspector picker
/// needs to surface a new asset at first frame.
///
/// Public so the project-side scan system can rerun the same import
/// pass after a project opens.
pub fn eager_import_with(resources: &mut Resources, root: &Path) {
    let scanned = collect_typed_files(root);
    if scanned.is_empty() {
        return;
    }

    let Some(mut server) = resources.remove::<AssetServer>() else {
        return;
    };

    let mut counts = ImportCounts::default();
    for path in &scanned {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let ext_lower = ext.to_ascii_lowercase();
        match ext_lower.as_str() {
            "glb" | "gltf" => {
                if let Err(e) = server.load::<MeshletMesh>(path, resources) {
                    tracing::warn!(
                        target: "kooch_render::plugin::assets",
                        path = %path.display(),
                        error = %e,
                        "eager MeshletMesh import failed",
                    );
                } else {
                    counts.meshlet += 1;
                }
            }
            "png" | "jpg" | "jpeg" => {
                if let Err(e) = server.load::<Image>(path, resources) {
                    tracing::warn!(
                        target: "kooch_render::plugin::assets",
                        path = %path.display(),
                        error = %e,
                        "eager Image import failed",
                    );
                } else {
                    counts.image += 1;
                }
            }
            "ron" => {
                // PR5 invariant: every `.ron` under `assets/` is a
                // Material. When other RON-authored asset types
                // arrive, this branch grows a discriminator that
                // peeks the nominal struct tag at the head of the
                // file before dispatching to the matching loader.
                if let Err(e) = server.load::<Material>(path, resources) {
                    tracing::warn!(
                        target: "kooch_render::plugin::assets",
                        path = %path.display(),
                        error = %e,
                        "eager Material import failed",
                    );
                } else {
                    counts.material += 1;
                }
            }
            _ => {}
        }
    }
    resources.insert(server);

    if counts.any() {
        tracing::info!(
            target: "kooch_render::plugin::assets",
            meshlet = counts.meshlet,
            image = counts.image,
            material = counts.material,
            "eager-imported typed assets",
        );
    }
}

/// Recursive filesystem walk that collects every file under `root`
/// with a known typed extension. Stays alongside the importer
/// because both share the extension allowlist.
fn collect_typed_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_collect(root, &mut out);
    out
}

fn walk_collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            walk_collect(&path, out);
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        // Skip the sidecar files themselves — only consider their
        // source assets.
        if path.extension().is_some_and(|e| e == "meta") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        let typed = lower.ends_with(".glb")
            || lower.ends_with(".gltf")
            || lower.ends_with(".png")
            || lower.ends_with(".jpg")
            || lower.ends_with(".jpeg")
            || lower.ends_with(".ron");
        if typed {
            out.push(path);
        }
    }
}

#[derive(Default)]
struct ImportCounts {
    meshlet: usize,
    image: usize,
    material: usize,
}

impl ImportCounts {
    fn any(&self) -> bool {
        self.meshlet + self.image + self.material > 0
    }
}
