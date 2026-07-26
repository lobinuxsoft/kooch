//! Translating a [`JointDesc`] into the Rapier joint it describes.
//!
//! Every [`JointKind`] has a Rapier builder of the same name, so this is a
//! mapping rather than a construction: the engine exposes what the solver
//! already offers.

use rapier3d::dynamics::{
    FixedJointBuilder, GenericJointBuilder, ImpulseJointHandle, JointAxesMask, JointAxis,
    MotorModel as RapierMotorModel, MultibodyJointHandle, PrismaticJointBuilder,
    RevoluteJointBuilder, RopeJointBuilder, SphericalJointBuilder, SpringJointBuilder,
};
use rapier3d::prelude::GenericJoint;

use crate::backend::{BodyHandle, JointDesc, JointKind, JointMotor, MotorModel};

/// Which of Rapier's two joint sets holds a joint.
///
/// The families are not interchangeable at removal time — each set has its
/// own handle type and its own `remove` — so the choice made at build time
/// has to be remembered rather than re-derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum JointRef {
    Impulse(ImpulseJointHandle),
    Multibody(MultibodyJointHandle),
}

/// What the backend keeps per live joint.
pub(super) struct JointEntry {
    pub reference: JointRef,
    pub body_a: BodyHandle,
    pub body_b: BodyHandle,
    /// Linear impulse above which the joint breaks, or non-finite for
    /// never. See [`crate::backend::JointDesc::break_impulse`].
    pub break_impulse: f32,
}

/// Builds the Rapier joint a descriptor asks for.
///
/// Anchors are applied here rather than per branch so no kind can forget
/// them — a joint anchored at both origins when the author asked otherwise
/// is a bug that looks like a solver problem.
pub(super) fn generic_joint_for(desc: &JointDesc) -> GenericJoint {
    let mut joint: GenericJoint = match desc.kind {
        JointKind::Fixed => FixedJointBuilder::new().build().into(),
        JointKind::Revolute { axis } => RevoluteJointBuilder::new(safe_axis(axis)).build().into(),
        JointKind::Prismatic { axis } => PrismaticJointBuilder::new(safe_axis(axis)).build().into(),
        JointKind::Spherical => SphericalJointBuilder::new().build().into(),
        JointKind::Rope { max_length } => RopeJointBuilder::new(max_length.max(0.0)).build().into(),
        JointKind::Spring {
            rest_length,
            stiffness,
            damping,
        } => SpringJointBuilder::new(rest_length.max(0.0), stiffness, damping)
            .build()
            .into(),
        JointKind::PinSlot { axis } => pin_slot(axis),
        JointKind::Generic { locked_axes } => {
            GenericJointBuilder::new(JointAxesMask::from_bits_truncate(locked_axes))
                .build()
                .into()
        }
    };

    joint.set_local_anchor1(desc.anchor_a);
    joint.set_local_anchor2(desc.anchor_b);
    joint.set_contacts_enabled(desc.contacts_enabled);

    if let Some(axis) = primary_axis(&desc.kind) {
        if let Some([min, max]) = desc.limits {
            // Rapier reads these as `[min, max]` and misbehaves quietly if
            // they arrive the other way round, which an author dragging a
            // range in the Inspector will do.
            joint.set_limits(axis, [min.min(max), min.max(max)]);
        }
        if let Some(motor) = desc.motor.filter(JointMotor::is_effective) {
            apply_motor(&mut joint, axis, &motor);
        }
    }

    joint
}

/// Slide along an axis plus spin about it — a cylindrical joint.
///
/// Rapier ships `PinSlotJointBuilder` for 2D only: there, "pin slot" means
/// one free translation and the single rotation a plane has. The 3D
/// equivalent has no named builder, so it is spelled out through rapier's
/// own generic joint — the five-line definition of the joint, not a
/// reimplementation of one. Free axes are `LIN_X` and `ANG_X`, both along
/// the joint frame's axis.
fn pin_slot(axis: glam::Vec3) -> GenericJoint {
    let axis = safe_axis(axis);
    GenericJointBuilder::new(
        JointAxesMask::LIN_Y | JointAxesMask::LIN_Z | JointAxesMask::ANG_Y | JointAxesMask::ANG_Z,
    )
    .local_axis1(axis)
    .local_axis2(axis)
    .build()
}

/// Configures the motor on one axis.
fn apply_motor(joint: &mut GenericJoint, axis: JointAxis, motor: &JointMotor) {
    joint.set_motor_model(
        axis,
        match motor.model {
            MotorModel::AccelerationBased => RapierMotorModel::AccelerationBased,
            MotorModel::ForceBased => RapierMotorModel::ForceBased,
        },
    );
    // Both terms in one call: rapier solves position and velocity targets
    // together, so setting them separately would have the second overwrite
    // the first's coefficients.
    joint.set_motor(
        axis,
        motor.target_position,
        motor.target_velocity,
        motor.stiffness,
        motor.damping,
    );
    if motor.max_force.is_finite() && motor.max_force > 0.0 {
        joint.set_motor_max_force(axis, motor.max_force);
    }
}

/// The single axis limits and motors act on — see [`crate::backend`]'s
/// joint documentation for why there is only one.
///
/// `None` for the kinds that have nothing to limit or drive: a fixed joint
/// has no free axis, and a rope's length and a spring's rest length are
/// already its constraint.
pub(super) fn primary_axis(kind: &JointKind) -> Option<JointAxis> {
    match kind {
        JointKind::Revolute { .. } | JointKind::Spherical | JointKind::Generic { .. } => {
            Some(JointAxis::AngX)
        }
        JointKind::Prismatic { .. } | JointKind::PinSlot { .. } => Some(JointAxis::LinX),
        JointKind::Fixed | JointKind::Rope { .. } | JointKind::Spring { .. } => None,
    }
}

