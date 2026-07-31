//! [`RigidBody`] — whether the solver moves an entity, and how much of it there is.
//!
//! See the [components module docs](super::super) for why `kind` is a
//! `u32` discriminant rather than an enum: reflection has no enums, so a
//! variant is a number with a labelled choice set beside it.

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

/// Marks an entity as participating in the physics simulation.
///
/// Pairs with a [`Collider`]; an entity carrying only this gets a unit
/// sphere, so a half-authored entity falls rather than panics.
///
/// # Where the mass comes from
///
/// From [`mass`](Self::mass), and nowhere else. Colliders contribute
/// collision and no mass at all.
///
/// This is a deliberate departure from Unity and Unreal, which derive a
/// body's mass properties from every attached shape's volume and density.
/// That is physically correct and it is what surprised the author who
/// filed #618: adding a second collider to a body made it heavier and
/// slower to turn, with nothing in the Inspector saying so. Worse, the
/// `mass` field did not mean kilograms — rapier's `additional_mass` is
/// *added* to the shape-derived mass, so a 1 kg body with a two-metre
/// sphere weighed thirty-four.
///
/// A field labelled kilograms has to mean kilograms. So the shapes are
/// massless, `mass` is the whole mass, and the body's inertia is derived
/// from the entity's *own* collider scaled to that mass — which is also
/// why a compound body's centre of mass stays where the author expects
/// instead of drifting towards the children.
///
/// What that gives up is "a bigger rock is automatically heavier".
/// [`density`](Self::density) buys it back on demand: the Inspector's
/// **Calculate mass** button multiplies it by the colliders' volume and
/// writes the result here, once, where you can see and edit it.
///
/// # Default
///
/// A dynamic body of 1 kg at the density of water.
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Physics")]
pub struct RigidBody {
    /// How the solver treats this body. One of the `KIND_*` constants.
    #[reflect(choices = KIND_CHOICES)]
    pub kind: u32,
    /// Mass in kilograms — the body's whole mass. Ignored by static and
    /// kinematic bodies.
    pub mass: f32,
    /// Kilograms per cubic metre, for the Inspector's **Calculate mass**
    /// button.
    ///
    /// **The simulation never reads this.** It is authoring input: the
    /// button multiplies it by the volume of this entity's colliders and
    /// writes the product into [`mass`](Self::mass). Kept as a field
    /// rather than asked for in a dialog so the number that produced a
    /// mass is visible next to it — 1000 is water, ~2700 aluminium, ~7850
    /// steel, ~600 dry pine.
    pub density: f32,
    /// Put the centre of mass somewhere other than the collider's centre.
    pub center_of_mass_enabled: bool,
    /// The centre of mass, in the entity's local space.
    ///
    /// What Unity calls `centerOfMass` and Unreal calls `COMOffset`. A
    /// vehicle wants its centre of mass low or it rolls in every corner,
    /// and no arrangement of collision shapes expresses that as directly
    /// as saying where it is.
    #[reflect(shown_when = CENTER_OF_MASS_WHEN)]
    pub center_of_mass: Vec3,
    /// How quickly the body loses linear motion with nothing touching it.
    ///
    /// Not friction — this applies in a vacuum, which makes it the tool for
    /// "this moves through air" and the wrong one for "this slides less on
    /// ice". Zero means a body keeps its motion forever, which is rapier's
    /// default and was the engine's only option until #623.
    pub linear_damping: f32,
    /// The same for spin. A thrown object that should stop tumbling wants
    /// this; a wheel that should keep turning does not.
    pub angular_damping: f32,
    /// How much the world's gravity pulls on this body.
    ///
    /// A multiplier, not a replacement: 1 is normal, 0 is weightless, 0.16
    /// is the Moon, and 2 is a body that falls twice as fast. Negative is
    /// legal and is how a balloon rises.
    ///
    /// Gravity is an *acceleration*, so this changes how fast the body
    /// falls and not how heavy it is — two bodies of different mass at the
    /// same scale still fall together.
    pub gravity_scale: f32,
}

impl Default for RigidBody {
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

impl Component for RigidBody {}

impl RigidBody {
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
