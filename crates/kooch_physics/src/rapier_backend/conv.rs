use rapier3d::prelude::*;

use glam::{Quat, Vec3};

use crate::backend::{
    ColliderInteraction, CollisionShape, CombineRule, InteractionMask, MIN_EXTENT, SurfaceMaterial,
};

use super::shapes::{ShapeError, shape_builder};

/// Builds the Rapier collider for an engine shape.
///
/// Since Rapier 0.32 the math types *are* glam's, so this module no
/// longer translates vectors or quaternions — only shapes, which have no
/// engine-side equivalent.
/// `offset` becomes the collider's position relative to its parent body,
/// which is how rapier expresses a shape that is not centred on the body.
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

/// Same, with a rotation in the body's local space.
///
/// A compound shape needs one: a child entity contributing its collider
/// can be rotated relative to the body, and dropping that would silently
/// axis-align every attached shape.
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

/// Applies the filtering, the sensor flag and the event opt-ins.
///
/// `ActiveEvents` starts empty in rapier, so a collider that does not ask
/// generates nothing — which is why the engine heard silence until #561.
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

/// Our mask, as rapier's.
///
/// `And` is the test mode rapier 0.34 added and the one
/// [`InteractionMask::interacts_with`] documents: both sides have to agree.
/// `Or` would let a one-sided claim through, which is the behaviour that
/// makes filtering bugs impossible to reason about.
pub(super) fn groups(mask: InteractionMask) -> InteractionGroups {
    InteractionGroups::new(
        Group::from_bits_truncate(mask.memberships),
        Group::from_bits_truncate(mask.filter),
        InteractionTestMode::And,
    )
}

/// Applies the surface coefficients to a builder.
///
/// Applied here rather than per call site so no path can forget them and
/// silently fall back to rapier's defaults — which is the state #623 was
/// filed about.
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

/// The un-built builder, so the offset and the density are applied in one
/// place.
///
/// Every collider is built massless. Mass belongs to the body — see
/// [`BodyDesc::mass`](crate::backend::BodyDesc::mass) for why the shapes
/// do not get a say. A density left at rapier's default would make a body
/// weigh its authored mass *plus* its volume, which is exactly the units
/// bug #618 exists to close.
fn builder_for(shape: &CollisionShape) -> Result<ColliderBuilder, ShapeError> {
    Ok(shape_builder(shape)?.density(0.0))
}

/// The mass properties a body of `mass` kg shaped like `shape` has.
///
/// The shape is measured at unit density and then *scaled* to the authored
/// mass, so the tensor keeps the shape's proportions — a long thin capsule
/// still resists rolling differently from tumbling — while the mass is
/// exactly what the author typed.
///
/// # A hollow shape still needs a tensor
///
/// A trimesh, a plane and a polyline all measure to zero inertia, and
/// `set_mass(m, true)` scales zero to zero — leaving a body that takes
/// infinite angular acceleration from any torque. Those fall back to the
/// ball that encloses them, which is wrong in the way every engine is
/// wrong here and finite in the way that matters.
///
/// Clamped away from zero for the same reason a dimension is: a mass field
/// mid-edit passes through zero on the way to the value the author means,
/// and the NaNs it would produce outlive the typo.
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

/// The radius of a ball around the shape, or one metre when there is no
/// shape to measure.
///
/// An infinite half-space measures infinite, so the AABB is clamped: the
/// number only has to be finite and roughly the size of the thing.
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
