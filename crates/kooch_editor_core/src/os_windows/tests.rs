use super::*;
use crate::state::default_dock_state;

/// Opening a panel in a window takes it out of the dock.
#[test]
fn a_detached_tab_leaves_the_dock() {
    let mut dock = default_dock_state();
    let mut windows = OsWindows::default();
    detach(&mut dock, &mut windows, EditorTab::Inspector);
    assert!(!crate::state::dock_has_tab(&dock, &EditorTab::Inspector));
    assert_eq!(windows.detached.len(), 1);
    assert!(shown(&dock, &windows, EditorTab::Inspector));
}

/// Closing its window puts the panel back in the dock.
#[test]
fn a_closed_window_docks_back() {
    let mut dock = default_dock_state();
    let mut windows = OsWindows::default();
    detach(&mut dock, &mut windows, EditorTab::Console);
    dock_back(&mut dock, &mut windows, EditorTab::Console);
    assert!(crate::state::dock_has_tab(&dock, &EditorTab::Console));
    assert!(windows.detached.is_empty());
}

/// Closing from the Window menu hides the panel: it is neither docked nor detached.
#[test]
fn a_menu_close_hides_the_panel() {
    let mut dock = default_dock_state();
    let mut windows = OsWindows::default();
    detach(&mut dock, &mut windows, EditorTab::Inspector);
    close(&mut windows, EditorTab::Inspector);
    assert!(!shown(&dock, &windows, EditorTab::Inspector));
}

/// A layout that lists a tab both ways keeps it in its window only.
#[test]
fn a_saved_window_wins_over_dock() {
    let mut dock = default_dock_state();
    let windows = OsWindows {
        detached: vec![Detached {
            tab: EditorTab::Inspector,
            size: DEFAULT_SIZE,
            pos: None,
            home: None,
        }],
        ..Default::default()
    };
    settle(&mut dock, &windows);
    assert!(!crate::state::dock_has_tab(&dock, &EditorTab::Inspector));
}

/// The OS title carries the panel's name and not its icon glyph, which a window list cannot draw.
#[test]
fn a_title_drops_the_icon() {
    assert_eq!(title_of(EditorTab::Inspector), "Kóoch — Inspector");
}

/// A torn-off panel gets a native window, not a window drawn inside the main one.
#[test]
fn viewports_are_not_embedded() {
    let ctx = egui::Context::default();
    install(&ctx, SharedLive::default());
    assert!(!ctx.embed_viewports());
}

/// A nested pass shares the main pass's texture limit, or egui rebuilds the font atlas under it.
#[test]
fn a_nested_pass_keeps_the_atlas() {
    let ctx = egui::Context::default();
    install(&ctx, SharedLive::default());
    let mut seen = None;
    let input = egui::RawInput {
        max_texture_side: Some(4096),
        ..Default::default()
    };
    let _ = ctx.run_ui(input, |ui| {
        let id = viewport_of(EditorTab::Console);
        ui.ctx()
            .show_viewport_immediate(id, egui::ViewportBuilder::default(), |ui, _| {
                seen = Some(ui.ctx().input(|i| i.max_texture_side));
            });
    });
    assert_eq!(seen, Some(4096));
}

/// Same layout; which leaf has the keyboard is not part of a panel's place.
fn same(a: &DockState<EditorTab>, b: &DockState<EditorTab>) -> bool {
    let shape = |dock: &DockState<EditorTab>| {
        let text = ron::ser::to_string(dock).unwrap();
        let start = text.find("focused_node:").unwrap();
        let end = start + text[start..].find(",collapsed").unwrap();
        format!("{}{}", &text[..start], &text[end..])
    };
    shape(a) == shape(b)
}

/// A panel with a split of its own comes back to that side, at that size.
#[test]
fn a_split_panel_returns_home() {
    for tab in [EditorTab::Inspector, EditorTab::World] {
        let before = default_dock_state();
        let mut dock = before.clone();
        let mut windows = OsWindows::default();
        detach(&mut dock, &mut windows, tab);
        dock_back(&mut dock, &mut windows, tab);
        assert!(same(&dock, &before), "{tab:?} came back elsewhere");
    }
}

/// A panel that shared a leaf comes back to that leaf, at its place among the tabs.
#[test]
fn a_shared_panel_returns_home() {
    let mut before = default_dock_state();
    let game = before.find_tab(&EditorTab::View).unwrap();
    before.set_active_tab(game);
    let mut dock = before.clone();
    let mut windows = OsWindows::default();
    detach(&mut dock, &mut windows, EditorTab::View);
    dock_back(&mut dock, &mut windows, EditorTab::View);
    assert!(same(&dock, &before));
}

/// A dock reshaped while the panel was away still takes it back.
#[test]
fn a_reshaped_dock_still_takes_it() {
    let mut dock = default_dock_state();
    let mut windows = OsWindows::default();
    detach(&mut dock, &mut windows, EditorTab::World);
    let console = dock.find_tab(&EditorTab::Game).unwrap();
    dock.main_surface_mut()
        .split_below(console.node, 0.7, vec![EditorTab::Console]);
    dock_back(&mut dock, &mut windows, EditorTab::World);
    assert!(crate::state::dock_has_tab(&dock, &EditorTab::World));
    assert!(crate::state::dock_has_tab(&dock, &EditorTab::Console));
}

/// A panel window never waits on vsync when the platform offers anything else.
#[test]
fn a_panel_window_skips_vsync() {
    use wgpu::PresentMode::*;
    assert_eq!(present_mode(&[Fifo, Mailbox, Immediate]), Mailbox);
    assert_eq!(present_mode(&[Fifo, Immediate]), Immediate);
    assert_eq!(present_mode(&[Fifo]), Fifo);
}
