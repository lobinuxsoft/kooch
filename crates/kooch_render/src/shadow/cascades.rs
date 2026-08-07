//! Where each cascade goes, and how big it is.
//!
//! Pure maths against `glam` — no GPU, no wgpu, so the part of shadow
//! mapping that is easiest to get subtly wrong is also the part that is
//! fully testable.
//!
//! # The two things that stop shadows from boiling
//!
//! A cascade's orthographic projection is refit every frame as the
//! camera moves. Done naively, the projection changes shape from frame
//! to frame, every shadow texel lands somewhere slightly different, and
//! the result is a shimmer along every shadow edge that no amount of
//! filtering hides. Two properties fix it, and **both are required**:
//!
//! 1. **A bounding sphere, not a bounding box.** The AABB of a frustum
//!    slice changes size as the camera rotates — turn 45° and the box
//!    grows by up to √2 — so the world-units-per-texel ratio changes
//!    and the whole shadow resamples. A sphere around the same slice is
//!    invariant to rotation: same radius from any angle.
//! 2. **Snapping the centre to the texel grid.** Even with a fixed
//!    radius, translating the camera slides the projection by fractions
//!    of a texel. Rounding the centre to whole texels in light space
//!    means the shadow moves in whole-texel steps — visibly quantised
//!    if you look for it, and stable, which matters far more.
//!
//! Fix only the first and shadows shimmer when the camera moves. Fix
//! only the second and they shimmer when it turns. There is a test for
//! each below, because "it looks fine" is how this ships broken.

use glam::{Mat4, Vec3, Vec4Swizzles};

/// How many cascades the atlas holds. Four quadrants of one square
/// texture; changing this changes the atlas layout, not just a loop
/// bound.
pub const CASCADE_COUNT: usize = 4;

/// Overlap band between neighbouring cascades, as a fraction of the
/// split distance.
///
/// Texel density and filter width both change at a split, so a hard
/// handover is a line drawn across the ground. The shading pass samples
/// both cascades inside the band and mixes; Bevy calls the same number
/// `overlap_proportion` and ships 0.2.
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
    ///
    /// An orthographic projection has no eye, and the cull pass needs
    /// one anyway: its backface cone test and its LOD selector both
    /// measure from a point. Passing the origin instead — which is what
    /// "the light is infinitely far away, so there is no position" leads
    /// to — makes both of them measure from a place the light is not,
    /// and the backface test then rejects whichever meshlets happen to
    /// face away from the world origin. Those meshlets stop writing
    /// depth, and the result is a shadow with holes in it.
    ///
    /// Far enough back that it is a good stand-in for a direction, close
    /// enough that the LOD selector still sees a sane distance.
    pub light_eye: Vec3,
    /// World units the `[0,1]` depth range spans.
    ///
    /// The shading pass needs it to turn a difference between two
    /// stored depths back into a distance in metres, which is what
    /// PCSS's penumbra is proportional to. Without it the shader has a
    /// ratio and no scale, and the only way to use one is a magic
    /// constant that is wrong in three of the four cascades.
    pub depth_extent: f32,
}

/// Where each cascade hands over to the next.
///
/// Purely logarithmic from `first_bound` to `far`, which is Bevy 0.19's
/// `calculate_cascade_bounds` and, by their own comment, what Unity,
/// Unreal and Godot arrive at too.
///
/// # Why the first bound is a distance, not the camera's near plane
///
/// 🔴 This used to interpolate between a logarithmic and a uniform
/// distribution starting at the camera's **near plane**, and a near
/// plane is 0.1 m. A logarithmic scheme anchored there spends the first
/// cascade on the first few centimetres of a scene, so the cascade that
/// should be carrying everything the player is looking at gets a sliver
/// of the range and the rest lands in cascades with metre-wide texels.
/// The symptom is needing far more shadow-map resolution than the scene
/// deserves.
///
/// Anchoring at an authored distance decouples the split scheme from
/// the lens. Unity ships 10.05, Godot 10, and this ships 10 — a first
/// cascade covering the ten metres around the camera, whatever the near
/// plane happens to be.
pub fn split_distances(first_bound: f32, far: f32) -> [f32; CASCADE_COUNT] {
    let first = first_bound.max(1e-3);
    let far = far.max(first + 1e-3);
    // Each cascade covers the same *ratio* of distance as the last, so
    // a texel subtends roughly the same screen angle in all four. That
    // is the whole argument for a logarithmic split.
    let base = (far / first).powf(1.0 / (CASCADE_COUNT - 1) as f32);
    std::array::from_fn(|i| first * base.powi(i as i32))
}

