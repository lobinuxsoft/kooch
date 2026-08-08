//! What the author decides about shadows.
//!
//! A `Resource` with a default, published from `.rendersettings` the
//! same way exposure and ambient are (#744). Separate from
//! `RenderSettings` as a type because the shadow pass reads it and the
//! shading model does not — and because a `Resource` nobody can author
//! is the failure this engine has now committed three times.

/// How far from the camera shadows are drawn, in metres.
///
/// Two hundred is a scene, not a planet. The cascades are fitted to
/// whatever range they are given, so raising this does not add shadows
/// in the distance — it moves the near cascade's texels outward and
/// blurs the shadows that are actually being looked at.
pub const DEFAULT_SHADOW_DISTANCE: f32 = 100.0;

/// Where the first cascade ends, in metres.
///
/// The split scheme is logarithmic from here, so this is the single
/// number that decides how much resolution the shadows near the camera
/// get. Unity ships 10.05, Godot 10, and Bevy takes both as its
/// reference — ten metres around the camera is what a scene at human
/// scale wants, and anchoring at the camera's near plane instead spends
/// the first cascade on the first few centimetres.
pub const DEFAULT_FIRST_CASCADE_DISTANCE: f32 = 10.0;

/// Side of one cascade in texels, when the author has not said.
///
/// Mirrors [`crate::shadow::DEFAULT_CASCADE_SIZE`]; stated here so the
/// settings type has a complete default without the caller reaching for
/// the atlas.
pub const DEFAULT_CASCADE_TEXELS: u32 = super::atlas::DEFAULT_CASCADE_SIZE;

/// Shadow settings, as a `Resource`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowSettings {
    /// Distance from the camera the cascades cover, in metres.
    pub max_distance: f32,
    /// Side of one cascade in texels. The atlas is twice this on each
    /// axis, so 2048 costs 64 MiB and 1024 costs 16.
    pub cascade_texels: u32,
    /// Whether the pass runs at all. Off means no atlas is allocated —
    /// the sixty-four megabytes are not spent by a project that does not
    /// want shadows.
    pub enabled: bool,
    /// Tangent of the sun's angular radius: how much wider a shadow gets
    /// per metre between the blocker and the surface it lands on.
    pub sun_softness: f32,
    /// Where the first cascade ends, in metres. The rest follow
    /// logarithmically out to `max_distance`.
    pub first_cascade_distance: f32,
}

impl Default for ShadowSettings {
    fn default() -> Self {
        Self {
            max_distance: DEFAULT_SHADOW_DISTANCE,
            cascade_texels: DEFAULT_CASCADE_TEXELS,
            enabled: true,
            sun_softness: kooch_lighting::DEFAULT_SUN_SOFTNESS,
            first_cascade_distance: DEFAULT_FIRST_CASCADE_DISTANCE,
        }
    }
}

impl ShadowSettings {
    /// Cascade size clamped to something a device will allocate.
    ///
    /// The atlas is twice this per axis, and 8192 is the smallest
    /// `max_texture_dimension_2d` any target guarantees — so a cascade
    /// larger than 4096 is a texture creation failure, which surfaces as
    /// a panic in wgpu rather than as a bad-looking shadow.
    pub fn clamped_texels(&self) -> u32 {
        self.cascade_texels.clamp(256, 4096)
    }
}

#[cfg(test)]
mod tests;
