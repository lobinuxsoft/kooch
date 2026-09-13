use rapier3d::prelude::*;

use glam::{Quat, Vec3};

use crate::backend::{
    ColliderInteraction, CollisionShape, CombineRule, InteractionMask, MIN_EXTENT, SurfaceMaterial,
};

use super::shapes::{ShapeError, shape_builder};

/// Builds the Rapier collider for an engine shape; `offset` becomes its position relative to the
/// body. Since Rapier 0.32 the math types are glam, so only shapes need translating.
pub(super) fn collider_for(
    shape: &CollisionShape,
    offset: Vec3,
    material: SurfaceMaterial,
    interaction: ColliderInteraction,
) -> Result<Collider, ShapeError> {
    Ok(
        with_interaction(with_material(builder_for(shape)?, material), interaction)
            .translation(offset)
            .build(),
    )
}

/// The same with a body-local rotation, or attached child shapes silently axis-align.
pub(super) fn collider_for_pose(
    shape: &CollisionShape,
    offset: Vec3,
    rotation: Quat,
    material: SurfaceMaterial,
    interaction: ColliderInteraction,
) -> Result<Collider, ShapeError> {
    Ok(
        with_interaction(with_material(builder_for(shape)?, material), interaction)
            .position(Pose::from_parts(offset, rotation))
            .build(),
    )
}

/// Applies filtering, sensor and event opt-ins; `ActiveEvents` starts empty in rapier (#561).
fn with_interaction(builder: ColliderBuilder, interaction: ColliderInteraction) -> ColliderBuilder {
    let mut events = ActiveEvents::empty();
    events.set(ActiveEvents::COLLISION_EVENTS, interaction.collision_events);
    events.set(
        ActiveEvents::CONTACT_FORCE_EVENTS,
        interaction.contact_force_events,
    );
    builder
        .sensor(interaction.sensor)
        .active_events(events)
        .contact_force_event_threshold(interaction.contact_force_threshold.max(0.0))
        .collision_groups(groups(interaction.collision_groups))
        .solver_groups(groups(interaction.solver_groups))
}

/// Our mask as rapier's, in `And` mode (0.34): both sides must agree, as
/// [`InteractionMask::interacts_with`] says.
pub(super) fn groups(mask: InteractionMask) -> InteractionGroups {
    InteractionGroups::new(
        Group::from_bits_truncate(mask.memberships),
        Group::from_bits_truncate(mask.filter),
        InteractionTestMode::And,
    )
}

/// Applies surface coefficients in one place, so no path falls back to rapier's defaults (#623).
fn with_material(builder: ColliderBuilder, material: SurfaceMaterial) -> ColliderBuilder {
    let material = material.sanitised();
    builder
        .friction(material.friction)
        .friction_combine_rule(combine_rule(material.friction_rule))
        .restitution(material.restitution)
        .restitution_combine_rule(combine_rule(material.restitution_rule))
}

/// Our rule, as rapier's.
fn combine_rule(rule: CombineRule) -> CoefficientCombineRule {
    match rule {
        CombineRule::Average => CoefficientCombineRule::Average,
        CombineRule::Min => CoefficientCombineRule::Min,
        CombineRule::Multiply => CoefficientCombineRule::Multiply,
        CombineRule::Max => CoefficientCombineRule::Max,
        CombineRule::ClampedSum => CoefficientCombineRule::ClampedSum,
    }
}

/// The builder with offset and density in one place. Colliders are massless — mass is the body's
/// ([`BodyDesc::mass`](crate::backend::BodyDesc::mass)) — or bodies weigh mass plus volume (#618).
fn builder_for(shape: &CollisionShape) -> Result<ColliderBuilder, ShapeError> {
    Ok(shape_builder(shape)?.density(0.0))
}

/// Mass properties of a `mass` kg body shaped like `shape`: measured at unit density and scaled,
/// keeping proportions. Zero-inertia shapes (trimesh, plane) fall back to their enclosing ball;
/// mass is clamped from zero.
pub(super) fn mass_properties_for(
    shape: &CollisionShape,
    mass: f32,
    center_of_mass: Option<Vec3>,
) -> MassProperties {
    const MIN_MASS: f32 = 1e-4;

    let built = shape_builder(shape).map(|builder| builder.build());
    let measured = built
        .as_ref()
        .map(|collider| collider.shape().mass_properties(1.0))
        .unwrap_or_default();

    let mut mprops = match usable(&measured) {
        true => measured,
        false => MassProperties::from_ball(1.0, enclosing_radius(built.as_ref().ok())),
    };
    mprops.set_mass(mass.max(MIN_MASS), true);
    if let Some(center) = center_of_mass {
        mprops.local_com = center;
    }
    mprops
}

/// Whether these properties give the solver something finite to divide by.
fn usable(mprops: &MassProperties) -> bool {
    mprops.mass() > 0.0
        && mprops.mass().is_finite()
        && mprops
            .principal_inertia()
            .to_array()
            .iter()
            .all(|i| *i > 0.0 && i.is_finite())
}

/// Radius of a ball around the shape, 1 m without one; the AABB is clamped, since half-spaces
/// measure infinite.
fn enclosing_radius(collider: Option<&Collider>) -> f32 {
    const FALLBACK: f32 = 1.0;
    const CEILING: f32 = 1.0e4;

    let Some(collider) = collider else {
        return FALLBACK;
    };
    let extents = collider.compute_aabb().extents();
    let radius = extents.max_element() * 0.5;
    match radius.is_finite() && radius > MIN_EXTENT {
        true => radius.min(CEILING),
        false => FALLBACK,
    }
}
