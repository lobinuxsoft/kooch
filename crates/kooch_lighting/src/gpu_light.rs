//! The GPU light record and its conversion. AoS on purpose: shaders read all of light `i` at once,
//! so one 80 B record beats scattered fetches; clustering reads it once per pass, not per pixel.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

use kooch_ecs::directional_light::DirectionalLight;
use kooch_ecs::point_light::PointLight;
use kooch_ecs::spot_light::SpotLight;

/// Discriminants shared with `inti_pbr.wgsl`'s `INTI_KIND_*`.
pub const LIGHT_KIND_DIRECTIONAL: u32 = 0;
pub const LIGHT_KIND_POINT: u32 = 1;
pub const LIGHT_KIND_SPOT: u32 = 2;

/// Smallest cone width the spot MAD can encode. Inner and outer angles
/// meeting exactly would divide by zero; a value being typed into the
/// Inspector passes through that state on the way to the intended one.
const MIN_CONE_COS_DELTA: f32 = 1e-4;

/// [`GpuLight::shadow_slot`] when the light casts no shadow. Every spot
/// light past [`MAX_SPOT_SHADOWS`](crate::MAX_SPOT_SHADOWS) carries it
/// too: such a light still lights the scene, it just has no map.
pub const NO_SHADOW_SLOT: u32 = u32::MAX;

/// One light as the shader reads it, 80 B `std430`, mirroring `IntiLight` byte for byte; the size
/// test in `gpu_light/tests.rs` stands in for a compiler.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct GpuLight {
    /// Linear RGB.
    pub color: [f32; 3],
    /// Photometric, unconverted (lux directional, lumens punctual), so Inspector and capture show
    /// the same number.
    pub intensity: f32,
    /// World-space. Meaningless for directional lights.
    pub position: [f32; 3],
    /// Attenuation cutoff. Meaningless for directional lights.
    pub range: f32,
    /// World-space unit vector along the entity's -Z — where the light
    /// points. Meaningless for point lights.
    pub direction: [f32; 3],
    /// One of the `LIGHT_KIND_*` discriminants.
    pub kind: u32,
    /// Cone falloff as `saturate(cos_angle * scale + offset)`, one MAD
    /// per light per fragment. See [`spot_cone_mad`].
    pub spot_scale: f32,
    pub spot_offset: f32,
    /// Per-light opt-ins, one bit each. See [`GpuLight::FLAG_CONTACT_SHADOWS`].
    pub flags: u32,
    /// Index into `IntiFrame::spot_shadows` when this spot light casts,
    /// or [`NO_SHADOW_SLOT`] (#777).
    pub shadow_slot: u32,
    /// Emitting sphere radius, scaled like `range`; `0` is a point. It grew the record 64 → 80 B
    /// (#776) — `position.w` already holds `range`.
    pub radius: f32,
    /// Padding to `std430`'s 16-byte vec3 alignment. 🔴 Already fetched per light per fragment, so a
    /// field fitting here is free and the next costs 16 B per light.
    /// ⚠️ Three scalars, never `[f32; 3]`, or WGSL realigns.
    pub _pad0: f32,
    /// Layers this light lights (#1220). A surface sharing no bit with it takes nothing from this
    /// light — the test is per pixel, in the light loop, and the padding made it free.
    pub layers: u32,
    /// Layers that cast into this light's shadow (#1220). Read by the lamp cull, which picks its
    /// casters on the GPU and has no CPU pass to filter them in.
    pub shadow_layers: u32,
}

/// 🔴 Every layer, not zero. A record built by hand — a test fixture, a slot the buffer pads with —
/// is a light that lights and shadows everything, which is what a light nobody masked means. Zeroed
/// masks would make every hand-built light invisible and every hand-built lamp cast nothing (#1220).
impl Default for GpuLight {
    fn default() -> Self {
        Self {
            color: [0.0; 3],
            intensity: 0.0,
            position: [0.0; 3],
            range: 0.0,
            direction: [0.0; 3],
            kind: LIGHT_KIND_DIRECTIONAL,
            spot_scale: 0.0,
            spot_offset: 0.0,
            flags: 0,
            shadow_slot: NO_SHADOW_SLOT,
            radius: 0.0,
            _pad0: 0.0,
            layers: u32::MAX,
            shadow_layers: u32::MAX,
        }
    }
}