/// A usable axis for a hinge or a slider.
///
/// A zero or denormal axis makes rapier build a degenerate frame whose
/// output is NaN, and a NaN in the solver outlives the frame that produced
/// it. An author mid-edit in the Inspector passes through zero on the way
/// to the value they meant, so the fallback is Y — up, which is the axis
/// most hinges use anyway.
fn safe_axis(axis: glam::Vec3) -> glam::Vec3 {
    axis.try_normalize().unwrap_or(glam::Vec3::Y)
}

/// The magnitude of the linear impulse holding a joint together.
///
/// Linear only, deliberately. Rapier reports six components — three of
/// force, three of torque — and a norm over all six adds newton-seconds to
/// newton-metre-seconds, producing a number no author can reason about.
/// "The pull it takes to tear this apart" is a quantity with a unit, and it
/// is the one a breaking threshold is written against. A separate torque
/// threshold is what a joint failing in bending would want, and it can have
/// one when something asks.
pub(super) fn linear_impulse(impulses: &[f32; 6]) -> f32 {
    glam::Vec3::from_slice(&impulses[..3]).length()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use slotmap::SlotMap;

    fn desc(kind: JointKind) -> JointDesc {
        let mut bodies: SlotMap<BodyHandle, ()> = SlotMap::with_key();
        let (a, b) = (bodies.insert(()), bodies.insert(()));
        JointDesc::new(a, b, kind)
    }

    /// A hinge whose axis is mid-edit passes through zero, and a degenerate
    /// joint frame produces NaNs that outlive the typo.
    #[test]
    fn a_degenerate_axis_falls_back_instead_of_producing_nans() {
        assert_eq!(safe_axis(Vec3::ZERO), Vec3::Y);
        assert!(safe_axis(Vec3::new(0.0, 0.0, 4.0)).abs_diff_eq(Vec3::Z, 1e-6));
    }

    /// Dragging a range in the Inspector puts the handles the wrong way
    /// round, and rapier does not sort them itself.
    #[test]
    fn inverted_limits_are_ordered_before_reaching_the_solver() {
        let joint = generic_joint_for(&JointDesc {
            limits: Some([1.5, -0.5]),
            ..desc(JointKind::Revolute { axis: Vec3::Y })
        });
        let limits = joint.limits(JointAxis::AngX).expect("the axis is limited");
        assert_eq!((limits.min, limits.max), (-0.5, 1.5));
    }

    /// The kinds with no free axis must not silently accept a limit that
    /// does nothing.
    #[test]
    fn the_kinds_without_a_free_axis_ignore_limits() {
        for kind in [
            JointKind::Fixed,
            JointKind::Rope { max_length: 2.0 },
            JointKind::Spring {
                rest_length: 1.0,
                stiffness: 10.0,
                damping: 1.0,
            },
        ] {
            assert!(primary_axis(&kind).is_none(), "{kind:?} has no free axis");
        }
    }

    #[test]
    fn anchors_reach_the_built_joint() {
        let joint = generic_joint_for(&JointDesc {
            anchor_a: Vec3::new(1.0, 0.0, 0.0),
            anchor_b: Vec3::new(-1.0, 0.0, 0.0),
            ..desc(JointKind::Spherical)
        });
        assert_eq!(joint.local_anchor1(), Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(joint.local_anchor2(), Vec3::new(-1.0, 0.0, 0.0));
    }

    /// Two bodies welded at a joint overlap there; colliding by default
    /// makes a door fight its own frame forever.
    #[test]
    fn contacts_between_jointed_bodies_are_off_unless_asked_for() {
        assert!(!generic_joint_for(&desc(JointKind::Fixed)).contacts_enabled());
        assert!(
            generic_joint_for(&JointDesc {
                contacts_enabled: true,
                ..desc(JointKind::Fixed)
            })
            .contacts_enabled()
        );
    }

    /// A motor left at its defaults contributes nothing, and configuring
    /// one anyway would put an inert motor on the axis where an author
    /// later looks for the real one.
    #[test]
    fn an_inert_motor_is_not_configured() {
        let joint = generic_joint_for(&JointDesc {
            motor: Some(JointMotor::default()),
            ..desc(JointKind::Revolute { axis: Vec3::Y })
        });
        assert!(
            joint.motor(JointAxis::AngX).is_none(),
            "an all-zero motor was still registered on the axis",
        );
    }

    #[test]
    fn a_velocity_motor_reaches_the_built_joint() {
        let joint = generic_joint_for(&JointDesc {
            motor: Some(JointMotor {
                target_velocity: 4.0,
                damping: 2.0,
                ..Default::default()
            }),
            ..desc(JointKind::Revolute { axis: Vec3::Y })
        });
        let motor = joint.motor(JointAxis::AngX).expect("the axis is motorised");
        assert_eq!(motor.target_vel, 4.0);
        assert_eq!(motor.damping, 2.0);
    }

    /// Mixing newton-seconds with newton-metre-seconds gives a threshold
    /// nobody can author against.
    #[test]
    fn the_break_measure_ignores_the_torque_components() {
        assert_eq!(linear_impulse(&[3.0, 4.0, 0.0, 100.0, 100.0, 100.0]), 5.0);
    }
}
