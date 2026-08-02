//! What a camera follows, said by the thing being followed.
//!
//! A [`VirtualCamera`](crate::VirtualCamera) used to name its target with
//! an `EntityRef`. That is the wrong shape, and the bug it produced
//! (#712) was only the symptom: the reference pointed at an identity
//! nothing persisted, so authoring "follow the ball" in the editor and
//! reloading the scene gave a camera following nothing.
//!
//! # Why a tag is the right shape rather than a workaround
//!
//! **If more than one entity carries the tag, that is a group.**
//! Framing several subjects at once is a thing cameras have to do, and
//! with a reference it needs a second mechanism: Cinemachine has a
//! separate `CinemachineTargetGroup` object, phantom-camera has its own
//! group node. With a tag, "follow one" is the degenerate case of "frame
//! several" and there is one code path.
//!
//! Everything else falls out of a query being a query:
//!
//! | | Reference | Tag |
//! |---|---|---|
//! | Survives reload | needs persisted identity | nothing to persist |
//! | Target comes from a prefab | broken (#712) | a query does not care |
//! | Target spawns later | resolved to nothing, silently | found next frame |
//! | Player dies and respawns | dangling | the tag travels along |
//!
//! A query cannot dangle.

use glam::Vec3;
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Marks this entity as something a camera follows.
///
/// Attach it to the player, the boss, the cart — whatever the framing is
/// about. A [`VirtualCamera`](crate::VirtualCamera) follows the tagged
/// entities whose `group` matches its own.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
pub struct CameraTarget {
    /// Which framing this entity belongs to.
    ///
    /// A number rather than a reference to a group object, because the
    /// whole point is not to hold references. Group `0` is the default
    /// and is what a scene with one camera and one subject uses without
    /// thinking about it.
    pub group: u32,
    /// How much this member pulls, when several share a group.
    ///
    /// Relative, not absolute: two members at `1.0` and one at `2.0`
    /// frame the same as `0.5`/`0.5`/`1.0`. A member at `0.0`
    /// contributes nothing and is the way to keep a subject tagged while
    /// temporarily ignoring it.
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

/// The point a group of targets asks a camera to look at.
///
/// The weighted mean of their positions. With one member this is exactly
/// that member's position, which is what keeps the single-target case
/// byte-identical to the old `target` behaviour.
///
/// Returns `None` when the group is empty or every weight is zero —
/// "nothing to follow", which leaves the camera where it is rather than
/// snapping it to the origin.
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
mod tests {
    use super::*;

    #[test]
    fn one_member_is_exactly_its_own_position() {
        let position = Vec3::new(3.0, 4.0, 5.0);
        assert_eq!(weighted_centre(&[(position, 1.0)]), Some(position));
    }

    /// The single-target case must not shift when the weight is not 1.
    #[test]
    fn one_member_ignores_its_weight() {
        let position = Vec3::new(3.0, 4.0, 5.0);
        assert_eq!(weighted_centre(&[(position, 7.5)]), Some(position));
    }

    #[test]
    fn two_equal_members_meet_in_the_middle() {
        let centre = weighted_centre(&[(Vec3::ZERO, 1.0), (Vec3::new(10.0, 0.0, 0.0), 1.0)]);
        assert_eq!(centre, Some(Vec3::new(5.0, 0.0, 0.0)));
    }

    #[test]
    fn weights_are_relative_not_absolute() {
        let members = [(Vec3::ZERO, 1.0), (Vec3::new(9.0, 0.0, 0.0), 2.0)];
        let scaled = [(Vec3::ZERO, 10.0), (Vec3::new(9.0, 0.0, 0.0), 20.0)];
        assert_eq!(weighted_centre(&members), weighted_centre(&scaled));
    }

    #[test]
    fn a_heavier_member_pulls_the_centre_towards_itself() {
        let centre = weighted_centre(&[(Vec3::ZERO, 1.0), (Vec3::new(9.0, 0.0, 0.0), 2.0)])
            .expect("two positive weights");
        assert!(
            centre.x > 4.5,
            "the double-weighted member should pull past the midpoint; got {centre:?}"
        );
        assert_eq!(centre, Vec3::new(6.0, 0.0, 0.0));
    }

    #[test]
    fn an_empty_group_has_no_centre() {
        assert_eq!(weighted_centre(&[]), None);
    }

    /// Zero weight is "ignore me", and a group of only those is empty.
    #[test]
    fn a_group_of_zero_weights_has_no_centre() {
        assert_eq!(
            weighted_centre(&[(Vec3::ZERO, 0.0), (Vec3::ONE, 0.0)]),
            None
        );
    }

    /// A negative weight would drag the centre away from the subject and
    /// could cancel the total to zero, which reads as "no target".
    #[test]
    fn a_negative_weight_is_treated_as_zero() {
        let centre = weighted_centre(&[(Vec3::ZERO, 1.0), (Vec3::new(10.0, 0.0, 0.0), -5.0)]);
        assert_eq!(
            centre,
            Some(Vec3::ZERO),
            "a negative weight must not move the centre, nor void the group"
        );
    }
}
