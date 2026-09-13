//! Queries that move nothing. Rays are zero-width and often wrong for a moving body, which moves a
//! shape. Glam and [`BodyHandle`] only; rapier's results never cross.

use glam::Vec3;

use super::BodyHandle;
use super::interaction::InteractionMask;

/// Which bodies a query sees — filtered in the pipeline, which skips rejects before testing;
/// post-filtering misses a body's second collider after discarding the first.
#[derive(Debug, Clone, Copy)]
pub struct QueryFilter {
    /// Skip this body and every collider it owns.
    ///
    /// What a character controller needs on every ground probe.
    pub exclude: Option<BodyHandle>,
    /// Groups the query belongs to and will interact with, read the same
    /// way a collider's own [`InteractionMask`] is.
    pub groups: InteractionMask,
    /// Skip overlap-only colliders; on by default, since a query stopping at a checkpoint is a bug.
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
    /// The cast began touching: `t` is zero and `normal` points out. Treat it as a contact and a
    /// controller digs in.
    pub penetrating: bool,
}

/// The nearest point on the nearest body to some point in space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointHit {
    /// Body the point projected onto.
    pub body: BodyHandle,
    /// World-space nearest point on it.
    pub point: Vec3,
    /// The point was inside the body; `point` is still the nearest surface, for pushing out.
    pub inside: bool,
}

/// A placed shape for queries — bundled so six positional arguments cannot swap two `Vec3`s
/// silently.
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
