//! The GPU-side light record, and the conversion from a light
//! component + its transform into one.
//!
//! # Why AoS and not SoA
//!
//! The engine's rule is data-oriented layout, and this is the case
//! where AoS wins on its own terms: every shader invocation that
//! touches light `i` reads *all* of light `i`'s fields in the same few
//! instructions. Splitting into parallel arrays would turn one 64-byte
//! cache line into six scattered fetches. SoA pays when a pass reads
//! one field across many records — which is what light **culling** does
//! (positions and ranges only), so the day clustering lands, its input
//! is a separate positions/ranges pair, not a reinterpretation of this.

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

/// One light, as the shader reads it. 80 bytes, `std430`-compatible.
///
/// Mirrors `IntiLight` in `inti_pbr.wgsl` byte for byte. Nothing checks
/// that correspondence at compile time on either side of the boundary —
/// a reordered field reads a light's range as its intensity and renders
/// something plausible-looking and wrong. [`Self::size_matches_shader`]
/// is the test that stands in for the missing compiler.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuLight {
    /// Linear RGB.
    pub color: [f32; 3],
    /// Photometric, straight off the component: lux for directional,
    /// lumens for punctual. Kept unconverted so the number in the
    /// Inspector and the number in the buffer are the same number when
    /// someone is staring at a capture wondering why a light is dark.
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
    /// Radius of the emitting sphere, in world units, scaled by the
    /// entity's transform the way `range` is. `0` is a point.
    ///
    /// This is the field that took the record from 64 B to 80 B (#776).
    /// Bevy grew theirs for the same reason and carries it as the `.w`
    /// of `position_radius`; here `position` already spent its `.w` on
    /// `range`, so there was nowhere to hide it.
    pub radius: f32,
    /// Padding to the 16-byte alignment `std430` gives any struct
    /// containing a `vec3`.
    ///
    /// 🔴 **Not free space to raid casually, but not dead either.** The
    /// shader reads the whole record per light per fragment, so these
    /// twelve bytes are already being fetched: a field that fits here
    /// costs nothing further, and the next one after it costs another
    /// sixteen for every light in the scene. #778's cube-map slot is
    /// the intended tenant. A rect light is NOT (#779) — that one is a
    /// second array, the way Bevy keeps `GpuRectLight` separate.
    ///
    /// ⚠️ Three scalars, never a `[f32; 3]` standing in for a `vec3`:
    /// the mirror in `inti_pbr.wgsl` must declare three separate `f32`
    /// or WGSL realigns and the strides diverge.
    pub _pad0: f32,
    pub _pad1: f32,
    pub _pad2: f32,
}

impl GpuLight {
    /// This light marches the depth buffer for contact shadows (#735).
    ///
    /// Opt-in per light rather than global because the march is the one
    /// shadow cost that scales with *light count* rather than with
    /// geometry: fifty lights in a room would be fifty screen-space
    /// marches on every pixel they touch. Mirrors
    /// `INTI_LIGHT_CONTACT_SHADOWS` in `inti_pbr.wgsl`.
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
            // A star has a size, and it is why its shadows have a
            // penumbra — but that is a shadow technique (#477), not
            // this one. The representative point below is a correction
            // to the distance to a nearby sphere; there is no distance
            // to a light that has no position.
            radius: 0.0,
            _pad0: 0.0,
            _pad1: 0.0,
            _pad2: 0.0,
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
            _pad1: 0.0,
            _pad2: 0.0,
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
            _pad1: 0.0,
            _pad2: 0.0,
        }
    }
}

/// The entity's forward direction: its local -Z in world space.
///
/// Falls back to -Y for a degenerate transform. A zero vector would
/// make every `dot` against it zero, which for a directional light is
/// indistinguishable from night; pointing straight down at least
/// renders something an author can see is wrong.
pub(crate) fn forward(world: Mat4) -> Vec3 {
    let f = world.transform_vector3(Vec3::NEG_Z);
    f.normalize_or(Vec3::NEG_Y)
}

/// The entity's largest axis scale, which every world-space length on a
/// light is multiplied by.
///
/// Matches what the editor's light gizmo draws (`light.range *
/// scale.abs().max_element()`) — the wire sphere is meant to show
/// exactly where the falloff reaches zero, and it can only do that if
/// both sides scale the same way. `radius` follows the same rule: a
/// lamp scaled up has a bigger bulb, not the same bulb further away.
///
/// Computed once per light rather than once per length: decomposing a
/// matrix is not what anyone wants running twice per light per frame.
fn max_scale(world: Mat4) -> f32 {
    world.to_scale_rotation_translation().0.abs().max_element()
}

/// Packs inner/outer **half-angles in degrees** into the multiply-add
/// the shader evaluates per fragment.
///
/// Half-angles measured from the axis to the edge, which is what
/// `gizmos/lights.rs` chose when it drew the cone (Unreal's shape, not
/// Unity's single full `spotAngle`). That file called out that the
/// lighting work would either honour the convention or draw a cone half
/// the width it lights. This is it honouring it.
///
/// Solving `saturate(cos_a * scale + offset)` for 0 at `outer` and 1 at
/// `inner` gives `scale = 1 / (cos_inner - cos_outer)` and
/// `offset = -cos_outer * scale`.
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
