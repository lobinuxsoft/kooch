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
    /// Ambient daylight, not direct sun. **The `DirectionalLight`
    /// default**, here and in Bevy.
    pub const AMBIENT_DAYLIGHT: f32 = 10_000.0;
    /// Direct sunlight.
    pub const DIRECT_SUNLIGHT: f32 = 100_000.0;
}

/// Luminous flux, in lumens. What a **point or spot light** emits in
/// every direction combined.
///
/// ⚠️ Every value here is a real bulb's real output, and every one of
/// them is dimmer than it looks in a scene with no bounce light. See the
/// module docs. [`ROOM_LIGHT_NO_GI`] is the one calibrated for this
/// renderer rather than for reality.
pub mod lumens {
    /// A 9 W LED bulb — a normal household lamp.
    pub const LED_BULB_9W: f32 = 800.0;
    /// 🔴 **Not a real bulb.** What a room light has to emit to read as a
    /// room light with direct lighting only — roughly forty times a real
    /// 9 W LED, standing in for the bounces this renderer does not
    /// compute.
    ///
    /// The `PointLight` and `SpotLight` default. It goes back down to
    /// [`LED_BULB_9W`] the day #450 lands; that is the point of naming it
    /// after the compromise instead of after a fixture.
    pub const ROOM_LIGHT_NO_GI: f32 = 32_000.0;
    #[cfg(test)]
    /// A stadium floodlight.
    pub const FLOODLIGHT: f32 = 200_000.0;
    /// A very large cinema light. Bevy's `PointLight` default, listed
    /// here for the comparison rather than as a recommendation.
    pub const VERY_LARGE_CINEMA_LIGHT: f32 = 1_000_000.0;
}

#[cfg(test)]
mod tests;