/// The eight world-space corners of the frustum described by
/// `inverse_view_proj`, unprojected from the NDC cube.
///
/// Under reversed-Z the near plane is at `ndc.z = 1` and the far plane
/// at `0`, which is the opposite of what every reference on this
/// subject assumes.
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
///
/// Ported from Bevy 0.19's `calculate_cascade`. `light_direction` points
/// **where the light shines**, matching the component (the entity's -Z).
/// `shadow_map_size` is one cascade's side in texels, not the atlas's.
///
/// # Why the slice's own diameter, and not a bounding sphere
///
/// The volume has to be the same size from every camera angle or the
/// world-units-per-texel ratio changes as you turn and every shadow
/// resamples. A bounding sphere gives that, and it gives away
/// resolution: a sphere around the centroid of a frustum slice is
/// noticeably larger than the slice.
///
/// The **longer of the slice's two diagonals** — corner to opposite
/// corner through the body, and across the far plane — is just as
/// invariant, because those are distances between fixed corners of a
/// rigid shape, and it is the smallest square that always contains the
/// slice. `ceil` on top: an integer diameter over a power-of-two texture
/// makes the texel size an exact power of two, which is what lets the
/// snap below be exact in floating point rather than approximately
/// exact.
///
/// # Why the depth range hugs the slice
///
/// The near plane sits at the slice's own nearest point to the light
/// plus `near_extension`, rather than a multiple of the volume's size.
/// Every metre of unused depth range is precision the comparison does
/// not get, and it is why the bias had to be as large as it was.
///
/// `near_extension` is what still catches occluders outside the view
/// frustum — a wall behind the camera shadowing the floor in front of
/// it. Bevy avoids needing it by rendering the shadow pass with
/// `unclipped_depth`; the same trick applies here the day this pipeline
/// asks for `DEPTH_CLIP_CONTROL`.
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
        // Interpolate the whole frustum's corners along each edge to get
        // this slice's, rather than rebuilding a projection per slice:
        // one inverse instead of four, and the edges are lines so the
        // interpolation is exact.
        let t_near = ((slice_near - near) / (far - near)).clamp(0.0, 1.0);
        let t_far = ((slice_far - near) / (far - near)).clamp(0.0, 1.0);
        let mut slice = [Vec3::ZERO; 8];
        for corner in 0..4 {
            let n = whole[corner];
            let f = whole[corner + 4];
            slice[corner] = n.lerp(f, t_near);
            slice[corner + 4] = n.lerp(f, t_far);
        }

        // Measured on the world-space corners, not the light-space ones.
        // The lengths are the same in exact arithmetic and are not in
        // f32, and this is the value the whole cascade's stability rests
        // on.
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

        // Snap the centre to the texel grid, in light space, where that
        // grid is axis-aligned. Rounding in world space would be
        // rounding against the wrong grid and would stabilise nothing.
        //
        // 🔴 max.z, not the centre: in a right-handed light space the
        // largest z is the point NEAREST the light, and that is where
        // the near plane belongs.
        let near_extension = diameter * near_extension_scale.max(0.0);
        let centre_light = Vec3::new(
            (0.5 * (min.x + max.x) / texel_world_size).floor() * texel_world_size,
            (0.5 * (min.y + max.y) / texel_world_size).floor() * texel_world_size,
            max.z + near_extension,
        );
        let depth_extent = (max.z - min.z + near_extension).max(1e-3);

        // Form clip-from-world directly rather than inverting a
        // world-from-cascade. The inverse of a matrix built from a
        // rotation and a translation is exactly expressible, and asking
        // the general inverse for it is asking for a different answer
        // every frame in the low bits — which is a shimmer no snapping
        // can undo.
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
        slice_near = slice_far;
    }
    cascades
}

