//! Fitting a shadow map to one spot light (#777).
//!
//! A cascade is an orthographic slice of the camera's frustum and needs
//! fitting, splitting and stabilising. A spot needs none of that: the
//! light *is* a frustum, so its shadow view is the light's own cone and
//! there is one map with nothing to blend into.
//!
//! What it does need, and a cascade does not, is a perspective
//! projection — which is why `inti_shadow_coords` divides by `w`.

use glam::{Mat4, Vec3};

use kooch_lighting::{GpuCascade, SpotShadowSource};

/// Near plane for every spot's shadow frustum, in metres.
///
/// Bevy's `SpotLight::DEFAULT_SHADOW_MAP_NEAR_Z`. Theirs is per light;
/// one constant here until somebody has a light that needs its own,
/// because a near plane is the kind of control that produces bug reports
/// when it is exposed and nothing when it is not.
pub const SPOT_SHADOW_NEAR_Z: f32 = 0.1;

/// Widest half-angle a spot's shadow frustum is fitted to.
///
/// A cone approaching 90° projects to an infinite frustum, and the
/// `tan` in the projection runs away well before it gets there. Bevy
/// clamps its spot cone the same way when it builds the light's
/// projection.
const MAX_HALF_ANGLE: f32 = 1.5; // ~86°

/// One spot light's shadow, as the pass needs it.
#[derive(Copy, Clone, Debug)]
pub struct SpotShadowDraw {
    /// What the shading model samples.
    pub record: GpuCascade,
    /// The light's position — a real eye, unlike a cascade's, which is
    /// a stand-in for a projection that has none.
    pub eye: Vec3,
}

/// The record the shading model samples for one spot light.
///
/// `layer` is where the pass renders it — see
/// [`ShadowAtlas::spot_layer`](super::atlas::ShadowAtlas::spot_layer).
pub fn spot_shadow(source: &SpotShadowSource, layer: u32, layer_texels: u32) -> GpuCascade {
    let half_angle = source.outer_angle.clamp(1e-3, MAX_HALF_ANGLE);
    let range = source.range.max(SPOT_SHADOW_NEAR_Z * 2.0);

    // The full cone, not the half: a perspective projection takes the
    // vertical field of view, and fitting the half-angle would clip
    // everything outside the middle of the light's own pool.
    let fov_y = half_angle * 2.0;
    let projection =
        crate::projection::perspective_infinite_rh_reverse_z(fov_y, 1.0, SPOT_SHADOW_NEAR_Z);
    let view = spot_view(source.position, source.direction);

    GpuCascade {
        view_proj: (projection * view).to_cols_array_2d(),
        layer,
        _pad_layer: [0; 3],
        // No split to hand over at: a spot has one map. The shading
        // model only reads this to choose between cascades, and it never
        // chooses between spots.
        far_depth: 0.0,
        // Per metre of distance, not metres — `inti_spot_shadow`
        // multiplies it by the fragment's own distance to the light.
        texel_world_size: spot_texel_size_per_metre(half_angle, layer_texels),
        depth_extent: range,
        _pad0: 0.0,
    }
}

/// Light-space from world for a spot.
///
/// The up vector is chosen against the cone direction rather than fixed
/// to world up: a light pointing straight down would otherwise have a
/// degenerate basis, which is the single most common way to author a
/// spot light.
fn spot_view(position: Vec3, direction: Vec3) -> Mat4 {
    let forward = direction.normalize_or(Vec3::NEG_Z);
    let up = if forward.y.abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    Mat4::look_to_rh(position, forward, up)
}

/// Shadow-texel size **per metre of distance from the light**, which is
/// what a perspective map has instead of a constant.
///
/// 🔴 This is Bevy's `texel_size` and it deliberately does NOT involve
/// `range`:
///
/// ```text
/// texel_size = 2 * tan(outer_angle) / shadow_map_size
/// ```
///
/// It is an angle per texel, so multiplying it by a fragment's own
/// distance to the light gives that fragment's texel in metres. The
/// shader does that multiply per pixel — see `inti_spot_shadow`.
///
/// Baking `range` in here instead is what the first version did, and it
/// biases every fragment as though it sat at the far end of the cone. A
/// 100 m spot lighting objects five metres away then offsets them by
/// 17 cm along their normals, which shrinks the shadow and lifts it off
/// the object: textbook peter-panning, found in the first smoke.
///
/// The `SQRT_2` is Bevy's too, for the worst-case diagonal offset.
fn spot_texel_size_per_metre(half_angle: f32, layer_texels: u32) -> f32 {
    let texels = layer_texels.max(1) as f32;
    2.0 * half_angle.tan() / texels * std::f32::consts::SQRT_2
}

#[cfg(test)]
mod tests;
