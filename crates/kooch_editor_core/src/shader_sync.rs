//! Hot reload for `.shader` files, which are written by an IDE rather than by the editor (#1157).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use kooch_core::asset_database::AssetDatabase;
use kooch_core::resource::Resources;
use kooch_render::material::SHADER_TYPE_NAME;

/// The same cadence as `script_sync`: a `stat` per shader is nothing at this rate.
const POLL: Duration = Duration::from_millis(750);

/// The modification time each registered shader had at the last poll.
#[derive(Default)]
pub struct ShaderSync {
    next_poll: Option<Instant>,
    seen: HashMap<PathBuf, SystemTime>,
}

impl ShaderSync {
    /// Records `modified` for `path`, and reports whether it moved since the last sighting.
    fn moved(&mut self, path: PathBuf, modified: SystemTime) -> bool {
        self.seen
            .insert(path, modified)
            .is_some_and(|before| before != modified)
    }
}

/// Reloads every shader whose file changed since the last poll, locally and in the project.
pub fn sync_shaders_system(resources: &mut Resources) {
    let now = Instant::now();
    let Some(mut sync) = resources.remove::<ShaderSync>() else {
        return;
    };
    if sync.next_poll.is_some_and(|next| now < next) {
        resources.insert(sync);
        return;
    }
    sync.next_poll = Some(now + POLL);

    let paths: Vec<PathBuf> = resources
        .get::<AssetDatabase>()
        .map(|db| {
            db.entries_of_type(SHADER_TYPE_NAME)
                .map(|(_, entry)| entry.path.clone())
                .collect()
        })
        .unwrap_or_default();
    // 🔴 A first sighting is recorded, never acted on: opening a project reloads nothing.
    let changed: Vec<PathBuf> = paths
        .into_iter()
        .filter(|path| {
            std::fs::metadata(path)
                .and_then(|m| m.modified())
                .is_ok_and(|modified| sync.moved(path.clone(), modified))
        })
        .collect();
    resources.insert(sync);

    for path in changed {
        crate::actions::handlers::asset_saved(resources, &path);
        tracing::info!(path = %path.display(), "shader reloaded");
    }
}

/// What `.shader` files need from wgsl-analyzer, at the top level of `.vscode/settings.json`. It
/// cannot see what the engine composes around a surface, so what hinges on it is off: type errors,
/// naga's unresolved names and `[error]` type hints. Its own syntax errors stay.
const ANALYZER_SETTINGS: [&str; 3] = [
    "wgsl-analyzer.diagnostics.typeErrors",
    "wgsl-analyzer.diagnostics.nagaParsingErrors",
    "wgsl-analyzer.inlayHints.typeHints",
];

/// Adds what `.shader` files need to the project's `.vscode/settings.json`, creating it if absent.
/// Keys the author already set keep their values, and the rest of the file keeps its formatting.
pub(crate) fn write_vscode_settings(root: &Path) {
    let settings = root.join(".vscode").join("settings.json");
    let current = std::fs::read_to_string(&settings).unwrap_or_else(|_| "{}".to_owned());
    let merged = match merged_settings(&current) {
        Ok(Some(merged)) => merged,
        Ok(None) => return,
        Err(reason) => {
            tracing::warn!(path = %settings.display(), "VS Code settings left untouched: {reason}");
            return;
        }
    };
    let written = std::fs::create_dir_all(root.join(".vscode"))
        .and_then(|()| std::fs::write(&settings, merged));
    match written {
        Ok(()) => {
            tracing::info!(path = %settings.display(), "VS Code settings updated for .shader files")
        }
        Err(error) => {
            tracing::warn!(path = %settings.display(), %error, "could not write VS Code settings")
        }
    }
}

/// `text` with whatever `.shader` needs added, or `None` when nothing is missing.
///
/// 🔴 Spliced into the text rather than re-serialised: serde_json would sort the author's keys. A file
/// with comments or trailing commas is not plain JSON and is refused rather than guessed at.
fn merged_settings(text: &str) -> Result<Option<String>, String> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| {
        format!("not plain JSON ({e}); comments and trailing commas are not merged")
    })?;
    let object = value.as_object().ok_or("its top level is not an object")?;
    let mut merged = text.trim_end().to_owned();
    let mut additions: Vec<String> = Vec::new();

    match object.get("files.associations") {
        None => additions.push(r#""files.associations": { "*.shader": "wgsl" }"#.to_owned()),
        Some(serde_json::Value::Object(map)) if map.contains_key("*.shader") => {}
        Some(serde_json::Value::Object(map)) => {
            let key = merged
                .find("\"files.associations\"")
                .ok_or("files.associations is not where it parsed")?;
            let brace = key
                + merged[key..]
                    .find('{')
                    .ok_or("files.associations has no body")?;
            let entry = match map.is_empty() {
                true => r#" "*.shader": "wgsl" "#,
                false => r#" "*.shader": "wgsl","#,
            };
            merged.insert_str(brace + 1, entry);
        }
        Some(_) => return Err("files.associations is not an object".to_owned()),
    }
    for key in ANALYZER_SETTINGS {
        if !object.contains_key(key) {
            additions.push(format!("\"{key}\": false"));
        }
    }

    if !additions.is_empty() {
        let close = merged.rfind('}').ok_or("no closing brace")?;
        let before = merged[..close].trim_end();
        let separator = if before.ends_with('{') { "" } else { "," };
        let body: Vec<String> = additions.iter().map(|a| format!("  {a}")).collect();
        merged = format!("{before}{separator}\n{}\n}}", body.join(",\n"));
    }
    if merged == text.trim_end() {
        return Ok(None);
    }
    merged.push('\n');
    serde_json::from_str::<serde_json::Value>(&merged)
        .map_err(|e| format!("the merge would not be valid JSON ({e})"))?;
    Ok(Some(merged))
}

#[cfg(test)]
mod tests;