/// Right-handed orthographic projection with the depth range reversed:
/// near maps to 1, far to 0.
///
/// Reversed-Z buys nothing in an orthographic projection — depth is
/// linear either way, so there is no floating-point precision to
/// recover. It is here for consistency: the shadow pipeline can then use
/// the same `CompareFunction::Greater` as everything else, and a reader
/// does not have to work out which convention this particular target is
/// in.
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
mod tests {
    use super::*;
    use crate::perspective_rh_reverse_z;

    fn camera(pitch: f32, yaw: f32, position: Vec3) -> Mat4 {
        let rotation = glam::Quat::from_euler(glam::EulerRot::YXZ, yaw, pitch, 0.0);
        let to_world = Mat4::from_rotation_translation(rotation, position);
        perspective_rh_reverse_z(60.0_f32.to_radians(), 1.6, 0.1, 500.0) * to_world.inverse()
    }

    #[test]
    fn splits_cover_the_range_and_increase() {
        let splits = split_distances(10.0, 1000.0);
        assert!(
            splits.windows(2).all(|w| w[0] < w[1]),
            "splits must increase: {splits:?}",
        );
        assert!(
            (splits[CASCADE_COUNT - 1] - 1000.0).abs() < 1.0,
            "the last split is the far plane, got {}",
            splits[CASCADE_COUNT - 1],
        );
    }

    /// The first cascade covers exactly what it was asked to, rather
    /// than a share of the range derived from the lens. That is the
    /// difference between a scheme an author can reason about and one
    /// that changes when someone edits the camera's near plane.
    #[test]
    fn the_first_cascade_ends_where_it_was_told_to() {
        let splits = split_distances(10.0, 1000.0);
        assert!(
            (splits[0] - 10.0).abs() < 1e-3,
            "first split at {}",
            splits[0]
        );
    }

    /// Each cascade covers the same ratio of distance as the last, which
    /// is what makes a texel subtend roughly one screen angle in all
    /// four rather than four different ones.
    #[test]
    fn every_cascade_covers_the_same_ratio_as_the_last() {
        let splits = split_distances(10.0, 1000.0);
        let ratio = splits[1] / splits[0];
        for w in splits.windows(2) {
            assert!(
                (w[1] / w[0] - ratio).abs() < 1e-3,
                "ratios differ across the chain: {splits:?}",
            );
        }
    }

    /// 🔴 Property one. Rotating the camera in place must not change how
    /// much world a shadow texel covers. If it does, every shadow edge
    /// crawls as you look around, and no filter hides it.
    #[test]
    fn texel_size_is_invariant_to_camera_rotation() {
        let light = Vec3::new(-0.3, -1.0, -0.2);
        let at = Vec3::new(5.0, 2.0, -3.0);
        let mut sizes = Vec::new();
        for yaw_deg in [0.0f32, 17.0, 45.0, 90.0, 133.0, 180.0, 271.0] {
            let c = camera(0.0, yaw_deg.to_radians(), at);
            let cascades = build_cascades(c, light, 0.1, 500.0, 10.0, 2048, 4.0);
            sizes.push(cascades[0].texel_world_size);
        }
        let min = sizes.iter().copied().fold(f32::INFINITY, f32::min);
        let max = sizes.iter().copied().fold(0.0f32, f32::max);
        assert!(
            (max - min) / max < 1e-3,
            "texel size varies with yaw: {sizes:?} — the slice is being \
             bounded by a box, not a sphere",
        );
    }

    /// Same for pitch, which is the axis a bounding box distorts worst.
    #[test]
    fn texel_size_is_invariant_to_camera_pitch() {
        let light = Vec3::NEG_Y;
        let mut sizes = Vec::new();
        for pitch_deg in [-80.0f32, -40.0, 0.0, 40.0, 80.0] {
            let c = camera(pitch_deg.to_radians(), 0.0, Vec3::ZERO);
            sizes.push(build_cascades(c, light, 0.1, 500.0, 10.0, 2048, 4.0)[1].texel_world_size);
        }
        let min = sizes.iter().copied().fold(f32::INFINITY, f32::min);
        let max = sizes.iter().copied().fold(0.0f32, f32::max);
        assert!(
            (max - min) / max < 1e-3,
            "texel size varies with pitch: {sizes:?}"
        );
    }

