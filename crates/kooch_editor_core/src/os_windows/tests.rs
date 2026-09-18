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