impl GpuLight {
    /// Marches the depth buffer for contact shadows (#735). Per light, since the march scales with
    /// light count. Mirrors `INTI_LIGHT_CONTACT_SHADOWS`.
    pub const FLAG_CONTACT_SHADOWS: u32 = 1;

    /// `FLAG_CONTACT_SHADOWS` when `enabled`, nothing otherwise.
    fn flags(enabled: bool) -> u32 {
        if enabled {
            Self::FLAG_CONTACT_SHADOWS
        } else {
            0
        }
    }

    /// Directional: direction comes from the transform, never from a
    /// field. A light that ignores its own rotation is a second source
    /// of truth, and the gizmo already draws the arrow from this one.
    pub fn directional(light: &DirectionalLight, world: Mat4) -> Self {
        Self {
            color: light.color.to_array(),
            intensity: light.intensity,
            position: [0.0; 3],
            range: 0.0,
            direction: forward(world).to_array(),
            kind: LIGHT_KIND_DIRECTIONAL,
            spot_scale: 0.0,
            spot_offset: 0.0,
            flags: Self::flags(light.contact_shadows),
            shadow_slot: NO_SHADOW_SLOT,
            // A sun's size shapes its penumbra (#477); the representative point corrects distance
            // to a nearby sphere, which a sun lacks.
            radius: 0.0,
            _pad0: 0.0,
            layers: light.layers,
            shadow_layers: light.shadow_layers,
        }
    }

    pub fn point(light: &PointLight, world: Mat4) -> Self {
        let scale = max_scale(world);
        Self {
            color: light.color.to_array(),
            intensity: light.intensity,
            position: world.w_axis.truncate().to_array(),
            range: (light.range * scale).max(0.0),
            direction: [0.0; 3],
            kind: LIGHT_KIND_POINT,
            spot_scale: 0.0,
            spot_offset: 0.0,
            flags: Self::flags(light.contact_shadows),
            shadow_slot: NO_SHADOW_SLOT,
            radius: (light.radius * scale).max(0.0),
            _pad0: 0.0,
            layers: light.layers,
            shadow_layers: light.shadow_layers,
        }
    }

    pub fn spot(light: &SpotLight, world: Mat4) -> Self {
        let (cone_scale, offset) = spot_cone_mad(light.inner_angle, light.outer_angle);
        let scale = max_scale(world);
        Self {
            color: light.color.to_array(),
            intensity: light.intensity,
            position: world.w_axis.truncate().to_array(),
            range: (light.range * scale).max(0.0),
            direction: forward(world).to_array(),
            kind: LIGHT_KIND_SPOT,
            spot_scale: cone_scale,
            spot_offset: offset,
            flags: Self::flags(light.contact_shadows),
            shadow_slot: NO_SHADOW_SLOT,
            radius: (light.radius * scale).max(0.0),
            _pad0: 0.0,
            layers: light.layers,
            shadow_layers: light.shadow_layers,
        }
    }
}

/// The entity's local −Z in world space, −Y when degenerate: a zero vector reads as night, down at
/// least looks wrong.
pub(crate) fn forward(world: Mat4) -> Vec3 {
    let f = world.transform_vector3(Vec3::NEG_Z);
    f.normalize_or(Vec3::NEG_Y)
}

/// Largest axis scale, applied to every light length as the gizmo does, so the wire sphere matches
/// the falloff; a scaled lamp has a bigger bulb. Computed once per light.
fn max_scale(world: Mat4) -> f32 {
    world.to_scale_rotation_translation().0.abs().max_element()
}

/// Inner/outer half-angles in degrees → the shader's multiply-add. Half-angles as
/// `gizmos/lights.rs` draws them (Unreal's shape): `scale = 1 / (cos_inner - cos_outer)`, `offset =
/// -cos_outer * scale`.
pub fn spot_cone_mad(inner_angle_deg: f32, outer_angle_deg: f32) -> (f32, f32) {
    // An inner wider than its outer is authorable and physically
    // nonsense; clamping keeps the cone bright inside and dark outside
    // rather than inverting it.
    let outer = outer_angle_deg.clamp(0.0, 90.0);
    let inner = inner_angle_deg.clamp(0.0, outer);
    let cos_outer = outer.to_radians().cos();
    let cos_inner = inner.to_radians().cos();
    let scale = 1.0 / (cos_inner - cos_outer).max(MIN_CONE_COS_DELTA);
    (scale, -cos_outer * scale)
}

#[cfg(test)]
mod tests;
