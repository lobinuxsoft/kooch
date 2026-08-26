use super::*;
use glam::Vec3;
use slotmap::SlotMap;

fn desc(kind: JointKind) -> JointDesc {
    let mut bodies: SlotMap<BodyHandle, ()> = SlotMap::with_key();
    let (a, b) = (bodies.insert(()), bodies.insert(()));
    JointDesc::new(a, b, kind)
}

/// A hinge whose axis is mid-edit passes through zero, and a degenerate
/// joint frame produces NaNs that outlive the typo.
#[test]
fn a_degenerate_axis_falls_back_instead_of_producing_nans() {
    assert_eq!(safe_axis(Vec3::ZERO), Vec3::Y);
    assert!(safe_axis(Vec3::new(0.0, 0.0, 4.0)).abs_diff_eq(Vec3::Z, 1e-6));
}

/// Dragging a range in the Inspector puts the handles the wrong way
/// round, and rapier does not sort them itself.
#[test]
fn inverted_limits_are_ordered_before_reaching_the_solver() {
    let joint = generic_joint_for(&JointDesc {
        limits: Some([1.5, -0.5]),
        ..desc(JointKind::Revolute { axis: Vec3::Y })
    });
    let limits = joint.limits(JointAxis::AngX).expect("the axis is limited");
    assert_eq!((limits.min, limits.max), (-0.5, 1.5));
}

/// The kinds with no free axis must not silently accept a limit that
/// does nothing.
#[test]
fn the_kinds_without_a_free_axis_ignore_limits() {
    for kind in [
        JointKind::Fixed,
        JointKind::Rope { max_length: 2.0 },
        JointKind::Spring {
            rest_length: 1.0,
            stiffness: 10.0,
            damping: 1.0,
        },
    ] {
        assert!(primary_axis(&kind).is_none(), "{kind:?} has no free axis");
    }
}

#[test]
fn anchors_reach_the_built_joint() {
    let joint = generic_joint_for(&JointDesc {
        anchor_a: Vec3::new(1.0, 0.0, 0.0),
        anchor_b: Vec3::new(-1.0, 0.0, 0.0),
        ..desc(JointKind::Spherical)
    });
    assert_eq!(joint.local_anchor1(), Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(joint.local_anchor2(), Vec3::new(-1.0, 0.0, 0.0));
}

/// Two bodies welded at a joint overlap there; colliding by default
/// makes a door fight its own frame forever.
#[test]
fn contacts_between_jointed_bodies_are_off_unless_asked_for() {
    assert!(!generic_joint_for(&desc(JointKind::Fixed)).contacts_enabled());
    assert!(
        generic_joint_for(&JointDesc {
            contacts_enabled: true,
            ..desc(JointKind::Fixed)
        })
        .contacts_enabled()
    );
}

/// A motor left at its defaults contributes nothing, and configuring
/// one anyway would put an inert motor on the axis where an author
/// later looks for the real one.
#[test]
fn an_inert_motor_is_not_configured() {
    let joint = generic_joint_for(&JointDesc {
        motor: Some(JointMotor::default()),
        ..desc(JointKind::Revolute { axis: Vec3::Y })
    });
    assert!(
        joint.motor(JointAxis::AngX).is_none(),
        "an all-zero motor was still registered on the axis",
    );
}

#[test]
fn a_velocity_motor_reaches_the_built_joint() {
    let joint = generic_joint_for(&JointDesc {
        motor: Some(JointMotor {
            target_velocity: 4.0,
            damping: 2.0,
            ..Default::default()
        }),
        ..desc(JointKind::Revolute { axis: Vec3::Y })
    });
    let motor = joint.motor(JointAxis::AngX).expect("the axis is motorised");
    assert_eq!(motor.target_vel, 4.0);
    assert_eq!(motor.damping, 2.0);
}

/// Mixing newton-seconds with newton-metre-seconds gives a threshold
/// nobody can author against.
#[test]
fn the_break_measure_ignores_the_torque_components() {
    assert_eq!(linear_impulse(&[3.0, 4.0, 0.0, 100.0, 100.0, 100.0]), 5.0);
}
