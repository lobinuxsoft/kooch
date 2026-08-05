//! Real-world light levels, by name.
//!
//! A light's intensity is a number with a unit and no scale attached.
//! 10 000 what? Bright compared to what? These constants are the answer,
//! and they exist so an author can pick a *situation* instead of guessing
//! a magnitude.
//!
//! Ported from Bevy's `light_consts`, which is itself sourced from
//! Wikipedia's lux and lumen articles. The values are physical facts, so
//! there is nothing to improve on — what is worth taking is that they are
//! named at all.
//!
//! # 🔴 The honest caveat, which Bevy does not write down
//!
//! These numbers describe a world with **indirect light**. An office is
//! 320 lux because the light bounces off the ceiling, the walls and the
//! desk. Kóoch computes direct light only, so a physically-correct 9 W
//! bulb three metres away delivers its honest 7 lux and looks like
//! nothing.
//!
//! Bevy resolved this by defaulting `PointLight` to
//! `VERY_LARGE_CINEMA_LIGHT` — one million lumens — with the comment
//! *"capable of registering brightly at Bevy's default exposure level"*.
//! That is a fudge, and an undocumented fudge is a trap.
//!
//! The real fixes, in order: exposure the author controls (here today),
//! auto exposure (#254), and global illumination (#450). Until then, a
//! punctual light in Kóoch is worth roughly an order of magnitude more
//! than its real-world twin, and every default below says so.

/// Illuminance, in lux. What a **directional light** (a sun) measures.
pub mod lux {
    /// A moonless, overcast night sky — starlight alone.
    pub const MOONLESS_NIGHT: f32 = 0.0001;
    /// A full moon on a clear night.
    pub const FULL_MOON_NIGHT: f32 = 0.05;
    /// The dark limit of civil twilight, clear sky.
    pub const CIVIL_TWILIGHT: f32 = 3.4;
    /// Family living room lighting.
    pub const LIVING_ROOM: f32 = 50.0;
    /// An office building's hallway.
    pub const HALLWAY: f32 = 80.0;
    /// A very dark overcast day.
    pub const DARK_OVERCAST_DAY: f32 = 100.0;
    /// An office.
    pub const OFFICE: f32 = 320.0;
    /// Sunrise or sunset on a clear day.
    pub const CLEAR_SUNRISE: f32 = 400.0;
    /// An overcast day; also typical TV studio lighting.
    pub const OVERCAST_DAY: f32 = 1_000.0;
    /// Ambient daylight, not direct sun. **The `DirectionalLight`
    /// default**, here and in Bevy.
    pub const AMBIENT_DAYLIGHT: f32 = 10_000.0;
    /// Full daylight, not direct sun.
    pub const FULL_DAYLIGHT: f32 = 20_000.0;
    /// Direct sunlight.
    pub const DIRECT_SUNLIGHT: f32 = 100_000.0;
    /// Raw sunlight, unfiltered by an atmosphere. What a light outside a
    /// planet's air actually delivers.
    pub const RAW_SUNLIGHT: f32 = 130_000.0;
}

/// Luminous flux, in lumens. What a **point or spot light** emits in
/// every direction combined.
///
/// ⚠️ Every value here is a real bulb's real output, and every one of
/// them is dimmer than it looks in a scene with no bounce light. See the
/// module docs. [`ROOM_LIGHT_NO_GI`] is the one calibrated for this
/// renderer rather than for reality.
pub mod lumens {
    /// A candle.
    pub const CANDLE: f32 = 12.0;
    /// A 9 W LED bulb — a normal household lamp.
    pub const LED_BULB_9W: f32 = 800.0;
    /// A 100 W incandescent bulb.
    pub const INCANDESCENT_100W: f32 = 1_600.0;
    /// A bright shop or garage fixture.
    pub const SHOP_LIGHT: f32 = 5_000.0;
    /// A car headlight on high beam.
    pub const CAR_HEADLIGHT: f32 = 20_000.0;
    /// 🔴 **Not a real bulb.** What a room light has to emit to read as a
    /// room light with direct lighting only — roughly forty times a real
    /// 9 W LED, standing in for the bounces this renderer does not
    /// compute.
    ///
    /// The `PointLight` and `SpotLight` default. It goes back down to
    /// [`LED_BULB_9W`] the day #450 lands; that is the point of naming it
    /// after the compromise instead of after a fixture.
    pub const ROOM_LIGHT_NO_GI: f32 = 32_000.0;
    /// A stadium floodlight.
    pub const FLOODLIGHT: f32 = 200_000.0;
    /// A very large cinema light. Bevy's `PointLight` default, listed
    /// here for the comparison rather than as a recommendation.
    pub const VERY_LARGE_CINEMA_LIGHT: f32 = 1_000_000.0;
}

#[cfg(test)]
mod tests {
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
}
