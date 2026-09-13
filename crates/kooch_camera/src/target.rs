//! What a camera follows, said by the thing being followed: a tag, not a reference.
//! A query survives reload, prefabs and respawns and cannot dangle (#712); several tagged entities
//! are simply a group, so one code path frames one subject or many.

use glam::Vec3;
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Marks this entity as something a camera follows; a [`VirtualCamera`](crate::VirtualCamera)
/// follows the tagged entities in its `group`.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct CameraTarget {
    /// Which framing this entity belongs to — a number, not a reference. Group `0` is the default
    /// for a scene with one subject.
    pub group: u32,
    /// How much this member pulls within its group, relative to the others. `0.0` keeps a subject
    /// tagged but ignored.
    pub weight: f32,
}

impl Component for CameraTarget {}

impl Default for CameraTarget {
    fn default() -> Self {
        Self {
            group: 0,
            weight: 1.0,
        }
    }
}

/// The weighted mean of a group's positions — exactly one member's position when alone. `None` when
/// the group is empty or every weight is zero, leaving the camera in place.
pub fn weighted_centre(members: &[(Vec3, f32)]) -> Option<Vec3> {
    let total: f32 = members.iter().map(|(_, weight)| weight.max(0.0)).sum();
    if total <= 0.0 {
        return None;
    }
    let sum = members.iter().fold(Vec3::ZERO, |acc, (position, weight)| {
        acc + *position * weight.max(0.0)
    });
    Some(sum / total)
}

#[cfg(test)]
mod tests;
