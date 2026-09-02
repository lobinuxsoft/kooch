//! [`BoxGravity`] — a cube planet, pulling towards its nearest surface.

use glam::Vec3;

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// A solid box that pulls towards whichever part of its surface is
/// nearest: each face along its own normal, and the edges and corners
/// blending between them.
///
/// # How it works, and why the edges need no special case
///
/// The direction is towards the closest point on the box, which is the
/// point clamped into its half-extents. That single expression produces
/// every behaviour the shape needs:
///
/// - **Over a face**, the closest point is directly below, so the pull is
///   exactly that face's normal — constant across the whole face, which is
///   what makes it walkable.
/// - **Past an edge**, the closest point is *on* the edge, and the
///   direction rotates continuously as a body moves around it. The
///   transition between two faces is not blended, interpolated or
///   special-cased; it is what the arithmetic already says.
/// - **Past a corner**, it points at the corner, and the three faces meet
///   with no seam.
///
/// This is the gradient of the box's signed distance function, which is
/// why it comes out consistent: gravity that follows a surface *is* the
/// gradient of the distance to it.
///
/// # This is not [`super::AreaGravity`]
///
/// An area is a region with one uniform down, acting on what is inside it.
/// This is a solid acting on what is outside it, with a different direction
/// at every point. Same primitive, opposite job.
///
/// # The entity's scale does not resize this
///
/// `half_extents`, `rounding`, `range` and `falloff` are metres. A
/// field's space is rigid — rotation and translation only — so scaling
/// the entity places the source and nothing else. Resize a planet by
/// editing its extents.
///
/// # Default
///
/// A 10 m cube at Earth strength, with the corners slightly rounded and a
/// 20 m reach.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct BoxGravity {
    /// Half-extents of the solid, in metres.
    pub half_extents: Vec3,
    /// Acceleration at the surface, in metres per second squared.
    pub strength: f32,
    /// How gently gravity turns around the edges, in metres.
    ///
    /// The box is shrunk by this much before the closest point is taken,
    /// so the direction starts turning this far *before* the edge instead
    /// of at it. Zero is a hard cube; a value equal to the half-extents
    /// collapses the box to its centre and the field becomes a sphere —
    /// so this is the dial between a cube planet and a round one.
    pub rounding: f32,
    /// How far from the surface the field holds at full strength, in
    /// metres.
    ///
    /// Zero or less means unlimited, and `falloff` then never applies.
    pub range: f32,
    /// How far past `range` the field fades to nothing, in metres.
    ///
    /// Without it a body leaving the planet's reach loses its gravity
    /// between one step and the next. Zero for a hard cutoff.
    pub falloff: f32,
}

impl Default for BoxGravity {
    fn default() -> Self {
        Self {
            half_extents: Vec3::splat(5.0),
            strength: 9.81,
            rounding: 0.5,
            range: 20.0,
            falloff: 5.0,
        }
    }
}

impl Component for BoxGravity {}

impl BoxGravity {
    /// The acceleration this source applies at a point already expressed
    /// in the source's local space.
    pub fn acceleration_at_local(&self, local_point: Vec3) -> Vec3 {
        let Some((direction, distance)) = self.pull_at_local(local_point) else {
            return Vec3::ZERO;
        };
        direction * self.strength * self.influence(distance)
    }

    /// The direction towards the surface and the distance to it, or `None`
    /// for a point inside the solid.
    ///
    /// Split out because the direction alone is what a character controller
    /// wants — "which way is down from here" is a question worth asking
    /// without also applying a force.
    pub fn pull_at_local(&self, local_point: Vec3) -> Option<(Vec3, f32)> {
        // Shrinking by `rounding` and measuring from the shrunk box is the
        // whole of the rounded-box distance function. Clamped at zero so an
        // over-large rounding gives a sphere rather than an inside-out box.
        let half = (self.half_extents.abs() - Vec3::splat(self.rounding.max(0.0))).max(Vec3::ZERO);
        let offset = local_point.clamp(-half, half) - local_point;

        let reach = offset.length();
        // Inside the solid there is no surface to fall towards, and at the
        // exact centre no direction either. Both are the same answer.
        if reach <= self.rounding.max(0.0) {
            return None;
        }
        let direction = offset.try_normalize()?;
        Some((direction, reach - self.rounding.max(0.0)))
    }

    /// How strongly the field applies at a distance from the surface: 1 up
    /// to `range`, fading to 0 across `falloff`.
    pub fn influence(&self, distance: f32) -> f32 {
        if self.range <= 0.0 || distance <= self.range {
            return 1.0;
        }
        if self.falloff <= 0.0 {
            return 0.0;
        }
        (1.0 - (distance - self.range) / self.falloff).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests;
