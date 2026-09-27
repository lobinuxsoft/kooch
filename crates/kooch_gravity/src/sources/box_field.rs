//! [`BoxGravity`] — a cube planet, pulling towards its nearest surface.

use glam::Vec3;

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// A solid box pulling towards the nearest point of its surface — its SDF gradient — so faces are
/// walkable and edges turn with no special case.
/// Acts outside, unlike [`super::AreaGravity`]; metres, unscaled.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct BoxGravity {
    /// Half-extents of the solid, in metres.
    pub half_extents: Vec3,
    /// Acceleration at the surface, in metres per second squared.
    pub strength: f32,
    /// How gently gravity turns around the edges, in metres: the box shrinks by this before the
    /// nearest point is taken. Zero is a cube; the half-extents make a sphere.
    pub rounding: f32,
    /// How far past the solid the field holds at full strength, per **axis**, in metres — each
    /// component covering both of that axis's faces. Read as an absolute value, so a negative is
    /// the same reach the other way round.
    ///
    /// 🔴 All zero is unlimited, which is what a planet with no cutoff has always been. A zero on
    /// **one** axis is a reach of nothing on that axis: the field stops at those two faces. A field
    /// that differs face by face is an [`AreaGravity`](super::AreaGravity), not this (#1326).
    pub range: Vec3,
    /// The single reach a scene wrote before it was one per axis. Folded into `range` on load.
    #[reflect(hidden, alias = "range")]
    pub legacy_range: f32,
    /// How far past `range` the field fades to nothing, in metres, so leaving is not an instant
    /// loss. Zero for a hard cutoff.
    pub falloff: f32,
}

impl Default for BoxGravity {
    fn default() -> Self {
        Self {
            half_extents: Vec3::splat(5.0),
            strength: 9.81,
            rounding: 0.5,
            range: Vec3::splat(20.0),
            legacy_range: 0.0,
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
        direction * self.strength * self.influence_at_local(local_point, distance)
    }

    /// The direction towards the surface and the distance to it, or `None` only where the box has
    /// no size at all. The direction alone is what a controller asks.
    ///
    /// 🔴 Inside the solid the nearest point on the box **is** the point, so the gradient vanishes
    /// and the formula has nothing to answer. It keeps falling towards the nearest face instead —
    /// the same direction it had a step before crossing, so gravity does not switch off around
    /// anything that clips in or spawns there (#1324).
    pub fn pull_at_local(&self, local_point: Vec3) -> Option<(Vec3, f32)> {
        // Shrinking by `rounding` and measuring from the shrunk box is the
        // whole of the rounded-box distance function. Clamped at zero so an
        // over-large rounding gives a sphere rather than an inside-out box.
        let half = (self.half_extents.abs() - Vec3::splat(self.rounding.max(0.0))).max(Vec3::ZERO);
        let offset = local_point.clamp(-half, half) - local_point;

        let reach = offset.length();
        if reach > self.rounding.max(0.0) {
            let direction = offset.try_normalize()?;
            return Some((direction, reach - self.rounding.max(0.0)));
        }
        // Inside: carry on into the nearest face, which is the one with the smallest gap.
        let normal = self.inward_normal(local_point)?;
        Some((normal, 0.0))
    }

    /// The face a point inside the solid is under, as the inward normal: continuous across the
    /// surface, since just outside a face the pull is that same direction.
    fn inward_normal(&self, local_point: Vec3) -> Option<Vec3> {
        let half = self.half_extents.abs();
        let gap = half - local_point.abs();
        let axis = match (gap.x <= gap.y, gap.x <= gap.z, gap.y <= gap.z) {
            (true, true, _) => Vec3::X,
            (false, _, true) => Vec3::Y,
            _ => Vec3::Z,
        };
        // Away from the face it is under: just outside that face the pull points inwards, so just
        // inside it points the same way. The body keeps falling the way it was falling, and past
        // the middle the nearest face becomes the opposite one — it settles around the centre.
        let side = local_point.dot(axis);
        match side == 0.0 {
            true => None,
            false => Some(-axis * side.signum()),
        }
    }

    /// How far the field reaches at full strength past the axis `local_point` sits beyond — the
    /// axis with the largest overshoot, which is the pair of faces it is over.
    pub fn range_at_local(&self, local_point: Vec3) -> f32 {
        let out = local_point.abs() - self.half_extents.abs();
        let range = self.range.abs();
        match (out.x >= out.y, out.x >= out.z, out.y >= out.z) {
            (true, true, _) => range.x,
            (false, _, true) => range.y,
            _ => range.z,
        }
    }

    /// Whether the field reaches everywhere: no cutoff at all, which is what all-zero says.
    pub fn is_unlimited(&self) -> bool {
        self.range.abs().max_element() <= 0.0
    }

    /// How strongly the field applies at a point: 1 up to that axis's range, fading to 0 across
    /// `falloff`.
    pub fn influence_at_local(&self, local_point: Vec3, distance: f32) -> f32 {
        if self.is_unlimited() {
            return 1.0;
        }
        let range = self.range_at_local(local_point);
        if distance <= range {
            return 1.0;
        }
        if self.falloff <= 0.0 {
            return 0.0;
        }
        (1.0 - (distance - range) / self.falloff).clamp(0.0, 1.0)
    }
}

/// Folds the single `range` a scene wrote before it was one per axis into all three, and clears it.
///
/// 🔴 Said out loud: this writes to the author's data and a save makes it permanent. The mapping is
/// one number to three of the same number, so it cannot be misread — unlike a mask, which is how
/// the last migration wrote a planet into thirty-one layers (#1320).
pub fn migrate_box_range(resources: &mut kooch_core::resource::Resources) {
    let Some(registry) = resources.get_mut::<kooch_ecs::component::ComponentRegistry>() else {
        return;
    };
    let Some(storage) = registry.get_cpu_mut::<BoxGravity>() else {
        return;
    };
    for (&entity, field) in storage.iter_mut() {
        if field.legacy_range == 0.0 {
            continue;
        }
        let was = field.legacy_range;
        field.range = Vec3::splat(was);
        field.legacy_range = 0.0;
        tracing::info!(
            target: "kooch_gravity",
            entity = entity.index(),
            range = was,
            "a box field's reach was migrated to one per axis",
        );
    }
}

#[cfg(test)]
mod tests;
