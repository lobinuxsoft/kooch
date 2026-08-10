//! Fitting six shadow faces to one point light (#778).
//!
//! A spot light *is* a frustum, so its shadow view is the cone itself. A
//! point light is not a frustum at all — it lights every direction — so
//! it gets six 90° frusta that tile the sphere, and the shading model
//! picks one by the direction to the fragment rather than by a matrix.
//!
//! That is what makes the record so small: there is no `view_proj` to
//! send. See [`GpuPointShadow`].

use glam::{Mat4, Vec3};

use kooch_lighting::{GpuPointShadow, PointShadowSource};

/// Near plane for every cube face, in metres. Bevy's
/// `PointLight::DEFAULT_SHADOW_MAP_NEAR_Z`, and the same value the spots
/// use — a light's near plane is about how close geometry may get to the
/// bulb, which does not depend on the shape of the light.
pub const POINT_SHADOW_NEAR_Z: f32 = 0.1;

/// How many faces a cube map has. Named because `6` appears in the
/// layer arithmetic, the memory budget and the draw count, and only one
/// of those three is obviously the same six.
pub const CUBE_FACES: usize = 6;

/// The six directions, in the layer order a cube-array texture expects:
/// +X, −X, +Y, −Y, +Z, −Z.
///
/// 🔴 Ported verbatim from Bevy's `CUBE_MAP_FACES`
/// (`bevy_camera/src/primitives.rs:347`), including the two that look
/// wrong: face 4 is labelled +Z and targets `NEG_Z`, face 5 is labelled
/// −Z and targets `Z`. **Cube maps are left-handed and the rest of the
/// engine is not**, so the Z faces are swapped here and the sampling
/// direction is mirrored on Z at the other end (`flip_z` in the shader).
/// Fixing either half alone puts the shadow of everything in front of a
/// lamp behind it.
///
/// The `up` vectors are Bevy's too, not the D3D reference table's — they
/// are the ones that agree with the mirror above.
pub const FACE_DIRECTIONS: [(Vec3, Vec3); CUBE_FACES] = [
    (Vec3::X, Vec3::Y),
    (Vec3::NEG_X, Vec3::Y),
    (Vec3::Y, Vec3::Z),
    (Vec3::NEG_Y, Vec3::NEG_Z),
    (Vec3::NEG_Z, Vec3::Y),
    (Vec3::Z, Vec3::Y),
];

/// One point light's shadow, as the pass needs it: where the light is,
/// and the six matrices its faces draw with.
#[derive(Copy, Clone, Debug)]
pub struct PointShadowDraw {
    /// The light's position — a real eye, six times over.
    pub eye: Vec3,
    /// Clip-from-world per face, in cube-array layer order.
    pub faces: [Mat4; CUBE_FACES],
}

impl PointShadowDraw {
    pub fn new(position: Vec3) -> Self {
        Self {
            eye: position,
            faces: std::array::from_fn(|face| face_view_proj(position, face, POINT_SHADOW_NEAR_Z)),
        }
    }
}

/// The record the shading model reads for one point light.
///
/// `size` is the side of one face in texels.
pub fn point_shadow(source: &PointShadowSource, size: u32) -> GpuPointShadow {
    GpuPointShadow {
        near: POINT_SHADOW_NEAR_Z,
        texel_world_size: face_texel_size(size),
        depth_extent: source.range.max(POINT_SHADOW_NEAR_Z * 2.0),
        _pad0: 0.0,
    }
}

/// Clip-from-world for one face of one light.
///
/// The same infinite reverse-Z projection every other view in the engine
/// uses (ADR 0002), which is what lets the shader reconstruct the stored
/// depth as `near / major_axis_magnitude` instead of carrying the four
/// projection terms Bevy sends per light.
///
/// A cube face is 90° by construction — six of them tile the sphere
/// exactly — so the field of view is not a parameter and neither is the
/// aspect ratio.
pub fn face_view_proj(position: Vec3, face: usize, near: f32) -> Mat4 {
    let (target, up) = FACE_DIRECTIONS[face.min(CUBE_FACES - 1)];
    let projection = crate::projection::perspective_infinite_rh_reverse_z(
        std::f32::consts::FRAC_PI_2,
        1.0,
        near.max(1e-4),
    );
    projection * Mat4::look_to_rh(position, target, up)
}

/// Shadow-texel size **per metre of distance from the light**.
///
/// A face spans 90°, so Bevy's `2 * tan(half_fov) / size` has
/// `tan(45°) = 1` and collapses to `2 / size`. Like the spot's, it is an
/// angle per texel and never involves `range` — the shader multiplies it
/// by the fragment's own distance. Baking the range in is what produced
/// #777's peter-panning, and a cube face would produce it six times.
///
/// The `SQRT_2` is Bevy's, for the worst-case diagonal offset.
fn face_texel_size(size: u32) -> f32 {
    2.0 / size.max(1) as f32 * std::f32::consts::SQRT_2
}

#[cfg(test)]
mod tests;
