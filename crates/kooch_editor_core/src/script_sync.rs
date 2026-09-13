//! Keeping `registrations.rs` level with `src/`, without being told.

use std::path::Path;
use std::time::{Duration, Instant};

use kooch_core::resource::Resources;

use crate::actions::register_scripts;
use crate::project_state::ProjectState;

/// How often the fingerprint is taken. Fast enough that saving in
/// another window and alt-tabbing back finds it already done, slow
/// enough that a `stat` per file is nothing.
const POLL: Duration = Duration::from_millis(750);

/// Whether the generated registrations match `src/`, and whether the
/// running project matches them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum SyncState {
    /// Nothing to report: the file is current and no rewrite has
    /// happened since the project was last built.
    #[default]
    Current,
    /// `src/` moved, so the project's compiled code is now behind its source. Drawn as a pulse on
    /// **Rebuild & Run**, which is the button that fixes it, and cleared by the reload that a
    /// finished rebuild triggers — see [`crate::code_reload`].
    NeedsRebuild,
}

/// The poll's state between frames.
#[derive(Default)]
pub struct ScriptSync {
    /// When the fingerprint may be taken again.
    next_poll: Option<Instant>,
    /// Newest modification time in `src/`, and how many files it saw.
    fingerprint: Option<(Duration, usize)>,
    pub state: SyncState,
}

impl ScriptSync {
    /// The author has seen it. Also what a rebuild calls.
    pub fn acknowledge(&mut self) {
        self.state = SyncState::Current;
    }
}

/// Newest mtime and file count under `src/`, or `None` if it cannot be read.
fn fingerprint(src: &Path) -> Option<(Duration, usize)> {
    fn walk(dir: &Path, newest: &mut Duration, count: &mut usize) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, newest, count);
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            if path.file_name().is_some_and(|n| n == "registrations.rs") {
                continue;
            }
            *count += 1;
            let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
                continue;
            };
            if let Ok(since) = modified.duration_since(std::time::UNIX_EPOCH) {
                *newest = (*newest).max(since);
            }
        }
    }
    if !src.is_dir() {
        return None;
    }
    let mut newest = Duration::ZERO;
    let mut count = 0;
    walk(src, &mut newest, &mut count);
    Some((newest, count))
}

/// Regenerates `registrations.rs` when `src/` has moved under it.
pub fn sync_scripts_system(resources: &mut Resources) {
    let now = Instant::now();
    let Some(sync) = resources.get_mut::<ScriptSync>() else {
        return;
    };
    if sync.next_poll.is_some_and(|next| now < next) {
        return;
    }
    sync.next_poll = Some(now + POLL);

    let Some(src) = resources
        .get::<ProjectState>()
        .and_then(|ps| ps.active_project.as_ref())
        .map(|active| active.root_path.join("src"))
    else {
        return;
    };
    let Some(taken) = fingerprint(&src) else {
        return;
    };
    let Some(sync) = resources.get_mut::<ScriptSync>() else {
        return;
    };
    // 🔴 A first sighting is recorded, never acted on. Opening a project
    // would otherwise regenerate on the first frame and announce a
    // rebuild for a file that was already correct.
    let known = sync.fingerprint.replace(taken);
    if known.is_none_or(|last| last == taken) {
        return;
    }

    // 🔴 The outcome is deliberately not consulted. `registrations.rs` lists TYPES, so adding a
    // field to a component, fixing a system's body or changing a default rewrites nothing — and
    // every one of them leaves the dylib the editor reads from behind the source on screen.
    register_scripts(resources);
    if let Some(sync) = resources.get_mut::<ScriptSync>() {
        sync.state = SyncState::NeedsRebuild;
    }
}

#[cfg(test)]
mod tests;
