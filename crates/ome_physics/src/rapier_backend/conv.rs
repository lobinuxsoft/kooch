use rapier3d::prelude::*;

use glam::{Quat, Vec3};

use crate::backend::{
    ColliderInteraction, CollisionShape, CombineRule, InteractionMask, SurfaceMaterial,
};

/// Builds the Rapier collider for an engine shape.
///
/// Since Rapier 0.32 the math types *are* glam's, so this module no
/// longer translates vectors or quaternions — only shapes, which have no
/// engine-side equivalent.
/// `offset` becomes the collider's position relative to its parent body,
/// which is how rapier expresses a shape that is not centred on the body.
pub(super) fn collider_for(
    shape: CollisionShape,
    offset: Vec3,
    material: SurfaceMaterial,
    interaction: ColliderInteraction,
) -> Collider {
    with_interaction(with_material(builder_for(shape), material), interaction)
        .translation(offset)
        .build()
}

/// Same, with a rotation in the body's local space.
///
/// A compound shape needs one: a child entity contributing its collider
/// can be rotated relative to the body, and dropping that would silently
/// axis-align every attached shape.
pub(super) fn collider_for_pose(
    shape: CollisionShape,
    offset: Vec3,
    rotation: Quat,
    material: SurfaceMaterial,
    interaction: ColliderInteraction,
) -> Collider {
    with_interaction(with_material(builder_for(shape), material), interaction)
        .position(Pose::from_parts(offset, rotation))
        .build()
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
fn groups(mask: InteractionMask) -> InteractionGroups {
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
fn builder_for(shape: CollisionShape) -> ColliderBuilder {
    shape_builder(shape).density(0.0)
}

/// The shape, before density or placement.
fn shape_builder(shape: CollisionShape) -> ColliderBuilder {
    match shape {
        CollisionShape::Sphere { radius } => ColliderBuilder::ball(radius),
        CollisionShape::Cuboid { half_extents } => ColliderBuilder::cuboid(
            half_extents.x.max(1e-4),
            half_extents.y.max(1e-4),
            half_extents.z.max(1e-4),
        ),
        CollisionShape::Capsule {
            radius,
            half_height,
        } => ColliderBuilder::capsule_y(half_height, radius),
    }
}

/// The mass properties a body of `mass` kg shaped like `shape` has.
///
/// The shape is measured at unit density and then *scaled* to the authored
/// mass, so the tensor keeps the shape's proportions — a long thin capsule
/// still resists rolling differently from tumbling — while the mass is
/// exactly what the author typed.
///
/// Clamped away from zero: `set_mass(0.0, true)` scales the inertia to
/// zero too, and a body with no inertia takes infinite angular
/// acceleration from any torque. A mass field mid-edit passes through zero
/// on the way to the value the author means, and the NaNs it would produce
/// outlive the typo.
pub(super) fn mass_properties_for(
    shape: CollisionShape,
    mass: f32,
    center_of_mass: Option<Vec3>,
) -> MassProperties {
    const MIN_MASS: f32 = 1e-4;

    let mut mprops = shape_builder(shape).build().shape().mass_properties(1.0);
    mprops.set_mass(mass.max(MIN_MASS), true);
    if let Some(center) = center_of_mass {
        mprops.local_com = center;
    }
    mprops
}
