use super::*;

/// The element rows are zipped against `ElementMode::ALL` in `collect`, so a row added to one and
/// not the other silently shifts every key by one.
#[test]
fn the_element_rows_match_modes() {
    assert_eq!(ELEMENTS.len(), crate::block_edit::ElementMode::ALL.len());
}

/// A pointer gesture has no key, and asking it whether it fired must not answer yes — the overlay
/// shares this table with the code that acts on it.
#[test]
fn a_mouse_chord_never_fires() {
    let input = egui::InputState::default();
    assert!(!Command::mouse("RMB", "Fly").pressed(&input));
}
