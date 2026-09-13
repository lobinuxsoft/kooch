use super::*;

#[test]
fn the_no_gi_default_sits_between_a_real_bulb_and_a_floodlight() {
    // If this ever inverts, the compromise value stopped being a
    // compromise and became a fudge nobody can justify.
    assert!(lumens::ROOM_LIGHT_NO_GI > lumens::LED_BULB_9W);
    assert!(lumens::ROOM_LIGHT_NO_GI < lumens::FLOODLIGHT);
}
