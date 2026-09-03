//! Holding the world across a restart of the project.
//!
//! # What is lost without it
//!
//! In remote mode the world belongs to the project's process. Rebuild &
//! Run kills that process and starts a new one, which opens its main
//! scene from disk — so everything the author had done since the last
//! save was gone, and the scene they were looking at was replaced by
//! whichever one the project starts with.
//!
//! That is a rebuild costing work, which makes the button expensive
//! enough to avoid — and it is the button that picks up a code change.
//!
//! # Why through a file
//!
//! The wire has exactly one way to put a scene into the project:
//! `LoadScene { path }`. There is no method that carries a document, so
//! the live world goes to a holding file and comes back by the same road
//! every other scene takes. Nothing new had to be added to the protocol.
//!
//! # 🔴 What comes back is still unsaved
//!
//! The restored scene is re-pointed at the file it came from and left
//! **dirty**, because it is: it holds edits that file does not. Marking
//! it clean would be the editor claiming the author's work was written
//! out when a rebuild is exactly when it was not.

use std::path::{Path, PathBuf};

use kooch_core::Guid;
use kooch_core::resource::Resources;

use crate::actions::EditorAction;

/// One scene, held on disk while the project restarts.
struct Held {
    /// Identity, which survives the round trip: the document written out
    /// is the one read back.
    id: Guid,
    /// The file it should be written back to. `None` for a scene that
    /// was never saved anywhere.
    origin: Option<PathBuf>,
    /// Where its live state is parked.
    held: PathBuf,
}

/// How far along putting the world back has got.
#[derive(PartialEq, Eq)]
enum Phase {
    /// Written out, waiting for the project to answer again.
    Waiting,
    /// The loads have been queued; the paths still have to be put back.
    Sent,
}

/// The world, between one project process and the next.
pub struct CarriedWorld {
    scenes: Vec<Held>,
    phase: Phase,
}

/// Where held scenes live. Cleared on every capture, so at most one
/// generation is ever on disk.
fn holding() -> PathBuf {
    std::env::temp_dir().join("kooch_carried_world")
}

/// Writes every open scene out and remembers where each belongs.
///
/// Call before tearing the session down. Returns how many were held —
/// zero is the ordinary answer with nothing open, and not a failure.
pub fn capture(resources: &mut Resources) -> usize {
    let Some(manager) = resources.get::<kooch_ecs::SceneManager>() else {
        return 0;
    };
    let open: Vec<(Guid, Option<PathBuf>)> = manager
        .scenes()
        .iter()
        .map(|scene| (scene.id, scene.path.clone()))
        .collect();
    if open.is_empty() {
        return 0;
    }

    let dir = holding();
    let _ = std::fs::remove_dir_all(&dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::error!(dir = %dir.display(), error = %e, "cannot hold the world; the rebuild will start from the project's own scene");
        return 0;
    }

    let mut scenes = Vec::new();
    for (id, origin) in open {
        let held = dir.join(format!("{id}.{}", crate::project::SCENE_EXTENSION));
        // 🔴 Adopts the holding path as the scene's own. Harmless here
        // and only here: the session is about to be torn down and every
        // scene reloaded, and `restore` puts the origin back.
        match crate::actions::scene_io::save_open_scene_as(resources, id, held.clone()) {
            Ok(()) => scenes.push(Held { id, origin, held }),
            Err(e) => {
                tracing::error!(%id, error = %e, "a scene could not be held across the rebuild")
            }
        }
    }

    let count = scenes.len();
    if count > 0 {
        resources.insert(CarriedWorld {
            scenes,
            phase: Phase::Waiting,
        });
    }
    count
}

/// What to do about a held world this frame.
///
/// Drained by `apply_actions`, which is where queued work becomes
/// dispatched work. Returns nothing until the project answers again —
/// loading into a process that is still compiling would go nowhere.
pub(crate) fn resume(resources: &mut Resources) -> Vec<EditorAction> {
    let Some(carried) = resources.get::<CarriedWorld>() else {
        return Vec::new();
    };
    match carried.phase {
        Phase::Sent => {
            finish(resources);
            Vec::new()
        }
        Phase::Waiting => {
            let connected = resources
                .get::<crate::remote_session::RemoteState>()
                .is_some_and(|state| state.is_connected());
            if !connected {
                return Vec::new();
            }
            let actions = loads(&carried.scenes);
            if let Some(carried) = resources.get_mut::<CarriedWorld>() {
                carried.phase = Phase::Sent;
            }
            actions
        }
    }
}

/// The first scene replaces the world, the rest join it — which is what
/// having several open meant in the first place.
fn loads(scenes: &[Held]) -> Vec<EditorAction> {
    scenes
        .iter()
        .enumerate()
        .map(|(i, scene)| match i {
            0 => EditorAction::OpenScene {
                path: Some(scene.held.clone()),
            },
            _ => EditorAction::OpenSceneAdditive {
                path: Some(scene.held.clone()),
            },
        })
        .collect()
}

/// Puts each scene's real path back and leaves it dirty, then forgets
/// the holding files.
fn finish(resources: &mut Resources) {
    let Some(carried) = resources.remove::<CarriedWorld>() else {
        return;
    };
    let mut restored = 0usize;
    if let Some(manager) = resources.get_mut::<kooch_ecs::SceneManager>() {
        for scene in &carried.scenes {
            if manager.adopt_path(scene.id, scene.origin.clone()) {
                // It holds edits its file does not. Saying otherwise
                // would claim the author's work was written out by the
                // one action that did not write it.
                manager.mark_scene_dirty(scene.id);
                restored += 1;
            }
        }
    }
    let _ = std::fs::remove_dir_all(holding());
    tracing::info!(
        scenes = restored,
        "the world came back across the rebuild, still unsaved",
    );
}

/// Whether a path is one of the holding files, for anything that should
/// not treat it as a project asset.
pub fn is_held(path: &Path) -> bool {
    path.starts_with(holding())
}

#[cfg(test)]
mod tests;
