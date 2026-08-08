use super::*;
use slotmap::SlotMap;

fn two_handles() -> (BodyHandle, BodyHandle) {
    let mut bodies: SlotMap<BodyHandle, ()> = SlotMap::with_key();
    (bodies.insert(()), bodies.insert(()))
}

#[test]
fn a_new_joint_is_unlimited_unmotorised_and_unbreakable() {
    let (a, b) = two_handles();
    let desc = JointDesc::new(a, b, JointKind::Fixed);
    assert_eq!(desc.limits, None);
    assert_eq!(desc.motor, None);
    assert!(!desc.articulated);
    assert!(!desc.break_impulse.is_finite());
}

/// Two bodies welded at a joint overlap there. Colliding by default
/// would make every door fight its own frame.
#[test]
fn jointed_bodies_do_not_collide_by_default() {
    let (a, b) = two_handles();
    assert!(!JointDesc::new(a, b, JointKind::Fixed).contacts_enabled);
}

/// The kinds with nothing to limit must say so, or the Inspector shows
/// a limit range that silently does nothing.
#[test]
fn only_the_kinds_with_a_free_axis_read_limits_and_motors() {
    let (a, b) = two_handles();
    let with = [
        JointKind::Revolute { axis: Vec3::Y },
        JointKind::Prismatic { axis: Vec3::X },
        JointKind::Spherical,
        JointKind::PinSlot { axis: Vec3::X },
        JointKind::Generic { locked_axes: 0 },
    ];
    let without = [
        JointKind::Fixed,
        JointKind::Rope { max_length: 1.0 },
        JointKind::Spring {
            rest_length: 1.0,
            stiffness: 1.0,
            damping: 0.1,
        },
    ];
    for kind in with {
        assert!(
            JointDesc::new(a, b, kind).has_primary_axis(),
            "{kind:?} should read limits and motors",
        );
    }
    for kind in without {
        assert!(
            !JointDesc::new(a, b, kind).has_primary_axis(),
            "{kind:?} has no axis to limit or drive",
        );
    }
}

/// A motor left at its defaults contributes nothing to the solve, and
/// the difference between "off" and "on but inert" is a bug report.
#[test]
fn a_motor_with_no_coefficients_does_nothing() {
    assert!(!JointMotor::default().is_effective());
    assert!(
        JointMotor {
            damping: 1.0,
            ..Default::default()
        }
        .is_effective()
    );
    assert!(
        JointMotor {
            stiffness: 1.0,
            ..Default::default()
        }
        .is_effective()
    );
}
