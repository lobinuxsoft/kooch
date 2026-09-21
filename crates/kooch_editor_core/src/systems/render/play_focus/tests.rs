use super::*;

fn dock() -> DockState<EditorTab> {
    // The Edit View in front, the Game panel behind it in the same leaf — the default layout.
    DockState::new(vec![EditorTab::View, EditorTab::Game])
}

/// Whether `tab` is the one its leaf is showing.
fn shown(dock: &DockState<EditorTab>, tab: EditorTab) -> bool {
    let path = dock.find_tab(&tab).expect("the panel is docked");
    dock[path.surface]
        .leaf(path.node)
        .is_ok_and(|leaf| leaf.active == path.tab)
}

/// 🔴 Playing shows the game and hands it the keyboard; stopping gives both back to the Edit View.
#[test]
fn play_brings_the_game_forward() {
    let (mut dock, mut focused, mut was) = (dock(), None, false);
    let started = follow(&mut dock, &mut focused, &mut was, true);
    assert!(started);
    assert_eq!(focused, Some(EditorTab::Game));
    assert!(
        shown(&dock, EditorTab::Game),
        "the game panel stayed behind"
    );

    let started = follow(&mut dock, &mut focused, &mut was, false);
    assert!(!started);
    assert_eq!(focused, Some(EditorTab::View));
    assert!(shown(&dock, EditorTab::View), "the edit view stayed behind");
}

/// Only the edge moves anything: a panel the author switches to mid-play stays where they put it.
#[test]
fn a_held_state_moves_nothing() {
    let (mut dock, mut was) = (dock(), true);
    let mut focused = Some(EditorTab::Inspector);
    assert!(!follow(&mut dock, &mut focused, &mut was, true));
    assert_eq!(focused, Some(EditorTab::Inspector));
}

#[test]
fn the_chord_toggles() {
    assert!(matches!(toggled(false), EditorAction::Play));
    assert!(matches!(toggled(true), EditorAction::Stop));
}
