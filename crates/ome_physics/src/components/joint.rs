//! [`Joint`] — two bodies held together by the solver.
//!
//! See the [module docs](super) for why the joint type is a `u32`
//! discriminant rather than an enum, and [`crate::backend`]'s joint
//! documentation for why limits and motors act on a single axis.

use glam::Vec3;

use ome_ecs::Reflect;
use ome_ecs::component::Component;
use ome_ecs::reflect::{EntityRef, FieldChoice, FieldCondition};

use crate::backend::{JointKind, JointMotor, MotorModel};

/// Welds both bodies — all six degrees of freedom removed.
pub const JOINT_FIXED: u32 = 0;
/// A hinge about `axis`.
pub const JOINT_REVOLUTE: u32 = 1;
/// A slider along `axis`.
pub const JOINT_PRISMATIC: u32 = 2;
/// A ball socket: all rotation, no translation.
pub const JOINT_SPHERICAL: u32 = 3;
/// A tether, rigid at `max_length` and free below it.
pub const JOINT_ROPE: u32 = 4;
/// A soft constraint — a force, so it stretches by design.
pub const JOINT_SPRING: u32 = 5;
/// Slide along `axis` and spin about it.
pub const JOINT_PIN_SLOT: u32 = 6;
/// An arbitrary set of locked degrees of freedom.
pub const JOINT_GENERIC: u32 = 7;

/// Labels for the `kind` dropdown in the Inspector.
pub static JOINT_KIND_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Fixed",
        value: JOINT_FIXED as i64,
    },
    FieldChoice {
        label: "Revolute (hinge)",
        value: JOINT_REVOLUTE as i64,
    },
    FieldChoice {
        label: "Prismatic (slider)",
        value: JOINT_PRISMATIC as i64,
    },
    FieldChoice {
        label: "Spherical (ball socket)",
        value: JOINT_SPHERICAL as i64,
    },
    FieldChoice {
        label: "Rope",
        value: JOINT_ROPE as i64,
    },
    FieldChoice {
        label: "Spring",
        value: JOINT_SPRING as i64,
    },
    FieldChoice {
        label: "Pin slot (cylindrical)",
        value: JOINT_PIN_SLOT as i64,
    },
    FieldChoice {
        label: "Generic",
        value: JOINT_GENERIC as i64,
    },
];

/// Motor correction independent of mass — a heavy door and a light one
/// reach the target at the same rate.
pub const MOTOR_ACCELERATION: u32 = 0;
/// Motor correction as a force, so mass matters.
pub const MOTOR_FORCE: u32 = 1;

/// Labels for the `motor_model` dropdown.
pub static MOTOR_MODEL_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Acceleration",
        value: MOTOR_ACCELERATION as i64,
    },
    FieldChoice {
        label: "Force",
        value: MOTOR_FORCE as i64,
    },
];

/// Which kinds read `axis`: the ones with a principal direction.
///
/// Beside the `JOINT_*` constants on purpose — someone adding a kind is
/// already editing here, and a condition kept in another file is a
/// condition that goes stale.
pub static AXIS_WHEN: FieldCondition = FieldCondition {
    field: "kind",
    values: &[
        JOINT_REVOLUTE as i64,
        JOINT_PRISMATIC as i64,
        JOINT_PIN_SLOT as i64,
    ],
};

/// Which kinds read `max_length`: only the rope.
pub static MAX_LENGTH_WHEN: FieldCondition = FieldCondition {
    field: "kind",
    values: &[JOINT_ROPE as i64],
};

/// Which kinds read the spring parameters: only the spring.
pub static SPRING_WHEN: FieldCondition = FieldCondition {
    field: "kind",
    values: &[JOINT_SPRING as i64],
};

/// Which kinds read `locked_axes`: only the generic escape hatch.
pub static LOCKED_AXES_WHEN: FieldCondition = FieldCondition {
    field: "kind",
    values: &[JOINT_GENERIC as i64],
};

/// Which kinds have an axis to limit or drive.
///
/// A fixed joint has no free axis; a rope's length and a spring's rest
/// length already *are* its constraint. Showing a limit range on those
/// would imply it does something.
pub static FREE_AXIS_WHEN: FieldCondition = FieldCondition {
    field: "kind",
    values: &[
        JOINT_REVOLUTE as i64,
        JOINT_PRISMATIC as i64,
        JOINT_SPHERICAL as i64,
        JOINT_PIN_SLOT as i64,
        JOINT_GENERIC as i64,
    ],
};

