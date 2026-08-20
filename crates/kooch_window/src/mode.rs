//! Applying [`WindowMode`] to the live window.
//!
//! The value is authored in `.rendersettings` and published as a
//! resource by `kooch_render`; this is the half that touches winit.
//! Neither crate knows the other — the vocabulary is `kooch_core`'s.

use kooch_core::resource::Resources;
use kooch_core::window_mode::WindowMode;
use winit::window::Fullscreen;

use crate::handle::WindowHandle;

/// Puts [`WindowMode`] on the window, if it is not already there.
///
/// 🔴 Absent means "no opinion", the same rule the quality settings
/// follow: a game with no settings asset, and a test that made its own
/// window, keep the window they have. This system creates no default.
///
/// 🔴 Guarded on what the window currently is rather than applied every
/// frame. `set_fullscreen` on Wayland is a round trip to the compositor
/// that ends in a configure event and a surface resize; issuing it sixty
/// times a second would keep the swapchain being rebuilt for a value
/// that never changed.
///
/// Runs in `Stage::Last`, after `apply_render_settings_system` has
/// published the resource in `Update`, so a change lands on the frame it
/// is made rather than the one after.
pub(crate) fn apply_window_mode_system(resources: &mut Resources) {
    let Some(wanted) = resources.get::<WindowMode>().copied() else {
        return;
    };
    let Some(handle) = resources.get::<WindowHandle>() else {
        return;
    };
    let window = handle.window();

    if window.fullscreen().is_some() != wanted.fullscreen() {
        window.set_fullscreen(wanted.fullscreen().then(|| Fullscreen::Borderless(None)));
        tracing::info!(?wanted, "window mode changed");
    }
    if window.is_decorated() != wanted.decorated() {
        window.set_decorations(wanted.decorated());
    }
}

#[cfg(test)]
mod tests;
