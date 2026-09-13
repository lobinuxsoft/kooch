//! Bringing a rebuilt project into the editor without reopening it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kooch_core::resource::Resources;

use crate::project_state::ProjectState;

/// How often the library is stat'd. Slower than the source poll: a
/// build takes seconds, so checking twice a second buys nothing.
const POLL: Duration = Duration::from_millis(1000);

/// The library poll's state between frames.
#[derive(Default)]
pub struct CodeReload {
    /// When the library may be stat'd again.
    next_poll: Option<Instant>,
    /// Modification time and size of the library last seen.
    stamp: Option<(Duration, u64)>,
}

/// Modification time and size of `library`, or `None` if it cannot be
/// read.
fn stamp(library: &Path) -> Option<(Duration, u64)> {
    let meta = std::fs::metadata(library).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    Some((modified, meta.len()))
}

/// Where the open project's library is, and what its crate is called.
fn library_of(resources: &Resources) -> Option<(PathBuf, PathBuf, String)> {
    let state = resources.get::<ProjectState>()?;
    let project = state.active_project.as_ref()?;
    let root = project.root_path.clone();
    let crate_name = project.manifest.name.clone();
    let library = crate::project_plugin::library_path(&root, &crate_name)?;
    Some((library, root, crate_name))
}

/// Swaps the project's library when the one on disk has moved.
///
/// Registered in `PreUpdate`, beside the source poll.
pub fn reload_code_system(resources: &mut Resources) {
    let now = Instant::now();
    let Some(reload) = resources.get_mut::<CodeReload>() else {
        return;
    };
    if reload.next_poll.is_some_and(|next| now < next) {
        return;
    }
    reload.next_poll = Some(now + POLL);

    let Some((library, root, crate_name)) = library_of(resources) else {
        return;
    };
    let Some(taken) = stamp(&library) else {
        return;
    };
    let Some(reload) = resources.get_mut::<CodeReload>() else {
        return;
    };
    // 🔴 A first sighting is recorded and acted on, never. The library that was loaded when the
    // project opened is the one on disk, and swapping it for itself on the first frame would be
    // work with a report attached.
    let known = reload.stamp.replace(taken);
    if known.is_none_or(|last| last == taken) {
        return;
    }

    // A build writes the library incrementally, so a poll can land on a half-written one. That
    // fails to open, the types from before are kept, and the next poll a second later finds the
    // finished file — self-correcting, and quieter than trying to detect it.
    crate::project_plugin::reload_project_plugins(resources, &root, &crate_name);
    tracing::info!(library = %library.display(), "reloaded the project's code");

    // 🔴 A successful swap is exactly what makes the build no longer behind, so the notice clears
    // itself. Before this, clearing it was the author's job — and a warning you dismiss by hand is
    // one you learn to dismiss without reading.
    if let Some(sync) = resources.get_mut::<crate::script_sync::ScriptSync>() {
        sync.acknowledge();
    }
}

#[cfg(test)]
mod tests;
