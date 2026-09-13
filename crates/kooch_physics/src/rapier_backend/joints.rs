//! [`JointDesc`] → the Rapier joint; every [`JointKind`] has a builder of the same name.

use rapier3d::dynamics::{
    FixedJointBuilder, GenericJointBuilder, ImpulseJointHandle, JointAxesMask, JointAxis,
    MotorModel as RapierMotorModel, MultibodyJointHandle, PrismaticJointBuilder,
    RevoluteJointBuilder, RopeJointBuilder, SphericalJointBuilder, SpringJointBuilder,
};
use rapier3d::prelude::GenericJoint;

use crate::backend::{BodyHandle, JointDesc, JointKind, JointMotor, MotorModel};

/// Which Rapier joint set holds a joint — each removes with its own handle, so the choice is
/// remembered.
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

/// Builds the joint with anchors applied once, so no kind forgets them.
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

/// Cylindrical joint: Rapier's `PinSlotJointBuilder` is 2D only, so the generic joint frees `LIN_X`
/// and `ANG_X` along the axis.
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

/// The single axis limits and motors act on (see [`crate::backend`]); `None` for fixed, rope and
/// spring.
pub(super) fn primary_axis(kind: &JointKind) -> Option<JointAxis> {
    match kind {
        JointKind::Revolute { .. } | JointKind::Spherical | JointKind::Generic { .. } => {
            Some(JointAxis::AngX)
        }
        JointKind::Prismatic { .. } | JointKind::PinSlot { .. } => Some(JointAxis::LinX),
        JointKind::Fixed | JointKind::Rope { .. } | JointKind::Spring { .. } => None,
    }
}

/// A usable hinge or slider axis: zero builds a NaN frame, so an edit through zero falls back to Y.
fn safe_axis(axis: glam::Vec3) -> glam::Vec3 {
    axis.try_normalize().unwrap_or(glam::Vec3::Y)
}

/// Linear impulse magnitude only: a norm over force and torque mixes units no author can read, and
/// breaking thresholds are pulls.
pub(super) fn linear_impulse(impulses: &[f32; 6]) -> f32 {
    glam::Vec3::from_slice(&impulses[..3]).length()
}

#[cfg(test)]
mod tests;
