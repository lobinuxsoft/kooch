//! Two simulating bodies held by a constraint. A [`JointDesc`] has one limit and one motor on the
//! primary free axis: angular for revolute/spherical/generic, linear for prismatic/pin-slot, none
//! for fixed/rope/spring.

use glam::Vec3;

use super::body::BodyHandle;

/// Which constraint applies; every variant maps to the Rapier builder of the same name.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JointKind {
    /// Welds both bodies: all six degrees of freedom removed.
    Fixed,
    /// A hinge. Rotation about `axis` survives; everything else is locked.
    Revolute { axis: Vec3 },
    /// A slider. Translation along `axis` survives; everything else is
    /// locked.
    Prismatic { axis: Vec3 },
    /// A ball socket. All three rotations survive, no translation does.
    Spherical,
    /// A tether: unconstrained below `max_length`, rigid at it.
    Rope { max_length: f32 },
    /// A soft constraint — a force rather than a removed degree of
    /// freedom, so it stretches under load by design.
    Spring {
        rest_length: f32,
        stiffness: f32,
        damping: f32,
    },
    /// Translation along `axis` plus rotation about it — a cylindrical joint. Rapier names it only
    /// in 2D, so the backend spells it through the generic joint.
    PinSlot { axis: Vec3 },
    /// Arbitrary locked degrees of freedom, Rapier's `JointAxesMask`: bits 0–2 linear X/Y/Z, 3–5
    /// angular. The escape hatch.
    Generic { locked_axes: u8 },
}

/// How a motor converts its error into a correction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotorModel {
    /// Correction is independent of the body's mass — a heavy door and a
    /// light one reach the target at the same rate. The usual choice for
    /// animation-like motion.
    AccelerationBased,
    /// Correction is a force, so mass matters. The usual choice when the
    /// motor is meant to read as a physical actuator.
    ForceBased,
}

/// A motor on the primary free axis. Rapier solves position and velocity together: stiffness with
/// zero velocity is a spring, velocity with zero stiffness a drive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JointMotor {
    pub model: MotorModel,
    /// Target angle in radians, or target offset in world units.
    pub target_position: f32,
    /// Target angular or linear velocity.
    pub target_velocity: f32,
    /// How hard the motor pulls towards `target_position`.
    pub stiffness: f32,
    /// How hard the motor resists deviation from `target_velocity`.
    pub damping: f32,
    /// Ceiling on the motor's output. Non-finite or non-positive means
    /// unlimited — a motor with a zero ceiling is a motor that does
    /// nothing, which nobody asks for on purpose.
    pub max_force: f32,
}

impl Default for JointMotor {
    fn default() -> Self {
        Self {
            model: MotorModel::AccelerationBased,
            target_position: 0.0,
            target_velocity: 0.0,
            stiffness: 0.0,
            damping: 0.0,
            max_force: f32::INFINITY,
        }
    }
}

impl JointMotor {
    /// Whether the motor does anything; both coefficients zero is skipped, and the author who
    /// enabled it gets a warning.
    pub fn is_effective(&self) -> bool {
        self.stiffness != 0.0 || self.damping != 0.0
    }
}

/// Construction descriptor handed to [`add_joint`].
///
/// [`add_joint`]: super::PhysicsBackend::add_joint
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JointDesc {
    /// The first body. Anchors and axes are expressed in *its* local
    /// space, which is what makes a hinge authorable: the axis is a
    /// property of the door frame, not of the world.
    pub body_a: BodyHandle,
    /// The second body.
    pub body_b: BodyHandle,
    pub kind: JointKind,
    /// Where the joint attaches on `body_a`, in its local space.
    pub anchor_a: Vec3,
    /// Where the joint attaches on `body_b`, in its local space.
    pub anchor_b: Vec3,
    /// Range on the primary free axis — see the module docs. `None` leaves
    /// the axis unbounded.
    pub limits: Option<[f32; 2]>,
    /// Motor on the primary free axis. `None` leaves the axis passive.
    pub motor: Option<JointMotor>,
    /// Solve as a reduced-coordinate multibody: it cannot drift and costs more per joint — for
    /// chains that must not stretch. Rapier rejects cycles, so closed loops stay on impulse joints.
    pub articulated: bool,
    /// Whether the jointed bodies still collide; off, since a door leaf overlaps its frame at the
    /// hinge.
    pub contacts_enabled: bool,
    /// Impulse above which the joint breaks, non-finite for never. The engine reads the solver's
    /// impulse; Rapier has no breaking.
    pub break_impulse: f32,
}

impl JointDesc {
    /// A joint of `kind` between two bodies, anchored at both origins,
    /// unlimited, unmotorised, impulse-solved and unbreakable.
    pub fn new(body_a: BodyHandle, body_b: BodyHandle, kind: JointKind) -> Self {
        Self {
            body_a,
            body_b,
            kind,
            anchor_a: Vec3::ZERO,
            anchor_b: Vec3::ZERO,
            limits: None,
            motor: None,
            articulated: false,
            contacts_enabled: false,
            break_impulse: f32::INFINITY,
        }
    }

    /// Whether this joint's kind reads [`limits`](Self::limits) and
    /// [`motor`](Self::motor) at all — see the module docs.
    pub fn has_primary_axis(&self) -> bool {
        self.kind.has_primary_axis()
    }
}

impl JointKind {
    /// Whether this kind has an axis to limit or drive — the rule the Inspector and sync pass both
    /// enforce.
    pub fn has_primary_axis(&self) -> bool {
        !matches!(self, Self::Fixed | Self::Rope { .. } | Self::Spring { .. })
    }
}

slotmap::new_key_type! {
    /// Handle for one joint, its own key type so removing a joint cannot be written as removing a
    /// body.
    pub struct JointHandle;
}

/// A joint that broke during a step, from
/// [`PhysicsBackend::take_broken_joints`](super::PhysicsBackend::take_broken_joints); it names the
/// bodies, since the joint's handle is already dead.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrokenJoint {
    pub joint: JointHandle,
    pub body_a: BodyHandle,
    pub body_b: BodyHandle,
    /// The impulse magnitude that exceeded the threshold.
    pub impulse: f32,
}

#[cfg(test)]
mod tests;
