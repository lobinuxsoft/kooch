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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Playing(pub bool);

impl Playing {
    /// Reads the flag out of `Resources`, defaulting to not playing.
    pub fn is_playing(resources: &Resources) -> bool {
        resources.get::<Playing>().is_some_and(|p| p.0)
    }

    /// Writes the flag, inserting the resource if it is missing. Stopping puts the time scale back
    /// to 1: `Time` outlives Play, and a game paused when stopped would start the next one frozen.
    pub fn set(resources: &mut Resources, playing: bool) {
        match resources.get_mut::<Playing>() {
            Some(state) => state.0 = playing,
            None => {
                resources.insert(Playing(playing));
            }
        }
        if !playing && let Some(time) = resources.get_mut::<crate::time::Time>() {
            time.set_scale(1.0);
        }
    }
}

/// Wraps a system so it only runs while [`Playing`] is set.
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

/// Whether this world is a **mirror of another process** rather than a simulation of its own.
///
/// 🔴 A mirrored world must not simulate. The editor shows a connected project by copying its
/// entities, and anything the editor derives locally competes with what the project derived — two
/// answers to one question, with the author watching the wrong one. The rule the owner set is that
/// Play shows the game exactly, with no exceptions for being the editor.
///
/// Set by the editor around a remote session; never set in a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mirrored(pub bool);

impl Mirrored {
    /// Reads the flag out of `Resources`, defaulting to a world of its own.
    pub fn is_mirrored(resources: &Resources) -> bool {
        resources.get::<Mirrored>().is_some_and(|m| m.0)
    }

    /// Writes the flag, inserting the resource if it is missing.
    pub fn set(resources: &mut Resources, mirrored: bool) {
        match resources.get_mut::<Mirrored>() {
            Some(state) => state.0 = mirrored,
            None => {
                resources.insert(Mirrored(mirrored));
            }
        }
    }
}
