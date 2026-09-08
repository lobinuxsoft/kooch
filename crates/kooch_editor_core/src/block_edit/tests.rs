use super::{BlockSelection, ElementMode};
use kooch_ecs::entity::Entity;

fn entity(index: u32) -> Entity {
    Entity::new(index, 0)
}

#[test]
fn object_is_the_default_mode() {
    // Every other kind of entity wants clicks to select entities.
    assert_eq!(ElementMode::default(), ElementMode::Object);
}

#[test]
fn a_fresh_selection_holds_nothing() {
    let selection = BlockSelection::default();
    assert!(selection.is_empty());
    assert!(!selection.holds(entity(1), 0));
}

#[test]
fn only_replaces_what_was_held() {
    let mut selection = BlockSelection::default();
    selection.only(entity(1), 3);
    selection.only(entity(1), 5);
    assert_eq!(selection.faces, vec![5]);
}

#[test]
fn toggle_adds_then_removes() {
    let mut selection = BlockSelection::default();
    selection.toggle(entity(1), 2);
    selection.toggle(entity(1), 4);
    assert_eq!(selection.faces, vec![2, 4]);
    selection.toggle(entity(1), 2);
    assert_eq!(selection.faces, vec![4]);
}

#[test]
fn another_block_clears_rather_than_merges() {
    // 🔴 The faces are indices into ONE mesh. Keeping the previous
    // block's would address faces that may not exist in this one.
    let mut selection = BlockSelection::default();
    selection.toggle(entity(1), 5);
    selection.toggle(entity(2), 0);
    assert_eq!(selection.entity, Some(entity(2)));
    assert_eq!(selection.faces, vec![0]);
}

#[test]
fn a_face_of_another_block_is_not_held() {
    let mut selection = BlockSelection::default();
    selection.only(entity(1), 3);
    assert!(selection.holds(entity(1), 3));
    assert!(!selection.holds(entity(2), 3));
}

#[test]
fn clearing_empties_both_halves() {
    let mut selection = BlockSelection::default();
    selection.only(entity(1), 3);
    selection.clear();
    assert!(selection.is_empty());
    assert_eq!(selection.entity, None);
}

#[test]
fn a_click_on_a_face_selects_it() {
    let mut selection = BlockSelection::default();
    super::apply_click(&mut selection, entity(1), Some(2), false);
    assert_eq!(selection.faces, vec![2]);
}

#[test]
fn a_ctrl_click_adds_to_the_selection() {
    let mut selection = BlockSelection::default();
    super::apply_click(&mut selection, entity(1), Some(2), false);
    super::apply_click(&mut selection, entity(1), Some(4), true);
    assert_eq!(selection.faces, vec![2, 4]);
}

#[test]
fn a_click_on_nothing_clears() {
    let mut selection = BlockSelection::default();
    super::apply_click(&mut selection, entity(1), Some(2), false);
    super::apply_click(&mut selection, entity(1), None, false);
    assert!(selection.is_empty());
}

#[test]
fn a_ctrl_click_on_nothing_keeps_it() {
    // A miss, not "deselect everything" — the same rule entity picking
    // follows, so building a selection does not depend on aim.
    let mut selection = BlockSelection::default();
    super::apply_click(&mut selection, entity(1), Some(2), false);
    super::apply_click(&mut selection, entity(1), None, true);
    assert_eq!(selection.faces, vec![2]);
}
