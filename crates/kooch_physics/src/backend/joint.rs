//! Joints — two bodies held together by a constraint.
//!
//! A compound collider ([`attach_collider`]) covers "one body, several
//! shapes". A joint covers the other half: *two* bodies that both simulate,
//! kept in a fixed relationship by the solver. Doors, ragdolls, suspension,
//! robotic arms and rope bridges are all this.
//!
//! # The one axis limits and motors act on
//!
//! Rapier addresses limits and motors per degree of freedom, and a
//! spherical joint has three angular ones. Exposing all six per joint would
//! be six times the Inspector surface for a case that almost never comes
//! up, so a [`JointDesc`] carries **one** limit and **one** motor, applied
//! to the joint's *primary free axis*:
//!
//! | Kind | Primary axis |
//! |---|---|
//! | [`JointKind::Revolute`], [`JointKind::Spherical`], [`JointKind::Generic`] | the angular axis |
//! | [`JointKind::Prismatic`], [`JointKind::PinSlot`] | the linear axis |
//! | [`JointKind::Fixed`], [`JointKind::Rope`], [`JointKind::Spring`] | none — both are ignored |
//!
//! A ragdoll shoulder wanting a swing *cone* rather than a hinge range is
//! the case this does not cover; it wants three motors, and it can have
//! them when something asks for it.
//!
//! [`attach_collider`]: super::PhysicsBackend::attach_collider

use glam::Vec3;

use super::body::BodyHandle;

/// Which constraint the joint applies.
///
/// Every variant maps onto a Rapier joint builder of the same name, so the
/// set is exactly what the solver offers rather than a curated subset.
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
    /// Translation along `axis` plus rotation about it — a cylindrical
    /// joint. Cams, slotted linkages, a bolt in an oversized hole.
    ///
    /// Rapier names this one only in 2D; in 3D the backend spells it out
    /// through rapier's generic joint, because "pin slot" in a plane and
    /// "cylindrical" in space are the same four locked degrees of freedom
    /// counted differently.
    PinSlot { axis: Vec3 },
    /// An arbitrary set of locked degrees of freedom, for the shapes the
    /// named kinds do not cover.
    ///
    /// `locked_axes` is Rapier's `JointAxesMask`: bits 0–2 are the linear
    /// X/Y/Z axes, bits 3–5 the angular ones. This is the escape hatch, not
    /// the thing an author reaches for first.
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

/// A motor driving the joint's primary free axis.
///
/// Position and velocity targets are not exclusive: Rapier's motor solves
/// both terms together, so a non-zero `stiffness` with a zero
/// `target_velocity` is a spring to `target_position`, and a zero
/// `stiffness` with a non-zero `target_velocity` is a free-running drive.
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
    /// Whether this motor has any effect at all.
    ///
    /// Both coefficients zero means the motor contributes nothing to the
    /// solve, so the backend can skip configuring it — and, more usefully,
    /// an author who enabled the motor and left the defaults gets a warning
    /// instead of a joint that mysteriously does nothing.
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
    /// Solve this as a reduced-coordinate articulation rather than as an
    /// impulse constraint.
    ///
    /// A real trade-off, not an implementation detail: an impulse joint is
    /// cheap and drifts slightly under load; a multibody joint cannot drift
    /// because the stretched configuration is not representable, and costs
    /// more per joint. A chain that must not stretch — a robotic arm, an
    /// articulated vehicle — wants this. Rapier also refuses to build a
    /// multibody containing a cycle, so a closed loop must stay on impulse
    /// joints.
    pub articulated: bool,
    /// Whether the two jointed bodies still collide with each other.
    ///
    /// Off by default, because the common case is two parts that overlap at
    /// the joint: a door leaf inside its frame collides with it forever if
    /// this is on.
    pub contacts_enabled: bool,
    /// Impulse magnitude above which the joint breaks, or non-finite for
    /// "never breaks".
    ///
    /// Rapier has no breaking of its own — this is the engine reading the
    /// impulse the solver already computed and removing the constraint when
    /// it is exceeded. Reading the solver's output is not a second solver.
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

    /// Anchors the joint on both bodies.
    pub fn with_anchors(mut self, anchor_a: Vec3, anchor_b: Vec3) -> Self {
        self.anchor_a = anchor_a;
        self.anchor_b = anchor_b;
        self
    }

    /// Bounds the primary free axis.
    pub fn with_limits(mut self, min: f32, max: f32) -> Self {
        self.limits = Some([min, max]);
        self
    }

    /// Drives the primary free axis.
    pub fn with_motor(mut self, motor: JointMotor) -> Self {
        self.motor = Some(motor);
        self
    }

    /// Whether this joint's kind reads [`limits`](Self::limits) and
    /// [`motor`](Self::motor) at all — see the module docs.
    pub fn has_primary_axis(&self) -> bool {
        self.kind.has_primary_axis()
    }
}

impl JointKind {
    /// Whether this kind has an axis for a limit or a motor to act on.
    ///
    /// A fixed joint has no free axis; a rope's length and a spring's rest
    /// length already *are* its constraint. The Inspector hides both
    /// controls for these, and the sync pass declines to pass them, so the
    /// rule is stated once and enforced on both sides of the seam.
    pub fn has_primary_axis(&self) -> bool {
        !matches!(self, Self::Fixed | Self::Rope { .. } | Self::Spring { .. })
    }
}

slotmap::new_key_type! {
    /// Opaque handle for one joint the backend owns.
    ///
    /// Its own key type rather than a reuse of [`BodyHandle`] for the usual
    /// reason: removing a joint must not be expressible as removing a body,
    /// and the type system is where that is cheapest to enforce.
    pub struct JointHandle;
}

/// A joint that broke during a step, reported by [`take_broken_joints`].
///
/// Carries the bodies rather than only the handle because the handle is
/// already dead by the time anyone reads this — the joint was removed. What
/// a caller wants to know is *what came apart*.
///
/// [`take_broken_joints`]: super::PhysicsBackend::take_broken_joints
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
