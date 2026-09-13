//! [`PhysicsBody`]: whether the solver moves an entity and how much of it there is; `kind` is a
//! discriminant, see [module docs](super::super).

use glam::Vec3;

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;
use kooch_ecs::reflect::{FieldChoice, FieldCondition};

use crate::backend::{BodyKind, Damping};

/// Solver-driven: gravity and collisions move it.
pub const KIND_DYNAMIC: u32 = 0;
/// Author-driven: you set its transform, and it pushes dynamic bodies.
pub const KIND_KINEMATIC: u32 = 1;
/// Immovable: nothing moves it, it stops everything else.
pub const KIND_STATIC: u32 = 2;

/// Labels for the `kind` dropdown in the Inspector.
pub static KIND_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Dynamic",
        value: KIND_DYNAMIC as i64,
    },
    FieldChoice {
        label: "Kinematic",
        value: KIND_KINEMATIC as i64,
    },
    FieldChoice {
        label: "Static",
        value: KIND_STATIC as i64,
    },
];

/// Which state reads `center_of_mass`: only an explicit override.
pub static CENTER_OF_MASS_WHEN: FieldCondition = FieldCondition {
    field: "center_of_mass_enabled",
    values: &[1],
};

/// Fields only solver-driven bodies read, hidden for static ones — six dead controls teach
/// distrust. Display only: values round-trip, so Static and back restores them.
pub static DYNAMIC_ONLY: FieldCondition = FieldCondition {
    field: "kind",
    values: &[KIND_DYNAMIC as i64],
};

/// Simulated entity; mass is [`mass`](Self::mass) alone — colliders add none, since rapier's shape
/// mass made 1 kg weigh 34 (#618). Inertia from its own collider; [`density`](Self::density) feeds
/// **Calculate mass**. Default: dynamic, 1 kg.
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Physics")]
pub struct PhysicsBody {
    /// How the solver treats this body. One of the `KIND_*` constants.
    #[reflect(choices = KIND_CHOICES)]
    pub kind: u32,
    /// Mass in kilograms — the body's whole mass. Ignored by static and
    /// kinematic bodies.
    #[reflect(shown_when = DYNAMIC_ONLY)]
    pub mass: f32,
    /// kg/m³ for **Calculate mass**, which multiplies it by the colliders' volume into
    /// [`mass`](Self::mass). **The simulation never reads it.** Water 1000, aluminium ~2700, steel
    /// ~7850, pine ~600.
    #[reflect(shown_when = DYNAMIC_ONLY)]
    pub density: f32,
    /// Put the centre of mass somewhere other than the collider's centre.
    #[reflect(shown_when = DYNAMIC_ONLY)]
    pub center_of_mass_enabled: bool,
    /// Centre of mass in local space (Unity `centerOfMass`, Unreal `COMOffset`) — a vehicle wants
    /// it low.
    #[reflect(shown_when = CENTER_OF_MASS_WHEN)]
    pub center_of_mass: Vec3,
    /// Linear motion lost with nothing touching — air, not ice. Zero keeps motion forever, rapier's
    /// default (#623).
    #[reflect(shown_when = DYNAMIC_ONLY)]
    pub linear_damping: f32,
    /// The same for spin. A thrown object that should stop tumbling wants
    /// this; a wheel that should keep turning does not.
    #[reflect(shown_when = DYNAMIC_ONLY)]
    pub angular_damping: f32,
    /// Gravity multiplier: 1 normal, 0 weightless, 0.16 the Moon, negative a balloon. An
    /// acceleration, so bodies of any mass still fall together.
    #[reflect(shown_when = DYNAMIC_ONLY)]
    pub gravity_scale: f32,
}

impl Default for PhysicsBody {
    fn default() -> Self {
        Self {
            kind: KIND_DYNAMIC,
            mass: 1.0,
            density: 1000.0,
            center_of_mass_enabled: false,
            center_of_mass: Vec3::ZERO,
            linear_damping: 0.0,
            angular_damping: 0.0,
            gravity_scale: 1.0,
        }
    }
}

impl Component for PhysicsBody {}

impl PhysicsBody {
    /// The backend body kind, defaulting to dynamic for a discriminant
    /// outside the known set — a scene authored by a newer editor stays
    /// loadable rather than failing.
    pub fn body_kind(&self) -> BodyKind {
        match self.kind {
            KIND_KINEMATIC => BodyKind::Kinematic,
            KIND_STATIC => BodyKind::Static,
            _ => BodyKind::Dynamic,
        }
    }

    /// The authored centre of mass, or `None` to use the collider's.
    pub fn explicit_center_of_mass(&self) -> Option<Vec3> {
        self.center_of_mass_enabled.then_some(self.center_of_mass)
    }

    /// The damping the backend applies to this body.
    pub fn damping(&self) -> Damping {
        Damping {
            linear: self.linear_damping,
            angular: self.angular_damping,
        }
        .sanitised()
    }
}