/// Constrains two bodies to each other.
///
/// # Why the joint names both bodies
///
/// A joint could have lived on one of the bodies, naming only the other —
/// that is what Unity's `connectedBody` does. It costs a body the ability
/// to be in two joints at once, because a component appears once per
/// entity, and a ragdoll pelvis is in four. Naming both bodies puts the
/// joint on whatever entity the author likes, including an empty one, and
/// a body can be in as many as the scene has.
///
/// # Default
///
/// A fixed joint between nothing and nothing: unanchored, unlimited,
/// unmotorised, impulse-solved and unbreakable. It does nothing until both
/// bodies are named, which is the right behaviour for a component that was
/// just added from the menu.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct Joint {
    /// Which constraint to apply. One of the `JOINT_*` constants.
    #[reflect(choices = JOINT_KIND_CHOICES)]
    pub kind: u32,
    /// The first body. Anchors and `axis` are in *its* local space, which
    /// is what makes a hinge authorable: the axis belongs to the door
    /// frame, not to the world.
    #[reflect(requires = "RigidBody")]
    pub body_a: Option<EntityRef>,
    /// The second body.
    #[reflect(requires = "RigidBody")]
    pub body_b: Option<EntityRef>,
    /// Where the joint attaches on `body_a`, in its local space.
    pub anchor_a: Vec3,
    /// Where the joint attaches on `body_b`, in its local space.
    pub anchor_b: Vec3,
    /// The hinge or slide direction.
    #[reflect(shown_when = AXIS_WHEN)]
    pub axis: Vec3,
    /// How far the rope lets the bodies drift apart before going rigid.
    #[reflect(shown_when = MAX_LENGTH_WHEN)]
    pub max_length: f32,
    /// The separation the spring pulls towards.
    #[reflect(shown_when = SPRING_WHEN)]
    pub rest_length: f32,
    /// How hard the spring pulls towards `rest_length`.
    #[reflect(shown_when = SPRING_WHEN)]
    pub stiffness: f32,
    /// How hard the spring resists the bodies' relative motion.
    #[reflect(shown_when = SPRING_WHEN)]
    pub damping: f32,
    /// Bit mask of locked degrees of freedom: bits 0–2 are the linear
    /// X/Y/Z axes, bits 3–5 the angular ones.
    #[reflect(shown_when = LOCKED_AXES_WHEN)]
    pub locked_axes: u8,
    /// Bound the free axis to `[limit_min, limit_max]`.
    #[reflect(shown_when = FREE_AXIS_WHEN)]
    pub limits_enabled: bool,
    /// Lower bound, in radians for a rotation and world units for a slide.
    #[reflect(shown_when = FREE_AXIS_WHEN)]
    pub limit_min: f32,
    /// Upper bound, in radians for a rotation and world units for a slide.
    #[reflect(shown_when = FREE_AXIS_WHEN)]
    pub limit_max: f32,
    /// Drive the free axis.
    #[reflect(shown_when = FREE_AXIS_WHEN)]
    pub motor_enabled: bool,
    /// How the motor converts its error into a correction. One of the
    /// `MOTOR_*` constants.
    #[reflect(shown_when = FREE_AXIS_WHEN, choices = MOTOR_MODEL_CHOICES)]
    pub motor_model: u32,
    /// Angle or offset the motor pulls towards, weighted by
    /// `motor_stiffness`.
    #[reflect(shown_when = FREE_AXIS_WHEN)]
    pub motor_target_position: f32,
    /// Velocity the motor drives at, weighted by `motor_damping`.
    #[reflect(shown_when = FREE_AXIS_WHEN)]
    pub motor_target_velocity: f32,
    /// How hard the motor pulls towards `motor_target_position`.
    #[reflect(shown_when = FREE_AXIS_WHEN)]
    pub motor_stiffness: f32,
    /// How hard the motor holds `motor_target_velocity`.
    #[reflect(shown_when = FREE_AXIS_WHEN)]
    pub motor_damping: f32,
    /// Ceiling on the motor's output. Zero or below means unlimited.
    #[reflect(shown_when = FREE_AXIS_WHEN)]
    pub motor_max_force: f32,
    /// Solve as a reduced-coordinate articulation rather than an impulse
    /// constraint.
    ///
    /// A real trade-off, not an implementation detail: an impulse joint is
    /// cheap and drifts slightly under load; an articulated one cannot
    /// drift, because the stretched configuration is not representable, and
    /// costs more per joint. A chain that must not stretch wants this. A
    /// closed loop cannot use it — a multibody is a tree.
    pub articulated: bool,
    /// Whether the two jointed bodies still collide with each other.
    ///
    /// Off by default, because the common case is two parts that overlap at
    /// the joint: a door leaf inside its frame would collide with it
    /// forever.
    pub contacts_enabled: bool,
    /// Let the joint tear off above `break_impulse`.
    pub breakable: bool,
    /// The pull that tears the joint off, as an impulse magnitude.
    #[reflect(shown_when = BREAKABLE_WHEN)]
    pub break_impulse: f32,
}

