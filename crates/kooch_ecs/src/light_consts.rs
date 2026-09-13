//! Real-world light levels, by name.

/// Illuminance, in lux. What a **directional light** (a sun) measures.
pub mod lux {
    /// Ambient daylight, not direct sun. **The `DirectionalLight`
    /// default**, here and in Bevy.
    pub const AMBIENT_DAYLIGHT: f32 = 10_000.0;
    /// Direct sunlight.
    pub const DIRECT_SUNLIGHT: f32 = 100_000.0;
}

/// Luminous flux, in lumens. What a **point or spot light** emits in every direction combined.
pub mod lumens {
    /// A 9 W LED bulb — a normal household lamp.
    pub const LED_BULB_9W: f32 = 800.0;
    /// 🔴 **Not a real bulb.** What a room light has to emit to read as a room light with direct
    /// lighting only — roughly forty times a real 9 W LED, standing in for the bounces this
    /// renderer does not compute.
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