    /// 🔴 Property two. Translating the camera by less than a texel must
    /// not move the shadow projection at all — it snaps, or it swims.
    #[test]
    fn sub_texel_camera_movement_does_not_move_the_projection() {
        let light = Vec3::new(0.0, -1.0, -0.35);
        let base = camera(0.0, 0.0, Vec3::ZERO);
        let cascades = build_cascades(base, light, 0.1, 500.0, 10.0, 2048, 4.0);
        let texel = cascades[0].texel_world_size;

        // A hundredth of a texel: far below the quantisation step.
        let nudged = camera(0.0, 0.0, Vec3::new(texel * 0.01, 0.0, 0.0));
        let moved = build_cascades(nudged, light, 0.1, 500.0, 10.0, 2048, 4.0);

        let a = cascades[0].view_proj.to_cols_array();
        let b = moved[0].view_proj.to_cols_array();
        let drift = a
            .iter()
            .zip(b.iter())
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max);
        assert!(
            drift < 1e-4,
            "the projection moved by {drift} for a sub-texel camera nudge — \
             the centre is not snapped to the texel grid",
        );
    }

    /// And a movement of many texels must move it, or the "snap" is a
    /// freeze and the cascade stops following the camera.
    #[test]
    fn large_camera_movement_does_move_the_projection() {
        let light = Vec3::new(0.0, -1.0, -0.35);
        let base = camera(0.0, 0.0, Vec3::ZERO);
        let cascades = build_cascades(base, light, 0.1, 500.0, 10.0, 2048, 4.0);
        let far = camera(
            0.0,
            0.0,
            Vec3::new(cascades[0].texel_world_size * 500.0, 0.0, 0.0),
        );
        let moved = build_cascades(far, light, 0.1, 500.0, 10.0, 2048, 4.0);
        assert_ne!(cascades[0].view_proj, moved[0].view_proj);
    }

    #[test]
    fn cascades_are_ordered_near_to_far_and_grow() {
        let cascades = build_cascades(
            camera(0.0, 0.0, Vec3::ZERO),
            Vec3::NEG_Y,
            0.1,
            500.0,
            10.0,
            2048,
            4.0,
        );
        for pair in cascades.windows(2) {
            assert!(pair[0].far_depth < pair[1].far_depth);
            assert!(
                pair[0].texel_world_size < pair[1].texel_world_size,
                "a farther cascade must cover more world per texel, or the \
                 split scheme is upside down",
            );
        }
    }

    /// A light pointing straight down is the case that picks a degenerate
    /// up vector, and it is also the most common light in any scene.
    #[test]
    fn a_straight_down_light_produces_finite_matrices() {
        let cascades = build_cascades(
            camera(0.0, 0.0, Vec3::ZERO),
            Vec3::NEG_Y,
            0.1,
            500.0,
            10.0,
            2048,
            4.0,
        );
        for (i, c) in cascades.iter().enumerate() {
            assert!(
                c.view_proj.to_cols_array().iter().all(|v| v.is_finite()),
                "cascade {i} has a non-finite matrix",
            );
        }
    }

    #[test]
    fn a_degenerate_light_direction_falls_back_rather_than_producing_nan() {
        let cascades = build_cascades(
            camera(0.0, 0.0, Vec3::ZERO),
            Vec3::ZERO,
            0.1,
            500.0,
            10.0,
            2048,
            4.0,
        );
        assert!(
            cascades[0]
                .view_proj
                .to_cols_array()
                .iter()
                .all(|v| v.is_finite())
        );
    }

    /// Reversed-Z: a point at the light's near plane must land at ndc.z
    /// = 1 and one at the far plane at 0. Getting this backwards fills
    /// the atlas with the farthest surface instead of the nearest, and
    /// every shadow inverts.
    #[test]
    fn orthographic_is_reversed_z() {
        let p = orthographic_rh_reverse_z(-1.0, 1.0, -1.0, 1.0, 0.0, 100.0);
        // Right-handed: the camera looks down -Z, so the near plane is
        // at z = 0 and the far plane at z = -100.
        let near = p * glam::Vec4::new(0.0, 0.0, 0.0, 1.0);
        let far = p * glam::Vec4::new(0.0, 0.0, -100.0, 1.0);
        assert!(
            (near.z / near.w - 1.0).abs() < 1e-5,
            "near mapped to {}",
            near.z / near.w
        );
        assert!(
            (far.z / far.w).abs() < 1e-5,
            "far mapped to {}",
            far.z / far.w
        );
    }
}
