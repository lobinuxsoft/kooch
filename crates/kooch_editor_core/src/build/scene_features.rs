//! A scene only works in a game whose build carries the crates its components come from (#1187).
//!
//! 🔴 A scene naming a component the binary lacks still loads — the row is parked, by design — so a
//! game built without `kooch/blockmesh` started with every block of its level missing, and the only
//! trace was a WARN in the handheld's log. This reads the scenes, works out which optional engine
//! crates the `game` build enables, and adds the feature that is missing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The engine's own manifest, for its `[features]` and which dependencies are optional. Compiled in:
/// the editor and the engine it vendors are the same version.
const ENGINE_MANIFEST: &str = include_str!("../../../../Cargo.toml");

/// A component a scene uses whose crate the build leaves out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    pub scene: PathBuf,
    pub component: String,
    /// The engine feature that brings the crate in, `blockmesh`.
    pub feature: String,
}

/// Every component a scene set uses that a build with `features` (and the project's defaults)
/// cannot load, one entry per engine feature.
pub fn missing(project_root: &Path, features: &[String]) -> Vec<Missing> {
    let manifest = std::fs::read_to_string(project_root.join("Cargo.toml")).unwrap_or_default();
    let scenes = scenes_in(&project_root.join("assets"));
    let texts: Vec<(PathBuf, String)> = scenes
        .into_iter()
        .filter_map(|path| std::fs::read_to_string(&path).ok().map(|text| (path, text)))
        .collect();
    missing_in(&manifest, ENGINE_MANIFEST, features, &texts)
}

/// [`missing`] over text, so a test needs no files.
pub(crate) fn missing_in(
    project_manifest: &str,
    engine_manifest: &str,
    features: &[String],
    scenes: &[(PathBuf, String)],
) -> Vec<Missing> {
    let Ok(engine) = engine_manifest.parse::<toml::Table>() else {
        return Vec::new();
    };
    let enabled = enabled_crates(project_manifest, &engine, features);
    let optional = optional_crates(&engine);
    let mut found: Vec<Missing> = Vec::new();
    for (scene, text) in scenes {
        for component in components(text) {
            let krate = component.split("::").next().unwrap_or_default();
            if !optional.contains(krate) || enabled.contains(krate) {
                continue;
            }
            let Some(feature) = feature_for(&engine, krate) else {
                continue;
            };
            if !found.iter().any(|m| m.feature == feature) {
                found.push(Missing {
                    scene: scene.clone(),
                    component: component.to_owned(),
                    feature,
                });
            }
        }
    }
    found
}

/// Adds `kooch/<feature>` for each of `missing` to the manifest's `game` feature. `None` when there is
/// no single-line `game = [...]` to add to, which a build then reports instead.
pub(crate) fn with_features(manifest: &str, missing: &[Missing]) -> Option<String> {
    let line = manifest
        .lines()
        .find(|line| line.trim_start().starts_with("game = [") && line.trim_end().ends_with(']'))?;
    let close = line.rfind(']')?;
    let mut entries = line[..close].trim_end().to_owned();
    for m in missing {
        let entry = format!("\"kooch/{}\"", m.feature);
        if entries.contains(&entry) {
            continue;
        }
        if !entries.ends_with('[') {
            entries.push_str(", ");
        }
        entries.push_str(&entry);
    }
    Some(manifest.replacen(line, &format!("{entries}]"), 1))
}

/// Adds what the default (`game`) build is missing to the project's manifest, and says why.
/// Returns what it could not fix.
pub fn ensure(project_root: &Path, features: &[String]) -> Vec<Missing> {
    let missing = missing(project_root, features);
    if missing.is_empty() {
        return missing;
    }
    let path = project_root.join("Cargo.toml");
    let Some(updated) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|manifest| with_features(&manifest, &missing))
    else {
        return missing;
    };
    if let Err(error) = std::fs::write(&path, updated) {
        tracing::error!(file = %path.display(), %error, "could not add the features the scenes need");
        return missing;
    }
    for m in &missing {
        tracing::info!(
            scene = %m.scene.display(),
            component = %m.component,
            "Cargo.toml: `game` gains `kooch/{}`, or a built game would load this scene without it",
            m.feature,
        );
    }
    Vec::new()
}

/// The engine crates a build turns on: the project's `default` plus `features`, followed through
/// both manifests' `[features]`.
fn enabled_crates(
    project_manifest: &str,
    engine: &toml::Table,
    features: &[String],
) -> BTreeSet<String> {
    let project = project_manifest.parse::<toml::Table>().unwrap_or_default();
    let project_features = feature_table(&project);
    let mut engine_features: Vec<String> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue: Vec<String> = features.to_vec();
    queue.push("default".to_owned());
    while let Some(name) = queue.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some(engine_feature) = name.strip_prefix("kooch/") {
            engine_features.push(engine_feature.to_owned());
        } else if let Some(entries) = project_features.get(&name) {
            queue.extend(entries.iter().cloned());
        }
    }
    let kooch_defaults = project
        .get("dependencies")
        .and_then(|deps| deps.get("kooch"))
        .and_then(|kooch| kooch.get("default-features"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(true);
    if kooch_defaults {
        engine_features.push("default".to_owned());
    }

    let table = feature_table(engine);
    let mut crates = BTreeSet::new();
    let mut seen = BTreeSet::new();
    while let Some(name) = engine_features.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        for entry in table.get(&name).into_iter().flatten() {
            if let Some(krate) = entry.strip_prefix("dep:") {
                crates.insert(krate.to_owned());
            } else if !entry.contains('/') {
                engine_features.push(entry.clone());
            }
        }
    }
    crates
}

fn feature_table(manifest: &toml::Table) -> std::collections::HashMap<String, Vec<String>> {
    manifest
        .get("features")
        .and_then(toml::Value::as_table)
        .map(|features| {
            features
                .iter()
                .map(|(name, entries)| {
                    let entries = entries
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|e| e.as_str().map(str::to_owned))
                        .collect();
                    (name.clone(), entries)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The engine's dependencies a feature has to turn on.
fn optional_crates(engine: &toml::Table) -> BTreeSet<String> {
    engine
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .map(|deps| {
            deps.iter()
                .filter(|(_, spec)| {
                    spec.get("optional").and_then(toml::Value::as_bool) == Some(true)
                })
                .map(|(name, _)| name.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// The engine feature that brings `krate` in by itself.
fn feature_for(engine: &toml::Table, krate: &str) -> Option<String> {
    let wanted = format!("dep:{krate}");
    let mut candidates: Vec<(String, usize)> = feature_table(engine)
        .into_iter()
        .filter(|(_, entries)| entries.contains(&wanted))
        .map(|(name, entries)| (name, entries.len()))
        .collect();
    // The narrowest: `blockmesh`, not a feature that happens to pull it in among others.
    candidates.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    candidates.into_iter().next().map(|(name, _)| name)
}

/// The engine component types a scene names, `kooch_blockmesh::block::Block`.
fn components(scene: &str) -> impl Iterator<Item = &str> {
    scene.split("type_name: \"").skip(1).filter_map(|rest| {
        let name = &rest[..rest.find('"')?];
        name.starts_with("kooch_").then_some(name)
    })
}

fn scenes_in(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(scenes_in(&path));
        } else if path.extension().is_some_and(|ext| ext == "scene") {
            out.push(path);
        }
    }
    out
}

#[cfg(test)]
mod tests;
