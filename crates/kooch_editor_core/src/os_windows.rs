//! Panels torn off the dock into OS windows of their own, so one can live on another monitor
//! (#1196).
//!
//! 🔴 One egui context for every window: a torn-off panel is an immediate viewport of the main one,
//! drawn inside the same pass, so it keeps the panel state egui holds for it. The windows, their
//! surfaces and their input are ours to keep — egui only asks for them.

mod home;
mod nested;
mod paint;

use std::sync::{Arc, Mutex};

use egui_dock::DockState;
use winit::window::Window;

use crate::state::EditorTab;

pub(crate) use nested::{install, show};
pub(crate) use paint::{apply_textures, paint_all};

/// A panel that lives in its own window. What the layout file keeps of it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Detached {
    pub tab: EditorTab,
    /// Inner size, in logical pixels.
    pub size: [f32; 2],
    /// Outer position in physical pixels. `None` where the platform does not say — Wayland never
    /// does, and places the window itself.
    pub pos: Option<[i32; 2]>,
    /// Where it was in the dock, to go back there when its window closes.
    #[serde(default)]
    pub home: Option<home::Home>,
}

/// A torn-off panel's window, as long as it is open.
pub(crate) struct Live {
    pub tab: EditorTab,
    pub window: Arc<Window>,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    pub state: egui_winit::State,
    pub info: egui::ViewportInfo,
    /// Set by the window's close button; the dock takes the panel back on the next frame.
    pub closing: bool,
    /// What the nested pass drew, for the present to put on screen.
    pub output: Option<egui::FullOutput>,
}

/// Shared with the event handler and the nested pass, which both run outside the render system.
pub(crate) type SharedLive = Arc<Mutex<Vec<Live>>>;

/// Every torn-off panel, open or waiting for its window.
#[derive(Default)]
pub(crate) struct OsWindows {
    pub detached: Vec<Detached>,
    pub live: SharedLive,
}

/// The size a panel's window opens at when nothing was saved for it.
const DEFAULT_SIZE: [f32; 2] = [480.0, 640.0];

/// The egui viewport a tab is drawn in once torn off.
pub(crate) fn viewport_of(tab: EditorTab) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(("kooch_panel_window", tab))
}

/// The key a tab's window is requested under.
fn key_of(tab: EditorTab) -> u64 {
    tab as u64
}

/// The OS title. Named after the panel alone, so a window rule (KWin, for one) can pin it.
pub(crate) fn title_of(tab: EditorTab) -> String {
    let label = tab.label();
    let name = label
        .split_once(' ')
        .map_or(label.as_str(), |(_, name)| name);
    format!("Kóoch — {name}")
}

/// Takes `tab` out of the dock and into a window of its own.
pub(crate) fn detach(dock: &mut DockState<EditorTab>, windows: &mut OsWindows, tab: EditorTab) {
    let home = home::home_of(dock, tab);
    if let Some(path) = dock.find_tab(&tab) {
        dock.remove_tab(path);
    }
    if !windows.detached.iter().any(|d| d.tab == tab) {
        windows.detached.push(Detached {
            tab,
            size: DEFAULT_SIZE,
            pos: None,
            home,
        });
    }
}

/// Puts `tab` back in the dock, and forgets its window.
pub(crate) fn dock_back(dock: &mut DockState<EditorTab>, windows: &mut OsWindows, tab: EditorTab) {
    let home = windows
        .detached
        .iter()
        .find(|d| d.tab == tab)
        .and_then(|d| d.home.clone());
    windows.detached.retain(|d| d.tab != tab);
    if crate::state::dock_has_tab(dock, &tab) {
        return;
    }
    if !home.is_some_and(|home| home::go_home(dock, tab, &home)) {
        dock.push_to_first_leaf(tab);
    }
}

/// Whether `tab` is on screen, docked or in a window.
pub(crate) fn shown(dock: &DockState<EditorTab>, windows: &OsWindows, tab: EditorTab) -> bool {
    crate::state::dock_has_tab(dock, &tab) || windows.detached.iter().any(|d| d.tab == tab)
}

/// Closes `tab`'s window without docking it back.
pub(crate) fn close(windows: &mut OsWindows, tab: EditorTab) {
    windows.detached.retain(|d| d.tab != tab);
    for open in windows.live.lock().unwrap().iter_mut() {
        if open.tab == tab {
            open.closing = true;
        }
    }
}

