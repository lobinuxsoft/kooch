use super::*;

#[test]
fn the_no_gi_default_sits_between_a_real_bulb_and_a_floodlight() {
    // If this ever inverts, the compromise value stopped being a
    // compromise and became a fudge nobody can justify.
    assert!(lumens::ROOM_LIGHT_NO_GI > lumens::LED_BULB_9W);
    assert!(lumens::ROOM_LIGHT_NO_GI < lumens::FLOODLIGHT);
}

#[test]
fn lux_values_are_ordered_by_how_bright_they_describe() {
    let ladder = [
        lux::MOONLESS_NIGHT,
        lux::FULL_MOON_NIGHT,
        lux::CIVIL_TWILIGHT,
        lux::LIVING_ROOM,
        lux::HALLWAY,
        lux::DARK_OVERCAST_DAY,
        lux::OFFICE,
        lux::CLEAR_SUNRISE,
        lux::OVERCAST_DAY,
        lux::AMBIENT_DAYLIGHT,
        lux::FULL_DAYLIGHT,
        lux::DIRECT_SUNLIGHT,
        lux::RAW_SUNLIGHT,
    ];
    assert!(
        ladder.windows(2).all(|w| w[0] < w[1]),
        "a constant is out of order, so at least one name lies",
    );
}
