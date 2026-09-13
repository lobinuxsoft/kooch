//! Where each cascade goes, and how big it is.

use glam::{Mat4, Vec3, Vec4Swizzles};

/// How many cascades the atlas holds. Four quadrants of one square
/// texture; changing this changes the atlas layout, not just a loop
/// bound.
pub const CASCADE_COUNT: usize = 4;

/// Overlap band between neighbouring cascades, as a fraction of the split distance.
pub const CASCADE_BLEND_FRACTION: f32 = 0.2;

/// One cascade's placement.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Cascade {
    /// Light-space clip-from-world for this cascade. What the shadow
    /// pass renders with and what the shading pass samples with.
    pub view_proj: Mat4,
    /// View-space depth at which this cascade stops being the one to
    /// sample. Compared against the fragment's own view depth.
    pub far_depth: f32,
    /// World units covered per shadow texel. Feeds the depth bias —
    /// a bias in texels is a different distance in each cascade — and
    /// PCSS's penumbra estimate.
    pub texel_world_size: f32,
    /// Where the light looks from, for this cascade.
    pub light_eye: Vec3,
    /// World units the `[0,1]` depth range spans.
    pub depth_extent: f32,
}

/// Where each cascade hands over to the next.
pub fn split_distances(first_bound: f32, far: f32) -> [f32; CASCADE_COUNT] {
    let first = first_bound.max(1e-3);
    let far = far.max(first + 1e-3);
    // Each cascade covers the same *ratio* of distance as the last, so
    // a texel subtends roughly the same screen angle in all four. That
    // is the whole argument for a logarithmic split.
    let base = (far / first).powf(1.0 / (CASCADE_COUNT - 1) as f32);
    std::array::from_fn(|i| first * base.powi(i as i32))
}

/// The eight world-space corners of the frustum described by `inverse_view_proj`, unprojected from
/// the NDC cube.
pub fn frustum_corners(inverse_view_proj: Mat4) -> [Vec3; 8] {
    let mut corners = [Vec3::ZERO; 8];
    let mut i = 0;
    for z in [1.0f32, 0.0] {
        for y in [-1.0f32, 1.0] {
            for x in [-1.0f32, 1.0] {
                let p = inverse_view_proj * glam::Vec4::new(x, y, z, 1.0);
                // A degenerate matrix produces w = 0; leaving the corner
                // at the origin keeps the sphere finite rather than
                // propagating NaN into every cascade.
                corners[i] = if p.w.abs() < 1e-6 {
                    Vec3::ZERO
                } else {
                    p.xyz() / p.w
                };
                i += 1;
            }
        }
    }
    corners
}

/// Builds the cascades for one directional light and one camera.
#[allow(clippy::too_many_arguments)]
pub fn build_cascades(
    camera_view_proj: Mat4,
    light_direction: Vec3,
    near: f32,
    far: f32,
    first_cascade_distance: f32,
    shadow_map_size: u32,
    near_extension_scale: f32,
) -> [Cascade; CASCADE_COUNT] {
    let splits = split_distances(first_cascade_distance, far);
    let inverse = camera_view_proj.inverse();
    let whole = frustum_corners(inverse);
    let size = shadow_map_size.max(1) as f32;
    let direction = light_direction.normalize_or(Vec3::NEG_Y);

    // A pure rotation with -Z down the light. Built once: the light does
    // not move between cascades, and only the centre does.
    let up = if direction.y.abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let world_from_light = Mat4::look_to_rh(Vec3::ZERO, direction, up).inverse();
    let light_from_world = world_from_light.transpose();

    let mut cascades = [Cascade {
        view_proj: Mat4::IDENTITY,
        far_depth: 0.0,
        texel_world_size: 0.0,
        depth_extent: 0.0,
        light_eye: Vec3::ZERO,
    }; CASCADE_COUNT];

    let mut slice_near = near;
    for (i, cascade) in cascades.iter_mut().enumerate() {
        let slice_far = splits[i];
        // Interpolate the whole frustum's corners along each edge to get this slice's, rather than
        // rebuilding a projection per slice: one inverse instead of four, and the edges are lines
        // so the interpolation is exact.
        let t_near = ((slice_near - near) / (far - near)).clamp(0.0, 1.0);
        let t_far = ((slice_far - near) / (far - near)).clamp(0.0, 1.0);
        let mut slice = [Vec3::ZERO; 8];
        for corner in 0..4 {
            let n = whole[corner];
            let f = whole[corner + 4];
            slice[corner] = n.lerp(f, t_near);
            slice[corner + 4] = n.lerp(f, t_far);
        }

        // Measured on the world-space corners, not the light-space ones. The lengths are the same
        // in exact arithmetic and are not in f32, and this is the value the whole cascade's
        // stability rests on.
        let body_diagonal = (slice[0] - slice[7]).length();
        let far_diagonal = (slice[4] - slice[7]).length();
        let diameter = body_diagonal.max(far_diagonal).max(1e-3).ceil();
        let texel_world_size = diameter / size;

        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        for corner in slice {
            let light_space = light_from_world.transform_point3(corner);
            min = min.min(light_space);
            max = max.max(light_space);
        }

        // Snap the centre to the texel grid, in light space, where that grid is axis-aligned.
        // Rounding in world space would be rounding against the wrong grid and would stabilise
        // nothing.
        let near_extension = diameter * near_extension_scale.max(0.0);
        let centre_light = Vec3::new(
            (0.5 * (min.x + max.x) / texel_world_size).floor() * texel_world_size,
            (0.5 * (min.y + max.y) / texel_world_size).floor() * texel_world_size,
            max.z + near_extension,
        );
        let depth_extent = (max.z - min.z + near_extension).max(1e-3);

        // Form clip-from-world directly rather than inverting a world-from-cascade.
        let light_from_world_centred = Mat4::from_translation(-centre_light) * light_from_world;

        // Right-handed orthographic, reversed-Z, centred on the near
        // plane: z runs from 0 at the near plane to -depth_extent at the
        // far one, and maps to 1 and 0.
        let r = 1.0 / depth_extent;
        let clip_from_light = Mat4::from_cols(
            glam::Vec4::new(2.0 / diameter, 0.0, 0.0, 0.0),
            glam::Vec4::new(0.0, 2.0 / diameter, 0.0, 0.0),
            glam::Vec4::new(0.0, 0.0, r, 0.0),
            glam::Vec4::new(0.0, 0.0, 1.0, 1.0),
        );

        *cascade = Cascade {
            view_proj: clip_from_light * light_from_world_centred,
            far_depth: slice_far,
            texel_world_size,
            depth_extent,
            // Far enough back to stand in for a direction in the cull's
            // backface cone test, which measures from a point whatever
            // the projection is.
            light_eye: world_from_light.transform_point3(centre_light) - direction * depth_extent,
        };
        // 🔴 The next slice starts INSIDE this one.
        slice_near = slice_far * (1.0 - CASCADE_BLEND_FRACTION);
    }
    cascades
}

/// Right-handed orthographic projection with the depth range reversed: near maps to 1, far to 0.
pub fn orthographic_rh_reverse_z(
    left: f32,
    right: f32,
    bottom: f32,
    top: f32,
    near: f32,
    far: f32,
) -> Mat4 {
    let depth_flip = Mat4::from_cols(
        glam::Vec4::new(1.0, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, 1.0, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, -1.0, 0.0),
        glam::Vec4::new(0.0, 0.0, 1.0, 1.0),
    );
    depth_flip * Mat4::orthographic_rh(left, right, bottom, top, near, far)
}

#[cfg(test)]
mod tests;
