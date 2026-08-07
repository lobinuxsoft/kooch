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

/// One light, as the shader reads it. 64 bytes, `std430`-compatible.
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
    pub _pad0: f32,
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
            _pad0: 0.0,
        }
    }

    pub fn point(light: &PointLight, world: Mat4) -> Self {
        Self {
            color: light.color.to_array(),
            intensity: light.intensity,
            position: world.w_axis.truncate().to_array(),
            range: scaled_range(light.range, world),
            direction: [0.0; 3],
            kind: LIGHT_KIND_POINT,
            spot_scale: 0.0,
            spot_offset: 0.0,
            flags: Self::flags(light.contact_shadows),
            _pad0: 0.0,
        }
    }

    pub fn spot(light: &SpotLight, world: Mat4) -> Self {
        let (scale, offset) = spot_cone_mad(light.inner_angle, light.outer_angle);
        Self {
            color: light.color.to_array(),
            intensity: light.intensity,
            position: world.w_axis.truncate().to_array(),
            range: scaled_range(light.range, world),
            direction: forward(world).to_array(),
            kind: LIGHT_KIND_SPOT,
            spot_scale: scale,
            spot_offset: offset,
            flags: Self::flags(light.contact_shadows),
            _pad0: 0.0,
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

/// Range in world units, scaled by the entity's transform.
///
/// Matches what the editor's light gizmo draws (`light.range *
/// scale.abs().max_element()`) — the wire sphere is meant to show
/// exactly where the falloff reaches zero, and it can only do that if
/// both sides scale the same way.
fn scaled_range(range: f32, world: Mat4) -> f32 {
    let scale = world.to_scale_rotation_translation().0.abs().max_element();
    (range * scale).max(0.0)
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
mod tests {
    use super::*;
    use glam::{Quat, Vec3};

    #[test]
    fn size_matches_shader() {
        // `inti_pbr.wgsl`'s IntiLight: vec3+f32, vec3+f32, vec3+u32,
        // f32, f32, f32, f32 — 64 B under std430's vec3-aligns-to-16
        // rule. A mismatch here is the whole struct read at the wrong
        // stride.
        assert_eq!(std::mem::size_of::<GpuLight>(), 64);
        assert_eq!(std::mem::align_of::<GpuLight>(), 4);
    }

    #[test]
    fn directional_takes_direction_from_the_transform() {
        let world = Mat4::from_quat(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2));
        let light = DirectionalLight::default();
        let gpu = GpuLight::directional(&light, world);
        // -Z rotated -90° about X points straight down.
        let dir = Vec3::from(gpu.direction);
        assert!(
            dir.abs_diff_eq(Vec3::NEG_Y, 1e-5),
            "expected -Y, got {dir:?}",
        );
    }

    #[test]
    fn degenerate_transform_does_not_produce_nan() {
        let gpu = GpuLight::directional(&DirectionalLight::default(), Mat4::ZERO);
        assert!(Vec3::from(gpu.direction).is_finite());
    }

    #[test]
    fn point_range_scales_with_the_transform_like_the_gizmo_does() {
        let world = Mat4::from_scale(Vec3::splat(2.0));
        let mut light = PointLight::default();
        light.range = 10.0;
        assert_eq!(GpuLight::point(&light, world).range, 20.0);
    }

    #[test]
    fn spot_mad_is_one_inside_the_inner_cone_and_zero_outside_the_outer() {
        let (scale, offset) = spot_cone_mad(30.0, 45.0);
        let at = |deg: f32| (deg.to_radians().cos() * scale + offset).clamp(0.0, 1.0);
        assert!(
            (at(0.0) - 1.0).abs() < 1e-5,
            "axis should be full intensity"
        );
        assert!((at(30.0) - 1.0).abs() < 1e-5, "inner edge should be full");
        assert!(at(45.0).abs() < 1e-5, "outer edge should be dark");
        let mid = at(37.5);
        assert!(mid > 0.0 && mid < 1.0, "penumbra should ramp, got {mid}");
    }

    #[test]
    fn inner_wider_than_outer_does_not_invert_the_cone() {
        let (scale, offset) = spot_cone_mad(60.0, 20.0);
        let at = |deg: f32| (deg.to_radians().cos() * scale + offset).clamp(0.0, 1.0);
        assert!((at(0.0) - 1.0).abs() < 1e-5);
        assert!(at(21.0).abs() < 1e-5);
        assert!(scale.is_finite() && offset.is_finite());
    }

    #[test]
    fn coincident_angles_do_not_divide_by_zero() {
        let (scale, offset) = spot_cone_mad(45.0, 45.0);
        assert!(scale.is_finite(), "scale was {scale}");
        assert!(offset.is_finite(), "offset was {offset}");
    }
}