/// Which state reads `break_impulse`: only a breakable joint.
pub static BREAKABLE_WHEN: FieldCondition = FieldCondition {
    field: "breakable",
    values: &[1],
};

impl Default for Joint {
    fn default() -> Self {
        Self {
            kind: JOINT_FIXED,
            body_a: None,
            body_b: None,
            anchor_a: Vec3::ZERO,
            anchor_b: Vec3::ZERO,
            axis: Vec3::Y,
            max_length: 1.0,
            rest_length: 1.0,
            stiffness: 100.0,
            damping: 10.0,
            locked_axes: 0,
            limits_enabled: false,
            limit_min: 0.0,
            limit_max: 0.0,
            motor_enabled: false,
            motor_model: MOTOR_ACCELERATION,
            motor_target_position: 0.0,
            motor_target_velocity: 0.0,
            motor_stiffness: 0.0,
            motor_damping: 0.0,
            motor_max_force: 0.0,
            articulated: false,
            contacts_enabled: false,
            breakable: false,
            break_impulse: 100.0,
        }
    }
}

impl Component for Joint {}

impl Joint {
    /// The backend constraint these flat fields describe.
    ///
    /// An unknown discriminant falls back to fixed rather than failing: a
    /// scene authored by a newer editor stays loadable, and a weld is the
    /// least surprising thing for a joint whose type nobody recognises.
    pub fn joint_kind(&self) -> JointKind {
        match self.kind {
            JOINT_REVOLUTE => JointKind::Revolute { axis: self.axis },
            JOINT_PRISMATIC => JointKind::Prismatic { axis: self.axis },
            JOINT_SPHERICAL => JointKind::Spherical,
            JOINT_ROPE => JointKind::Rope {
                max_length: self.max_length.max(0.0),
            },
            JOINT_SPRING => JointKind::Spring {
                rest_length: self.rest_length.max(0.0),
                stiffness: self.stiffness,
                damping: self.damping,
            },
            JOINT_PIN_SLOT => JointKind::PinSlot { axis: self.axis },
            JOINT_GENERIC => JointKind::Generic {
                locked_axes: self.locked_axes,
            },
            _ => JointKind::Fixed,
        }
    }

    /// The authored range on the free axis, or `None` when unbounded.
    pub fn limits(&self) -> Option<[f32; 2]> {
        self.limits_enabled
            .then_some([self.limit_min, self.limit_max])
    }

    /// The authored motor, or `None` when the axis is passive.
    pub fn motor(&self) -> Option<JointMotor> {
        self.motor_enabled.then(|| JointMotor {
            model: match self.motor_model {
                MOTOR_FORCE => MotorModel::ForceBased,
                _ => MotorModel::AccelerationBased,
            },
            target_position: self.motor_target_position,
            target_velocity: self.motor_target_velocity,
            stiffness: self.motor_stiffness,
            damping: self.motor_damping,
            // The Inspector cannot type infinity, so "no ceiling" is
            // spelled as a non-positive number and translated here rather
            // than making every backend understand the convention.
            max_force: match self.motor_max_force > 0.0 {
                true => self.motor_max_force,
                false => f32::INFINITY,
            },
        })
    }

    /// The impulse above which the joint breaks, or infinity for never.
    pub fn break_impulse(&self) -> f32 {
        match self.breakable {
            true => self.break_impulse,
            false => f32::INFINITY,
        }
    }
}
