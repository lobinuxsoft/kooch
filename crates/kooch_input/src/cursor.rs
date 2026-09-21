//! [`CursorMode`] — what the game asks of the cursor (#1266).
//!
//! Mouse look needs the cursor out of the way: held where it is so a turn never reaches the edge of
//! the window, and hidden. It is a request rather than a call, so a system anywhere can make it and
//! the window owner applies it once, when it changes.

use kooch_core::resource::Resources;
use kooch_window::WindowHandle;
use winit::window::CursorGrabMode;

/// How the cursor should behave. Absent means no opinion: the window keeps whatever it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorMode {
    /// Visible, and free to leave the window.
    #[default]
    Free,
    /// Held in place and hidden. Motion still arrives — raw, from the device — so a camera can turn
    /// forever.
    Captured,
}

/// What was last put on the window, so a request is applied once and a platform that drops the
/// grab when focus goes (X11 does) gets it back when focus returns.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AppliedCursor {
    mode: Option<CursorMode>,
    focused: bool,
}

/// Puts [`CursorMode`] on the window when it differs from what the window has.
pub(crate) fn apply_cursor_system(resources: &mut Resources) {
    let Some(wanted) = resources.get::<CursorMode>().copied() else {
        return;
    };
    let Some(window) = resources
        .get::<WindowHandle>()
        .map(|handle| handle.window().clone())
    else {
        return;
    };
    let focused = window.has_focus();
    let applied = resources
        .get::<AppliedCursor>()
        .copied()
        .unwrap_or_default();
    if applied.mode == Some(wanted) && applied.focused == focused {
        return;
    }
    match wanted {
        CursorMode::Captured => {
            // Locked where it is; confined to the window where the platform has no lock (X11).
            // Either way raw motion keeps arriving, which is all mouse look reads.
            if let Err(locked) = window.set_cursor_grab(CursorGrabMode::Locked)
                && let Err(confined) = window.set_cursor_grab(CursorGrabMode::Confined)
            {
                tracing::warn!(%locked, %confined, "the cursor could not be captured");
            }
            window.set_cursor_visible(false);
        }
        CursorMode::Free => {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
        }
    }
    resources.insert(AppliedCursor {
        mode: Some(wanted),
        focused,
    });
}
