use super::*;

fn id(n: u32) -> ComponentId {
    ComponentId(n)
}

fn nav() -> InspectorNav {
    InspectorNav {
        rows: vec![id(1), id(2), id(3)],
        ..Default::default()
    }
}

#[test]
fn the_first_step_lands_on_the_first_section() {
    let mut n = nav();
    n.step(1);
    assert_eq!(n.cursor, Some(id(1)));
}

#[test]
fn stepping_stops_at_both_ends() {
    let mut n = nav();
    n.cursor = Some(id(1));
    n.step(-4);
    assert_eq!(n.cursor, Some(id(1)));
    n.step(9);
    assert_eq!(n.cursor, Some(id(3)));
}

#[test]
fn opening_and_closing_name_the_section_under_the_cursor() {
    let mut n = nav();
    n.cursor = Some(id(2));
    n.set_open(false);
    assert_eq!(n.toggle, Some((id(2), false)));
    n.set_open(true);
    assert_eq!(n.toggle, Some((id(2), true)));
}

/// A removed component, or a different entity selected, leaves a
/// cursor with no section. The next key has to recover rather than do
/// nothing.
#[test]
fn a_cursor_on_a_component_that_is_gone_recovers() {
    let mut n = nav();
    n.cursor = Some(id(99));
    n.step(1);
    assert_eq!(n.cursor, Some(id(1)));
}

/// Right with no cursor should place one, not be swallowed.
#[test]
fn the_first_arrow_places_a_cursor_even_when_it_is_a_toggle() {
    let mut n = nav();
    n.set_open(true);
    assert_eq!(n.cursor, Some(id(1)));
    assert_eq!(n.toggle, None, "nothing to toggle until a cursor exists");
}

#[test]
fn a_toggle_is_only_taken_by_the_component_it_names() {
    let mut n = nav();
    n.toggle = Some((id(3), true));
    assert_eq!(n.take_toggle_for(id(1)), None);
    assert_eq!(n.take_toggle_for(id(3)), Some(true));
    assert_eq!(n.take_toggle_for(id(3)), None, "taken once");
}

#[test]
fn nothing_selected_means_nowhere_for_a_cursor() {
    let mut n = InspectorNav::default();
    n.step(1);
    n.set_open(true);
    assert_eq!(n.cursor, None);
}
