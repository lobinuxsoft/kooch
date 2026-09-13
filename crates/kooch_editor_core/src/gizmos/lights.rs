//! Light visualizers — where a light points and how far it reaches.

use glam::{Mat3, Vec3, Vec4};

use kooch_ecs::directional_light::DirectionalLight;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::point_light::PointLight;
use kooch_ecs::spot_light::SpotLight;
use kooch_gizmos::{Gizmos, Visualizer};

/// Outline colour for every light.
const WHITE: Vec3 = Vec3::ONE;

/// Length of the directional arrow, in world units.
const DIRECTION_ARROW_LENGTH: f32 = 2.0;

/// Smallest range any light outline is drawn at.
const MIN_RANGE: f32 = 1e-3;

/// An orthonormal basis whose **Y axis is `forward`**.
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
            // `wire_cone` puts the apex at `centre + y * half_height` and the base at `centre - y *
            // half_height`. With Y along forward, seating the centre half a range back puts the
            // apex on the light and the base out at `range`.
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
fn cone_radius(angle_deg: f32, range: f32) -> f32 {
    const MAX_HALF_ANGLE: f32 = 89.0;
    let clamped = angle_deg.clamp(0.0, MAX_HALF_ANGLE);
    (range * clamped.to_radians().tan()).max(MIN_RANGE)
}

#[cfg(test)]
mod tests;