/// A tab both docked and detached — a layout file edited by hand, or an old one — stays detached.
pub(crate) fn settle(dock: &mut DockState<EditorTab>, windows: &OsWindows) {
    for detached in &windows.detached {
        if let Some(path) = dock.find_tab(&detached.tab) {
            dock.remove_tab(path);
        }
    }
}

/// Once a frame, before the UI: asks for the windows detached panels still lack, adopts the ones
/// that arrived, docks back the ones closed, and records where the open ones are.
pub(crate) fn sync(
    dock: &mut DockState<EditorTab>,
    windows: &mut OsWindows,
    extra: &mut kooch_window::ExtraWindows,
    ctx: &egui::Context,
    gpu: &kooch_core::gpu::GpuContext,
) {
    let closed: Vec<EditorTab> = {
        let mut live = windows.live.lock().unwrap();
        let closed = live.iter().filter(|l| l.closing).map(|l| l.tab).collect();
        // Dropping the `Live` drops the window, which closes it.
        live.retain(|l| !l.closing);
        closed
    };
    // Only a window closed by its own button docks back; one closed from the Window menu is gone.
    for tab in closed {
        if windows.detached.iter().any(|d| d.tab == tab) {
            dock_back(dock, windows, tab);
        }
    }

    for (key, window) in extra.take_created() {
        let Some(detached) = windows.detached.iter().find(|d| key_of(d.tab) == key) else {
            continue;
        };
        match adopt(detached.tab, window, ctx, gpu) {
            Ok(live) => windows.live.lock().unwrap().push(live),
            Err(err) => {
                tracing::error!("panel window for {:?} has no surface: {err}", detached.tab)
            }
        }
    }

    let mut live = windows.live.lock().unwrap();
    for detached in &mut windows.detached {
        if let Some(open) = live.iter_mut().find(|l| l.tab == detached.tab) {
            let size = open
                .window
                .inner_size()
                .to_logical::<f32>(open.window.scale_factor());
            if size.width > 0.0 && size.height > 0.0 {
                detached.size = [size.width, size.height];
            }
            detached.pos = open.window.outer_position().ok().map(|p| [p.x, p.y]);
        } else if !extra.is_pending(key_of(detached.tab)) {
            extra.request(key_of(detached.tab), attributes(detached));
        }
    }
}

fn attributes(detached: &Detached) -> winit::window::WindowAttributes {
    let attrs = winit::window::WindowAttributes::default()
        .with_title(title_of(detached.tab))
        .with_window_icon(kooch_window::icon::window_icon())
        .with_inner_size(winit::dpi::LogicalSize::new(
            detached.size[0],
            detached.size[1],
        ));
    match detached.pos {
        Some([x, y]) => attrs.with_position(winit::dpi::PhysicalPosition::new(x, y)),
        None => attrs,
    }
}

/// Gives a new window its surface and its own egui input.
fn adopt(
    tab: EditorTab,
    window: Arc<Window>,
    ctx: &egui::Context,
    gpu: &kooch_core::gpu::GpuContext,
) -> Result<Live, wgpu::CreateSurfaceError> {
    let surface = gpu.instance().create_surface(Arc::clone(&window))?;
    let size = window.inner_size();
    let caps = surface.get_capabilities(gpu.adapter());
    // 🔴 The main surface's format: the egui renderer's pipeline is built for one format, and a
    // second window draws through the same renderer.
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: gpu.format(),
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode: wgpu::PresentMode::AutoVsync,
        desired_maximum_frame_latency: 2,
        alpha_mode: caps
            .alpha_modes
            .first()
            .copied()
            .unwrap_or(wgpu::CompositeAlphaMode::Auto),
        view_formats: vec![],
    };
    surface.configure(gpu.device(), &config);
    let state = egui_winit::State::new(
        ctx.clone(),
        viewport_of(tab),
        window.as_ref(),
        Some(window.scale_factor() as f32),
        None,
        // Overwritten by the main pass's on every nested pass; see `nested::run_nested`.
        None,
    );
    let mut info = egui::ViewportInfo::default();
    egui_winit::update_viewport_info(&mut info, ctx, &window, true);
    Ok(Live {
        tab,
        window,
        surface,
        config,
        state,
        info,
        closing: false,
        output: None,
    })
}

#[cfg(test)]
mod tests;
