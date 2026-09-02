//! Asking the world a question without moving anything.
//!
//! A ray is the query everyone reaches for and the one that is wrong most
//! often. It is a line of zero width: it slips between two crates that a
//! body could never fit through, it finds the lip of a step instead of the
//! step, and it misses a thin wall that a fast projectile would hit. What
//! a character actually does is move a *shape*, so the honest question is
//! about a shape.
//!
//! Every type here is glam and [`BodyHandle`], the same rule the rest of
//! the backend follows: rapier's own query results never cross this line.

use glam::Vec3;

use super::BodyHandle;
use super::interaction::InteractionMask;

/// Which bodies a query is allowed to see.
///
/// Filtering here rather than in the caller is not only tidier: the
/// pipeline skips a rejected collider before testing it, so a query that
/// excludes its own body does less work than one that finds itself and
/// throws the answer away. Post-filtering also gets the common case
/// wrong — a character casting downward hits *itself* first, and
/// discarding only the nearest hit still misses the second collider on
/// the same body.
#[derive(Debug, Clone, Copy)]
pub struct QueryFilter {
    /// Skip this body and every collider it owns.
    ///
    /// What a character controller needs on every ground probe.
    pub exclude: Option<BodyHandle>,
    /// Groups the query belongs to and will interact with, read the same
    /// way a collider's own [`InteractionMask`] is.
    pub groups: InteractionMask,
    /// Skip colliders that only report overlaps.
    ///
    /// A trigger volume is not a floor and not a wall. On by default,
    /// because a query that stops at a checkpoint marker is a bug in
    /// every caller that has ever written one.
    pub skip_sensors: bool,
}

impl Default for QueryFilter {
    fn default() -> Self {
        Self::ALL
    }
}

impl QueryFilter {
    /// Everything solid, excluding nothing.
    pub const ALL: Self = Self {
        exclude: None,
        groups: InteractionMask::ALL,
        skip_sensors: true,
    };

    /// The same, blind to one body — almost always the one asking.
    pub fn excluding(body: BodyHandle) -> Self {
        Self {
            exclude: Some(body),
            ..Self::ALL
        }
    }

    /// Narrowed to a group mask.
    pub fn in_groups(mut self, groups: InteractionMask) -> Self {
        self.groups = groups;
        self
    }

    /// Including sensors, for a query that is *looking* for triggers.
    pub fn with_sensors(mut self) -> Self {
        self.skip_sensors = false;
        self
    }
}

/// Where a swept shape first met something.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeHit {
    /// Body the shape ran into.
    pub body: BodyHandle,
    /// Distance travelled before contact, in the direction's own lengths.
    pub t: f32,
    /// World-space contact point on the body that was hit.
    pub point: Vec3,
    /// World-space surface normal there — the slope of the ground, or the
    /// face of the wall.
    pub normal: Vec3,
    /// The cast began already touching.
    ///
    /// `t` is then zero and `normal` points the way out rather than
    /// describing a surface the shape ran into. Worth branching on: a
    /// controller that treats it as an ordinary contact will push itself
    /// further into whatever it is stuck in.
    pub penetrating: bool,
}

/// The nearest point on the nearest body to some point in space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointHit {
    /// Body the point projected onto.
    pub body: BodyHandle,
    /// World-space nearest point on it.
    pub point: Vec3,
    /// The queried point was inside that body.
    ///
    /// `point` is then still the nearest surface point, which is what
    /// makes this useful for pushing something back out.
    pub inside: bool,
}

/// A shape placed in the world, for the queries that take one.
///
/// Bundled rather than passed as three parameters: a sweep already needs
/// a direction, a distance and a filter, and six positional arguments is
/// where call sites start swapping two `Vec3`s without the compiler
/// noticing.
#[derive(Debug, Clone, Copy)]
pub struct ShapeAt<'a> {
    /// What to place. Mesh-derived shapes must already carry their
    /// geometry — a query has no cache to resolve a `Guid` against.
    pub shape: &'a super::CollisionShape,
    /// Where its centre sits.
    pub origin: Vec3,
    /// How it is turned.
    pub rotation: glam::Quat,
}

impl<'a> ShapeAt<'a> {
    /// Unrotated, at a point.
    pub fn new(shape: &'a super::CollisionShape, origin: Vec3) -> Self {
        Self {
            shape,
            origin,
            rotation: glam::Quat::IDENTITY,
        }
    }

    /// Turned.
    pub fn turned(mut self, rotation: glam::Quat) -> Self {
        self.rotation = rotation;
        self
    }
}
