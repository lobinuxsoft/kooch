use super::*;

#[test]
fn the_focused_panel_owns_input() {
    assert_eq!(
        resolve(Some(EditorTab::View), false),
        InputOwner::ViewCamera
    );
    assert_eq!(resolve(Some(EditorTab::Game), false), InputOwner::Game);
}

#[test]
fn play_state_is_not_part_of_the_rule() {
    // Deliberately absent from the signature. The Game panel gets
    // input because it is focused, not because something is running;
    // the View keeps its camera while the game plays beside it.
    // Anyone reintroducing play state here has to change the
    // signature, which is the point.
    assert_eq!(
        resolve(Some(EditorTab::View), false),
        InputOwner::ViewCamera
    );
}

#[test]
fn a_panel_that_does_not_take_input_owns_nothing() {
    assert_eq!(resolve(Some(EditorTab::World), false), InputOwner::None);
    assert_eq!(resolve(Some(EditorTab::Inspector), false), InputOwner::None);
    assert_eq!(resolve(None, false), InputOwner::None);
}

#[test]
fn a_focused_text_field_takes_the_keyboard_from_everyone() {
    assert_eq!(resolve(Some(EditorTab::View), true), InputOwner::None);
    assert_eq!(resolve(Some(EditorTab::Game), true), InputOwner::None);
}

/// The parameter is `text_edit_focused`, not "some widget has focus".
///
/// Reading it the loose way is what broke the View: the editor opens
/// with a widget focused, so the camera had no keyboard until the
/// first Play click moved that focus. The name here is the guard —
/// a caller passing `egui_wants_keyboard_input` is passing the wrong
/// question, and the name says so at the call site.
#[test]
fn only_a_text_field_takes_it_not_any_focused_widget() {
    assert_eq!(
        resolve(Some(EditorTab::View), false),
        InputOwner::ViewCamera,
        "a focused button is not someone typing",
    );
}

#[test]
fn exactly_one_owner_is_representable() {
    // Not a runtime assertion — a note that the type is the
    // guarantee. Two owners cannot be expressed, so "both the camera
    // and the game moved" cannot happen through this path.
    let focus = InputFocus {
        owner: InputOwner::Game,
    };
    assert!(focus.belongs_to(InputOwner::Game));
    assert!(!focus.belongs_to(InputOwner::ViewCamera));
    assert!(!focus.belongs_to(InputOwner::None));
}
