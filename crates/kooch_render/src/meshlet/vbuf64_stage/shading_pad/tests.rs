use super::{extend_slots, parse_pad};

#[test]
fn an_unset_pad_is_zero() {
    assert_eq!(parse_pad(None), 0);
}

#[test]
fn an_unparseable_pad_is_zero() {
    // The run this exists for is launched by typing a Steam launch
    // option on a handheld. " 4 " and "four" have to differ: one is a
    // measurement, the other is a mistake that must not look like one.
    assert_eq!(parse_pad(Some(" 4 ")), 4);
    assert_eq!(parse_pad(Some("four")), 0);
    assert_eq!(parse_pad(Some("-1")), 0);
    assert_eq!(parse_pad(Some("")), 0);
}

#[test]
fn an_unset_pad_changes_nothing() {
    assert_eq!(extend_slots(0..4, 0, 256), 0..4);
}

#[test]
fn the_pad_appends_never_prepends() {
    // The fragment path clears the colour target on the first slot of
    // the range and loads on every one after it, so a pad slot that
    // moved `start` would clear the frame and composite every real
    // material over nothing.
    assert_eq!(extend_slots(0..4, 4, 256), 0..8);
    assert_eq!(extend_slots(2..4, 4, 256), 2..8);
}

#[test]
fn the_pad_clamps_to_max() {
    // `screen_buffer` holds exactly `max` `ScreenUbo`s and the shader
    // indexes it by slot; a pad that ran past it would write outside
    // the buffer on a device with nobody watching.
    assert_eq!(extend_slots(0..4, 1_000, 256), 0..256);
    assert_eq!(extend_slots(0..4, u32::MAX, 256), 0..256);
}
