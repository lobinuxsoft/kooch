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

/// 🔴 Object mode stops the geometry being edited.
///
/// The gate was on whether a selection existed, not on the mode — so
/// switching to Object left the handle reshaping the block while the
/// author was asking to move it. Leaving faces selected while switching
/// is how you check what you just built.
#[test]
fn object_mode_is_not_face_mode() {
    assert_ne!(ElementMode::Object, ElementMode::Face);
    assert_eq!(ElementMode::Object.label(), "Object");
    assert_eq!(ElementMode::Face.label(), "Face");
}

/// Builds resources holding one face selection.
fn with_selection() -> kooch_core::resource::Resources {
    let mut resources = kooch_core::resource::Resources::new();
    let mut selection = BlockSelection::default();
    selection.only(entity(1), 3);
    resources.insert(selection);
    resources
}

fn held(resources: &kooch_core::resource::Resources) -> bool {
    !resources
        .get::<BlockSelection>()
        .map(|s| s.is_empty())
        .unwrap_or(true)
}

#[test]
fn face_mode_keeps_the_selection() {
    let mut resources = with_selection();
    super::drop_selection_unless_editing(&mut resources, ElementMode::Face, false);
    assert!(held(&resources));
}

#[test]
fn object_mode_drops_the_selection() {
    // 🔴 Cleared, not merely gated. A painted highlight over a grabbable
    // gizmo that records nothing looks like it worked.
    let mut resources = with_selection();
    super::drop_selection_unless_editing(&mut resources, ElementMode::Object, false);
    assert!(!held(&resources));
}

#[test]
fn play_drops_the_selection() {
    // The world Play restores is not the one these face indices were
    // read from.
    let mut resources = with_selection();
    super::drop_selection_unless_editing(&mut resources, ElementMode::Face, true);
    assert!(!held(&resources));
}

#[test]
fn dropping_twice_is_quiet() {
    let mut resources = with_selection();
    super::drop_selection_unless_editing(&mut resources, ElementMode::Object, false);
    super::drop_selection_unless_editing(&mut resources, ElementMode::Object, false);
    assert!(!held(&resources));
}

#[test]
fn face_mode_reaches_the_drawing() {
    // 🔴 The gizmo reads Resources, and the overlay is taken OUT of
    // Resources for the frame that draws it. The mode has to be
    // mirrored somewhere the drawing can see, or the wireframe never
    // appears however the toolbar looks.
    let mut resources = with_selection();
    super::drop_selection_unless_editing(&mut resources, ElementMode::Face, false);
    assert_eq!(
        resources.get::<BlockSelection>().map(|s| s.mode),
        Some(ElementMode::Face),
    );
}

#[test]
fn object_mode_reaches_it_too() {
    let mut resources = with_selection();
    super::drop_selection_unless_editing(&mut resources, ElementMode::Object, false);
    assert_eq!(
        resources.get::<BlockSelection>().map(|s| s.mode),
        Some(ElementMode::Object),
    );
}

#[test]
fn play_reads_as_object_to_the_drawing() {
    // Nothing is editable during Play, so nothing should look editable.
    let mut resources = with_selection();
    super::drop_selection_unless_editing(&mut resources, ElementMode::Face, true);
    assert_eq!(
        resources.get::<BlockSelection>().map(|s| s.mode),
        Some(ElementMode::Object),
    );
}
