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

/// Gives a project without VS Code settings a `.vscode/settings.json` that reads `.shader` as WGSL.
/// wgsl-analyzer cannot see what the engine composes around a surface, so what hinges on it is off:
/// type errors, naga's unresolved names and `[error]` type hints. Its own syntax errors stay.
pub(crate) fn write_vscode_settings(root: &Path) {
    let vscode = root.join(".vscode");
    let settings = vscode.join("settings.json");
    // The author's own settings are theirs.
    if settings.exists() {
        return;
    }
    let text = "{\n  \"files.associations\": { \"*.shader\": \"wgsl\" },\n  \
                \"wgsl-analyzer.diagnostics.typeErrors\": false,\n  \
                \"wgsl-analyzer.diagnostics.nagaParsingErrors\": false,\n  \
                \"wgsl-analyzer.inlayHints.typeHints\": false\n}\n";
    match std::fs::create_dir_all(&vscode).and_then(|()| std::fs::write(&settings, text)) {
        Ok(()) => {
            tracing::info!(path = %settings.display(), "VS Code settings written for .shader files")
        }
        Err(error) => {
            tracing::warn!(path = %settings.display(), %error, "could not write VS Code settings")
        }
    }
}

#[cfg(test)]
mod tests;
