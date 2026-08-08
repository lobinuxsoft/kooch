//! [`Playing`] — the runtime gate that decides whether gameplay runs.
//!
//! A project's systems must not run while it is being edited, but the
//! same binary has to be able to start running them without a rebuild:
//! that is what "play in the editor" means. So the gate is a resource
//! read every frame, not a compile-time or build-time branch — the
//! editor flips it and gameplay starts on the next tick.
//!
//! The schedule has no run-condition mechanism; a system is any
//! `FnMut(&mut Resources)`. [`run_if_playing`] therefore gates by
//! wrapping, which composes with `add_system` unchanged:
//!
//! ```ignore
//! app.add_system(Stage::Update, run_if_playing(move_system));
//! ```

use crate::resource::Resources;

/// Whether gameplay systems should run this frame.
///
/// Absent from `Resources` is treated as **not playing** by
/// [`run_if_playing`]: a world with no opinion is a world being edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Playing(pub bool);

impl Playing {
    /// Reads the flag out of `Resources`, defaulting to not playing.
    pub fn is_playing(resources: &Resources) -> bool {
        resources.get::<Playing>().is_some_and(|p| p.0)
    }

    /// Writes the flag, inserting the resource if it is missing.
    pub fn set(resources: &mut Resources, playing: bool) {
        match resources.get_mut::<Playing>() {
            Some(state) => state.0 = playing,
            None => {
                resources.insert(Playing(playing));
            }
        }
    }
}

/// Wraps a system so it only runs while [`Playing`] is set.
///
/// The wrapped system is still registered with the schedule — it is
/// skipped per frame, not omitted from the build — so toggling
/// [`Playing`] starts and stops gameplay live.
pub fn run_if_playing<F>(mut system: F) -> impl FnMut(&mut Resources) + Send + Sync + 'static
where
    F: FnMut(&mut Resources) + Send + Sync + 'static,
{
    move |resources: &mut Resources| {
        if Playing::is_playing(resources) {
            system(resources);
        }
    }
}

#[cfg(test)]
mod tests;
