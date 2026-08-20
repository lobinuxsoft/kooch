//! Applying [`WindowMode`] to the live window, and reporting what the
//! platform can do.
//!
//! The value is authored in `.rendersettings` and published as a
//! resource by `kooch_render`; this is the half that touches winit.
//! Neither crate knows the other — the vocabulary is `kooch_core`'s.

use kooch_core::resource::Resources;
use kooch_core::window_mode::{DisplayModes, Resolution, WindowMode, best_mode, effective};
use winit::window::{Fullscreen, Window};

use crate::handle::WindowHandle;

/// Whether this platform can honour [`WindowMode::Exclusive`].
///
/// 🔴 Asked of the window rather than of `cfg!(target_os)`. The same
/// Linux binary runs under Wayland and under X11, and only one of them
/// changes display modes — a compile-time answer would be wrong on
/// whichever machine it was not built for.
///
/// ⚠️ Asked through `xdg_toplevel()`, which is the only Wayland-vs-X11
/// question winit's `Window` answers: its own documentation is *"or
/// `None` if the window is X11 window"*. There is an `is_wayland` in
/// this module of winit and it is on the event loop, which is not
/// reachable from a system.
fn exclusive_supported(window: &Window) -> bool {
    #[cfg(all(
        unix,
        not(any(target_os = "macos", target_os = "android", target_os = "ios"))
    ))]
    {
        use winit::platform::wayland::WindowExtWayland;
        return window.xdg_toplevel().is_none();
    }
    #[cfg(not(all(
        unix,
        not(any(target_os = "macos", target_os = "android", target_os = "ios"))
    )))]
    {
        let _ = window;
        true
    }
}

/// Every mode the window's current monitor reports, deduplicated and
/// sorted largest first.
///
/// Deduplicated because a monitor lists one entry per refresh rate and a
/// resolution dropdown wants one entry per size; the refresh a size can
/// reach is [`best_mode`]'s problem, not the list's.
fn monitor_modes(window: &Window) -> Vec<Resolution> {
    let Some(monitor) = window.current_monitor() else {
        return Vec::new();
    };
    let mut modes: Vec<Resolution> = monitor
        .video_modes()
        .map(|mode| {
            let size = mode.size();
            Resolution {
                width: size.width,
                height: size.height,
                refresh_mhz: mode.refresh_rate_millihertz(),
            }
        })
        .collect();
    modes.sort_unstable_by_key(|mode| {
        std::cmp::Reverse((mode.width, mode.height, mode.refresh_mhz))
    });
    modes
}

/// Publishes [`DisplayModes`] once the window exists.
///
/// 🔴 A game's options menu is built from this and not from a constant:
/// the list belongs to the **player's** monitor, and `exclusive` is
/// false under Wayland, where a resolution dropdown would change
/// nothing.
///
/// Refreshed only while the resource is absent — the modes of a monitor
/// do not change while a game runs, and enumerating them is a round trip
/// to the compositor.
pub(crate) fn publish_display_modes_system(resources: &mut Resources) {
    if resources.get::<DisplayModes>().is_some() {
        return;
    }
    let Some(handle) = resources.get::<WindowHandle>() else {
        return;
    };
    let window = handle.window();
    let modes = DisplayModes {
        modes: monitor_modes(window),
        exclusive: exclusive_supported(window),
    };
    tracing::info!(
        count = modes.modes.len(),
        exclusive = modes.exclusive,
        "display modes enumerated",
    );
    resources.insert(modes);
}

/// Puts [`WindowMode`] and [`Resolution`] on the window, if they are not
/// already there.
///
/// 🔴 Absent means "no opinion", the same rule the quality settings
/// follow: a game with no settings asset, and a test that made its own
/// window, keep the window they have. This system creates no default.
///
/// 🔴 Guarded on what the window currently is rather than applied every
/// frame. `set_fullscreen` on Wayland is a round trip to the compositor
/// that ends in a configure event and a surface resize; issuing it sixty
/// times a second would keep the swapchain being rebuilt for a value
/// that never changed. The exclusive request is degraded by
/// [`effective`] before the comparison for the same reason — an
/// un-degraded one never matches and would be retried forever.
///
/// Runs in `Stage::Last`, after `apply_render_settings_system` has
/// published the resources in `Update`, so a change lands on the frame
/// it is made rather than the one after.
pub(crate) fn apply_window_mode_system(resources: &mut Resources) {
    let Some(asked) = resources.get::<WindowMode>().copied() else {
        return;
    };
    let wanted_size = resources.get::<Resolution>().copied();
    let supported = resources
        .get::<DisplayModes>()
        .is_none_or(|modes| modes.exclusive);
    let available: Vec<Resolution> = resources
        .get::<DisplayModes>()
        .map(|modes| modes.modes.clone())
        .unwrap_or_default();
    let Some(handle) = resources.get::<WindowHandle>() else {
        return;
    };
    let window = handle.window();
    let wanted = effective(asked, supported);
    if wanted != asked {
        tracing::warn!(
            ?asked,
            ?wanted,
            "this platform does not change display modes; using borderless fullscreen",
        );
    }

    if window.fullscreen().is_some() != wanted.fullscreen() || wanted == WindowMode::Exclusive {
        let fullscreen = match wanted {
            WindowMode::Exclusive => exclusive_target(window, &available, wanted_size),
            WindowMode::Fullscreen => Some(Fullscreen::Borderless(None)),
            WindowMode::Windowed | WindowMode::Borderless => None,
        };
        // An exclusive request that found no matching mode comes back as
        // borderless rather than as nothing: the player asked to fill
        // the screen and the size is the part that could not be given.
        if window.fullscreen().is_some() != fullscreen.is_some() || fullscreen.is_some() {
            window.set_fullscreen(fullscreen);
            tracing::info!(?wanted, "window mode changed");
        }
    }
    if window.is_decorated() != wanted.decorated() {
        window.set_decorations(wanted.decorated());
    }

    // Only where a size means the window's own. Fullscreen of either
    // kind is sized by the monitor, and asking a fullscreen window to
    // resize is a request the compositor is right to ignore.
    if !wanted.fullscreen()
        && let Some(size) = wanted_size
    {
        let current = window.inner_size();
        if (current.width, current.height) != (size.width, size.height) {
            let _ =
                window.request_inner_size(winit::dpi::PhysicalSize::new(size.width, size.height));
            tracing::info!(size.width, size.height, "window size requested");
        }
    }
}

/// The `Fullscreen` an exclusive request turns into.
///
/// Falls back to borderless when the monitor has nothing of the asked
/// size — [`best_mode`] refuses to substitute a nearby resolution, and
/// the honest outcome is a full screen at the wrong size rather than a
/// window.
fn exclusive_target(
    window: &Window,
    available: &[Resolution],
    wanted: Option<Resolution>,
) -> Option<Fullscreen> {
    let (Some(wanted), Some(monitor)) = (wanted, window.current_monitor()) else {
        return Some(Fullscreen::Borderless(None));
    };
    let Some(chosen) = best_mode(available, wanted) else {
        tracing::warn!(
            wanted.width,
            wanted.height,
            "the monitor has no mode of that size; using borderless fullscreen",
        );
        return Some(Fullscreen::Borderless(None));
    };
    monitor
        .video_modes()
        .find(|mode| {
            let size = mode.size();
            (size.width, size.height, mode.refresh_rate_millihertz())
                == (chosen.width, chosen.height, chosen.refresh_mhz)
        })
        .map(Fullscreen::Exclusive)
        .or(Some(Fullscreen::Borderless(None)))
}

#[cfg(test)]
mod tests;
