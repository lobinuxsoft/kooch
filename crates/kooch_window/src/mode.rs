//! Applies [`WindowMode`] to the live window and reports what the platform can do; `kooch_render`
//! publishes the value, and neither crate knows the other.

use kooch_core::resource::Resources;
use kooch_core::window_mode::{DisplayModes, Resolution, WindowMode, best_mode, effective};
use winit::window::{Fullscreen, Window};

use crate::handle::WindowHandle;

/// Whether this platform can honour [`WindowMode::Exclusive`], asked of the window: one Linux
/// binary runs under Wayland and X11, and only X11 changes display modes.
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

/// Every mode the current monitor reports, one per size and largest first; the refresh for a size
/// is [`best_mode`]'s job.
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

/// Publishes [`DisplayModes`] once the window exists — the player's monitor, not a constant, and
/// not exclusive under Wayland. Only while absent: a monitor's modes do not change mid-game.
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

/// Puts [`WindowMode`] and [`Resolution`] on the window when they differ from what it has. 🔴 Absent
/// means no opinion; comparing first avoids a compositor round trip every frame.
/// Runs in `Stage::Last`, after the settings are published, so a change lands the same frame.
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

/// The `Fullscreen` an exclusive request turns into: borderless when the monitor lacks the exact
/// size, since [`best_mode`] never substitutes one.
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
