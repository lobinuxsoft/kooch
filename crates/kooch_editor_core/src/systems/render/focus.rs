//! Bringing a panel into view when an asset is opened in it.

use crate::actions::EditorAction;
use egui_dock::DockState;

use crate::os_windows::OsWindows;
use crate::state::{EditorTab, OpenInputMap, OpenShaderGraph};

/// Shows the Image view, in a window of its own unless it is already showing (#1792).
pub(super) fn show_image_view(ui: &egui::Ui, dock: &mut DockState<EditorTab>, windows: &OsWindows) {
    if crate::os_windows::shown(dock, windows, EditorTab::ImageView) {
        return;
    }
    let surface = dock.add_window(vec![EditorTab::ImageView]);
    let screen = ui.max_rect();
    let size = egui::vec2(screen.width() * 0.5, screen.height() * 0.6);
    if let Some(window) = dock.get_window_state_mut(surface) {
        window
            .set_size(size)
            .set_position(screen.center() - size / 2.0);
    }
}

/// Opens the panel an asset was just opened in, unless it is already showing — in the dock or in a
/// window of its own (#1196).
pub(super) fn show_opened(
    ui: &egui::Ui,
    dock: &mut DockState<EditorTab>,
    windows: &OsWindows,
    open_shader_graph: Option<&OpenShaderGraph>,
    open_input_map: Option<&OpenInputMap>,
    open_ui_builder: bool,
    actions: &mut Vec<EditorAction>,
) {
    // Opening an asset has to show it. A panel that loaded the
    // map behind a tab nobody switched to is indistinguishable
    // from one that did nothing.
    if open_shader_graph
        .as_ref()
        .is_some_and(|open| open.focus_requested)
    {
        if !crate::os_windows::shown(dock, windows, EditorTab::ShaderGraph) {
            let surface = dock.add_window(vec![EditorTab::ShaderGraph]);
            // A graph needs room: at egui's default size a window shows three nodes and
            // no preview. A share of the screen, so it fits whatever screen it opens on.
            let screen = ui.max_rect();
            let size = egui::vec2(screen.width() * 0.7, screen.height() * 0.75);
            if let Some(window) = dock.get_window_state_mut(surface) {
                window
                    .set_size(size)
                    .set_position(screen.center() - size / 2.0);
            }
        }
        actions.push(EditorAction::ShaderGraphFocused);
    }
    if open_input_map.is_some_and(|open| open.focus_requested) {
        if !crate::os_windows::shown(dock, windows, EditorTab::InputMap) {
            dock.add_window(vec![EditorTab::InputMap]);
        }
        actions.push(EditorAction::InputMapFocused);
    }
    if open_ui_builder && !crate::os_windows::shown(dock, windows, EditorTab::UiBuilder) {
        // A canvas needs room, like a graph.
        let surface = dock.add_window(vec![EditorTab::UiBuilder]);
        let screen = ui.max_rect();
        let size = egui::vec2(screen.width() * 0.7, screen.height() * 0.75);
        if let Some(window) = dock.get_window_state_mut(surface) {
            window
                .set_size(size)
                .set_position(screen.center() - size / 2.0);
        }
    }
}
