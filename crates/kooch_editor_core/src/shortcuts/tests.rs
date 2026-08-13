use kooch_ecs::entity::Entity;

use super::*;

fn entity(index: u32) -> Entity {
    Entity::new(index, 0)
}

/// The two panels that show entities are the two that get the chords.
#[test]
fn the_entity_panels_hear_them() {
    assert!(allowed(Some(EditorTab::World), false));
    assert!(allowed(Some(EditorTab::View), false));
}

/// 🔴 The Console, the Assets panel and the Inspector do not. Ctrl+D in
/// the Assets panel is about a file, and Ctrl+C in the Console is about
/// a log line — both of those chords already mean something there.
#[test]
fn other_panels_do_not() {
    assert!(!allowed(Some(EditorTab::Console), false));
    assert!(!allowed(Some(EditorTab::AssetBrowser), false));
    assert!(!allowed(Some(EditorTab::Inspector), false));
    assert!(!allowed(None, false));
}

/// Typing takes the keyboard from everyone, whichever panel is focused
/// — the same rule `input_focus` applies to the camera.
#[test]
fn typing_takes_them_all() {
    assert!(!allowed(Some(EditorTab::World), true));
    assert!(!allowed(Some(EditorTab::View), true));
}

/// One action per selected entity, which is what lets the dispatch
/// layer batch them into a single undo step.
#[test]
fn duplicate_acts_on_each() {
    let actions = actions_for(EditChord::Duplicate, &[entity(1), entity(2)]);
    assert_eq!(actions.len(), 2);
    assert!(matches!(actions[0], EditorAction::Duplicate(_)));
}

/// A chord with nothing to act on does nothing at all, rather than
/// queueing an action the dispatch layer has to recognise as empty.
#[test]
fn an_empty_selection_asks_nothing() {
    assert!(actions_for(EditChord::Duplicate, &[]).is_empty());
    assert!(actions_for(EditChord::Copy, &[]).is_empty());
}

/// Paste is the exception: it acts on the clipboard, so it fires with
/// nothing selected. Whether it has anything to build is the
/// clipboard's answer, not the selection's.
#[test]
fn paste_ignores_the_selection() {
    let actions = actions_for(EditChord::Paste, &[]);
    assert!(matches!(actions.as_slice(), [EditorAction::PasteEntities]));
}

/// Every chord is unique, and every one of them says so in the UI. A
/// duplicate key would give two commands to one press and the loop in
/// `gather` would run both.
#[test]
fn every_chord_is_distinct() {
    for (i, a) in ALL.iter().enumerate() {
        for b in &ALL[i + 1..] {
            assert_ne!(a.key(), b.key(), "{a:?} and {b:?} share a key");
            assert_ne!(a.chord(), b.chord(), "{a:?} and {b:?} print the same");
        }
        assert!(!a.tooltip().is_empty(), "{a:?} has no tooltip");
    }
}
