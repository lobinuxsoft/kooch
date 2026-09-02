//! [`PlaneGravity`] — one-sided uniform pull towards an infinite plane.

use glam::Vec3;

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// A floor of unbounded extent: everything above it falls towards it, along
/// one direction, however far to the side it is.
///
/// The plane passes through the entity and rotates with it, so `normal` is
/// given in local space and turning the entity tips the floor.
///
/// # Why this is not [`super::GlobalGravity`]
///
/// A global field is uniform *everywhere* — it has no position, so nothing
/// it does depends on where a body is. This one has a surface: it acts on
/// one side only, and it stops at a distance. That is what makes it a floor
/// rather than a direction, and it is why a body launched off the top of a
/// level can leave its pull instead of being drawn back forever.
///
/// # Why it is not [`super::AreaGravity`]
///
/// An area is a box, and a box has sides. This is bounded in one axis and
/// unbounded in the other two, which is what a level floor is. Reaching it
/// with an area means half-extents large enough to be a lie, and the box
/// still acts from underneath.
///
/// # The entity's scale does not resize this
///
/// `range` and `falloff` are heights in metres. Turning the entity tips
/// the floor; scaling it does nothing.
///
/// # Default
///
/// Earth-strength, pulling down, holding for 50 m above the plane and
/// fading over the next 10.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct PlaneGravity {
    /// Which way is up out of the plane, in the entity's local space.
    ///
    /// The pull is along its negation. Stated as up rather than as down so
    /// it matches the plane's own normal, which is the number an author
    /// reads off a surface.
    pub normal: Vec3,
    /// Acceleration in metres per second squared.
    pub strength: f32,
    /// How far above the plane the field holds at full strength, in
    /// metres.
    ///
    /// Zero or less means unlimited, and `falloff` then never applies.
    pub range: f32,
    /// How far past `range` the field fades to nothing, in metres.
    ///
    /// Without it a body leaving the floor's reach loses its gravity between
    /// one step and the next. Zero for a hard cutoff.
    pub falloff: f32,
}

impl Default for PlaneGravity {
    fn default() -> Self {
        Self {
            normal: Vec3::Y,
            strength: 9.81,
            range: 50.0,
            falloff: 10.0,
        }
    }
}

impl Component for PlaneGravity {}

impl PlaneGravity {
    /// The acceleration this source applies at a point already expressed in
    /// the source's local space.
    pub fn acceleration_at_local(&self, local_point: Vec3) -> Vec3 {
        let Some(normal) = self.normal.try_normalize() else {
            return Vec3::ZERO;
        };
        -normal * self.strength * self.influence_at_local(local_point)
    }

    /// How strongly the field applies at a local point: 1 above the plane
    /// out to `range`, fading to 0 across `falloff`, and 0 below.
    ///
    /// Nothing underneath, because a plane with no thickness that pulled
    /// from both sides would trap a body in it — pushed back towards the
    /// surface from whichever side it reached, with no way out.
    pub fn influence_at_local(&self, local_point: Vec3) -> f32 {
        let Some(normal) = self.normal.try_normalize() else {
            return 0.0;
        };
        let height = local_point.dot(normal);
        if height < 0.0 {
            return 0.0;
        }
        if self.range <= 0.0 || height <= self.range {
            return 1.0;
        }
        if self.falloff <= 0.0 {
            return 0.0;
        }
        (1.0 - (height - self.range) / self.falloff).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests;
