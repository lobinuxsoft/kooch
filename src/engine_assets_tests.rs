//! The engine's shipped assets checked as data: GUIDs must agree across files, and a material
//! pointing at a missing texture renders flat without error.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The repository's `assets/` directory, from this crate's manifest.
fn assets_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn every_file() -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(&assets_root(), &mut out);
    out
}

/// Reads the `guid = "..."` line out of a `.meta`.
fn guid_of(meta: &Path) -> Option<String> {
    let text = std::fs::read_to_string(meta).ok()?;
    let table: kooch_core::toml::Table = text.parse().ok()?;
    Some(table.get("guid")?.as_str()?.to_owned())
}

/// 🔴 Duplicate GUIDs make one asset unreachable, chosen by scan order — the failure a batch
/// generator produces.
#[test]
fn no_two_assets_share_a_guid() {
    let mut seen: HashMap<String, PathBuf> = HashMap::new();
    for path in every_file() {
        if path.extension().is_none_or(|e| e != "meta") {
            continue;
        }
        let Some(guid) = guid_of(&path) else {
            panic!("{} has no readable guid", path.display());
        };
        if let Some(first) = seen.insert(guid.clone(), path.clone()) {
            panic!(
                "{guid} is claimed by both {} and {}",
                first.display(),
                path.display(),
            );
        }
    }
}

/// Every asset has its `.meta`, or the scanner never registers it and scenes cannot reference it.
#[test]
fn every_asset_has_a_meta() {
    for path in every_file() {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(".meta") || name == "README.md" {
            continue;
        }
        let meta = PathBuf::from(format!("{}.meta", path.display()));
        assert!(
            meta.exists(),
            "{} ships without a .meta, so nothing can reference it",
            path.display(),
        );
    }
}

/// 🔴 Every texture a shipped material names ships — or it renders with the 1×1 white fallback,
/// traced to the wrong place.
#[test]
fn every_material_texture_exists() {
    let mut texture_guids: Vec<String> = Vec::new();
    for path in every_file() {
        if path.extension().is_none_or(|e| e != "meta") {
            continue;
        }
        let stem = path.with_extension("");
        if stem.extension().is_some_and(|e| e == "png" || e == "jpg") {
            texture_guids.extend(guid_of(&path));
        }
    }
    assert!(
        !texture_guids.is_empty(),
        "no textures found at all — the walk is looking in the wrong place",
    );

    for path in every_file() {
        if path.extension().is_none_or(|e| e != "ron") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for slot in ["albedo", "normal", "metal_roughness"] {
            let Some(rest) = text.split(&format!("{slot}: Some(\"")).nth(1) else {
                continue;
            };
            let guid = rest.split('"').next().unwrap_or_default().to_owned();
            assert!(
                texture_guids.contains(&guid),
                "{} points its {slot} at {guid}, which no shipped texture carries",
                path.display(),
            );
        }
    }
}
