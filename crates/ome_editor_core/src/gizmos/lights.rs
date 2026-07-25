//! Light visualizers — where a light points and how far it reaches.
//!
//! `range` and the cone angles are numbers in the Inspector with nothing
//! to check them against, so a light that falls short of the geometry it
//! was meant to hit looks like a shading bug.
//!
//! # White, not the light's colour
//!
//! Tempting to tint each outline by `LightN.color`, and wrong: a dim or
//! near-black light would draw an outline nobody can see, exactly when
//! they need to find out where it is. Gizmos are instruments, and an
//! instrument that goes dark with what it measures is not one.

use glam::{Mat3, Vec3, Vec4};

use ome_ecs::directional_light::DirectionalLight;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::point_light::PointLight;
use ome_ecs::spot_light::SpotLight;
use ome_gizmos::{Gizmos, Visualizer};

/// Outline colour for every light.
const WHITE: Vec3 = Vec3::ONE;

/// Length of the directional arrow, in world units.
///
/// Fixed rather than derived: a directional light has no range — it is
/// infinitely far away — so the arrow states a direction, not a distance,
/// and scaling it by anything would imply a reach it does not have.
const DIRECTION_ARROW_LENGTH: f32 = 2.0;

/// Smallest range any light outline is drawn at.
///
/// A zero range collapses the sphere and makes `tan` of the cone produce a
/// degenerate radius; a value being typed into the Inspector passes
/// through zero on the way to the intended one.
const MIN_RANGE: f32 = 1e-3;

/// An orthonormal basis whose **Y axis is `forward`**.
///
/// `wire_cone` and friends build along their basis' Y, while a light points
/// along its entity's -Z. This is the adaptor, and it is a function rather
/// than inline maths because getting the handedness wrong flips a cone
/// inside out in a way that is hard to see and easy to repeat.
fn basis_along(forward: Vec3) -> Mat3 {
    let up_ref = if forward.y.abs() > 0.99 {
        Vec3::X
    } else {
        Vec3::Y
    };
    let right = forward.cross(up_ref).normalize_or(Vec3::X);
    let out = right.cross(forward).normalize_or(Vec3::Z);
    Mat3::from_cols(right, forward, out)
}

/// The entity's world-space origin and forward direction.
///
/// `None` when the transform is degenerate — a zero-scaled entity has no
/// direction, and normalising it would give a NaN outline.
fn origin_and_forward(transform: &GlobalTransform) -> Option<(Vec3, Vec3)> {
    let origin = transform.matrix.w_axis.truncate();
    let forward = transform
        .matrix
        .transform_vector3(Vec3::NEG_Z)
        .normalize_or_zero();
    (forward != Vec3::ZERO).then_some((origin, forward))
}

#[derive(Default)]
pub(crate) struct DirectionalLightVisualizer;

impl Visualizer<DirectionalLight> for DirectionalLightVisualizer {
    fn draw(
        &self,
        _light: &DirectionalLight,
        transform: &GlobalTransform,
        gizmos: &mut Gizmos<'_>,
    ) {
        let Some((origin, forward)) = origin_and_forward(transform) else {
            return;
        };
        // The same solid arrow the translate handle draws, so the editor
        // has one arrow shape rather than two that mean the same thing.
        gizmos.filled_arrow(
            origin,
            origin + forward * DIRECTION_ARROW_LENGTH,
            Vec4::new(WHITE.x, WHITE.y, WHITE.z, 1.0),
        );
    }
}

#[derive(Default)]
pub(crate) struct PointLightVisualizer;

impl Visualizer<PointLight> for PointLightVisualizer {
    fn draw(&self, light: &PointLight, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        let (scale, rotation, origin) = transform.matrix.to_scale_rotation_translation();
        // The attenuation cutoff, which is the only thing about a point
        // light that has a place in space.
        let range = (light.range * scale.abs().max_element()).max(MIN_RANGE);
        gizmos.wire_sphere(origin, Mat3::from_quat(rotation), range, WHITE);
    }
}

#[derive(Default)]
pub(crate) struct SpotLightVisualizer;

impl Visualizer<SpotLight> for SpotLightVisualizer {
    fn draw(&self, light: &SpotLight, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        let Some((origin, forward)) = origin_and_forward(transform) else {
            return;
        };
        let scale = transform
            .matrix
            .to_scale_rotation_translation()
            .0
            .abs()
            .max_element();
        let range = (light.range * scale).max(MIN_RANGE);
        let basis = basis_along(forward);

        // Both cones, because the gap between them *is* the falloff. One
        // cone would hide which part of the pool is at full intensity.
        for angle_deg in [light.outer_angle, light.inner_angle] {
            let radius = cone_radius(angle_deg, range);
            // `wire_cone` puts the apex at `centre + y * half_height` and
            // the base at `centre - y * half_height`. With Y along forward,
            // seating the centre half a range back puts the apex on the
            // light and the base out at `range`.
            gizmos.wire_cone(
                origin + forward * range * 0.5,
                Mat3::from_cols(basis.x_axis, -basis.y_axis, basis.z_axis),
                radius,
                range * 0.5,
                WHITE,
            );
        }
    }
}

