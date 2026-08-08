use super::text_to_show;

/// The reported bug, stated as the rule that caused it: with focus,
/// the lagging snapshot must not win. It is one character behind what
/// was just typed, and egui pulls the caret back to fit.
#[test]
fn the_typed_text_wins_while_the_field_has_focus() {
    let shown = text_to_show(true, Some("Doo".to_owned()), "Do");
    assert_eq!(shown, "Doo", "the stale snapshot overwrote the keystroke");
}

/// Focused with nothing typed yet — the first frame after clicking in.
#[test]
fn focus_without_a_buffer_falls_back_to_the_world() {
    assert_eq!(text_to_show(true, None, "Door frame"), "Door frame");
}

/// Unfocused, the world is authoritative: a rename the project
/// altered or refused has to show what actually landed.
#[test]
fn the_world_wins_once_focus_is_gone() {
    let shown = text_to_show(false, Some("what I typed".to_owned()), "what landed");
    assert_eq!(shown, "what landed");
}