/// Base radius of a cone of half-angle `angle_deg` at distance `range`.
///
/// # Half-angle, and why it matters that this is written down
///
/// Nothing consumes `SpotLight`'s angles yet — no shader reads them — so
/// drawing the cone *chooses* the convention. Unreal treats inner/outer
/// cone angles as half-angles, measured from the axis to the edge; Unity
/// exposes one full `spotAngle` and halves it internally. Half-angle here,
/// because that is the form the shading maths wants (`cos(outer)` against
/// `dot(light_dir, surface_dir)`) and because inner/outer is the Unreal
/// shape rather than the Unity one.
///
/// If the lighting work later reads these as full angles, this gizmo will
/// draw a cone half the width the shader lights, and this is the line to
/// change.
///
/// Clamped below 90°: at 90° the tangent is infinite and beyond it a
/// "cone" points backwards.
fn cone_radius(angle_deg: f32, range: f32) -> f32 {
    const MAX_HALF_ANGLE: f32 = 89.0;
    let clamped = angle_deg.clamp(0.0, MAX_HALF_ANGLE);
    (range * clamped.to_radians().tan()).max(MIN_RANGE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Mat4;
    use ome_gizmos::{GizmoBatch, MeshBatch};

    /// Draws one visualizer and returns `(line segments, mesh draws)`.
    fn draw<C, V>(visualizer: V, component: &C, matrix: Mat4) -> (Vec<(Vec3, Vec3)>, usize)
    where
        C: ome_ecs::component::Component,
        V: Visualizer<C>,
    {
        let mut lines = GizmoBatch::default();
        let mut meshes = MeshBatch::default();
        {
            let mut gizmos = Gizmos::new(&mut lines, &mut meshes);
            visualizer.draw(component, &GlobalTransform { matrix }, &mut gizmos);
        }
        let segments = lines.lines.iter().map(|s| (s.start, s.end)).collect();
        (segments, mesh_count(&meshes))
    }

    /// How many mesh draws the batch received.
    fn mesh_count(meshes: &MeshBatch) -> usize {
        meshes.draws.len()
    }

    /// The furthest drawn point from `centre`, along each axis.
    fn extent(lines: &[(Vec3, Vec3)], centre: Vec3) -> Vec3 {
        lines
            .iter()
            .flat_map(|(a, b)| [*a, *b])
            .fold(Vec3::ZERO, |acc, p| acc.max((p - centre).abs()))
    }

    /// A directional light draws the handle's solid arrow — a mesh, not
    /// lines. Drawing lines here would be the old `+`-headed arrow, a
    /// second arrow shape for the same meaning.
    #[test]
    fn a_directional_light_draws_a_solid_arrow() {
        let (lines, meshes) = draw(
            DirectionalLightVisualizer,
            &DirectionalLight::default(),
            Mat4::IDENTITY,
        );
        assert!(meshes > 0, "no solid arrow was drawn");
        assert!(
            lines.is_empty(),
            "the line-based arrow is still being drawn as well"
        );
    }

    /// A degenerate transform has no direction, and normalising it would
    /// put NaNs in the batch.
    #[test]
    fn a_zero_scaled_light_draws_nothing() {
        let (lines, meshes) = draw(
            DirectionalLightVisualizer,
            &DirectionalLight::default(),
            Mat4::from_scale(Vec3::ZERO),
        );
        assert!(lines.is_empty() && meshes == 0);
    }

    /// The sphere's radius is the point light's range, which is the only
    /// thing about it that has a place in space.
    #[test]
    fn a_point_light_draws_a_sphere_at_its_range() {
        let light = PointLight {
            range: 7.0,
            ..Default::default()
        };
        let (lines, _) = draw(PointLightVisualizer, &light, Mat4::IDENTITY);

        assert!(!lines.is_empty());
        let reach = extent(&lines, Vec3::ZERO);
        assert!(
            (reach.max_element() - 7.0).abs() < 0.1,
            "the sphere does not match the range: {reach:?}"
        );
    }

    #[test]
    fn a_point_light_scales_with_its_entity() {
        let light = PointLight {
            range: 2.0,
            ..Default::default()
        };
        let (lines, _) = draw(
            PointLightVisualizer,
            &light,
            Mat4::from_scale(Vec3::splat(3.0)),
        );
        let reach = extent(&lines, Vec3::ZERO);
        assert!(
            (reach.max_element() - 6.0).abs() < 0.2,
            "expected 2.0 * 3.0 = 6.0, got {reach:?}"
        );
    }

    /// The apex sits on the light and the cone opens away along forward,
    /// which for an unrotated entity is -Z. A cone drawn the other way is
    /// the mistake `basis_along` exists to prevent.
    #[test]
    fn a_spot_light_opens_forward_from_its_own_origin() {
        let light = SpotLight {
            range: 10.0,
            inner_angle: 30.0,
            outer_angle: 45.0,
            ..Default::default()
        };
        let (lines, _) = draw(SpotLightVisualizer, &light, Mat4::IDENTITY);
        assert!(!lines.is_empty());

        let zs: Vec<f32> = lines.iter().flat_map(|(a, b)| [a.z, b.z]).collect();
        let nearest = zs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let furthest = zs.iter().copied().fold(f32::INFINITY, f32::min);
        assert!(
            nearest <= 0.01,
            "the cone reaches behind the light: max z = {nearest}"
        );
        assert!(
            (furthest + 10.0).abs() < 0.1,
            "the cone does not reach the range: min z = {furthest}"
        );
    }

    /// Two cones, so the gap between them shows the falloff.
    #[test]
    fn a_spot_light_draws_both_cones() {
        let light = SpotLight {
            range: 10.0,
            inner_angle: 20.0,
            outer_angle: 60.0,
            ..Default::default()
        };
        let (lines, _) = draw(SpotLightVisualizer, &light, Mat4::IDENTITY);

        // Radii at the far end: distinct, because the angles are.
        let radii: Vec<f32> = lines
            .iter()
            .flat_map(|(a, b)| [*a, *b])
            .filter(|p| (p.z + 10.0).abs() < 0.2)
            .map(|p| (p.x * p.x + p.y * p.y).sqrt())
            .collect();
        assert!(!radii.is_empty(), "nothing drawn at the far end");
        let widest = radii.iter().copied().fold(0.0f32, f32::max);
        let narrowest = radii.iter().copied().fold(f32::INFINITY, f32::min);
        assert!(
            widest - narrowest > 1.0,
            "only one cone was drawn: radii {narrowest}..{widest}"
        );
    }

    /// Half-angle, and the tangent is clamped below 90° — at 90° it is
    /// infinite and past it a cone points backwards.
    #[test]
    fn the_cone_radius_is_a_half_angle_and_stays_finite() {
        // 45° half-angle at range 10 → radius 10.
        assert!((cone_radius(45.0, 10.0) - 10.0).abs() < 0.01);
        // Well inside: 30° → 10 * tan(30°) ≈ 5.77.
        assert!((cone_radius(30.0, 10.0) - 5.7735).abs() < 0.01);

        for angle in [90.0, 120.0, 1000.0, -30.0] {
            let r = cone_radius(angle, 10.0);
            assert!(r.is_finite() && r > 0.0, "angle {angle} gave radius {r}");
        }
    }

    /// A zero range must not collapse into NaN geometry — the Inspector
    /// passes through it while the user types.
    #[test]
    fn a_zero_range_produces_finite_geometry() {
        let point = PointLight {
            range: 0.0,
            ..Default::default()
        };
        let (lines, _) = draw(PointLightVisualizer, &point, Mat4::IDENTITY);
        for (a, b) in &lines {
            assert!(a.is_finite() && b.is_finite(), "NaN in the batch");
        }

        let spot = SpotLight {
            range: 0.0,
            ..Default::default()
        };
        let (lines, _) = draw(SpotLightVisualizer, &spot, Mat4::IDENTITY);
        for (a, b) in &lines {
            assert!(a.is_finite() && b.is_finite(), "NaN in the batch");
        }
    }

    /// The basis adaptor stays right-handed and orthonormal for every
    /// direction, including straight up where the reference axis flips.
    #[test]
    fn the_basis_adaptor_is_orthonormal_for_any_direction() {
        for forward in [
            Vec3::NEG_Z,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::new(1.0, 1.0, 1.0).normalize(),
        ] {
            let b = basis_along(forward);
            assert!(
                b.y_axis.abs_diff_eq(forward, 1e-4),
                "Y is not forward for {forward:?}"
            );
            for axis in [b.x_axis, b.y_axis, b.z_axis] {
                assert!((axis.length() - 1.0).abs() < 1e-4, "{forward:?}");
            }
            assert!(b.x_axis.dot(b.y_axis).abs() < 1e-4, "{forward:?}");
            assert!(b.y_axis.dot(b.z_axis).abs() < 1e-4, "{forward:?}");
        }
    }
}
