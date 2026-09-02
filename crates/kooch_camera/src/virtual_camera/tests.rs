use super::*;

fn vcam(follow: u32) -> VirtualCamera {
    VirtualCamera {
        follow,
        damping: false,
        ..Default::default()
    }
}

/// A vcam cannot know whether anything carries its group — that is a
/// query over the world. So "nothing to follow" is enforced where the
/// pose is planned, and `plugin::nothing_tagged_means_nothing_to_follow`
/// is the test that guarantees it.
///
/// What `is_inert` still answers is the part a vcam knows alone.
#[test]
fn a_rig_with_both_modes_off_is_inert() {
    let mut r = VirtualCamera {
        follow: FOLLOW_NONE,
        look_at: LOOK_AT_NONE,
        ..Default::default()
    };
    assert!(
        r.is_inert(),
        "neither following nor looking is nothing to do"
    );
    r.follow = FOLLOW_SIMPLE;
    assert!(!r.is_inert());
}

/// The default must be usable the moment something is tagged — no
/// third step. If this ever needs one, the menu entry is lying.
#[test]
fn the_default_is_ready_to_work_the_moment_something_is_tagged() {
    let fresh = VirtualCamera::default();
    assert!(
        !fresh.is_inert(),
        "a default vcam should be waiting for a subject, not switched off"
    );
    assert_ne!(fresh.follow, FOLLOW_NONE);
    assert_ne!(fresh.look_at, LOOK_AT_NONE);
    assert_eq!(
        fresh.group, 0,
        "the default group is what a first tag lands in"
    );
}

#[test]
fn the_default_frames_a_subject_correctly() {
    let v = VirtualCamera::default();
    assert!(!v.is_inert());

    let target = Vec3::new(0.0, 0.0, -20.0);
    let (pos, rot) = v.desired(
        target,
        glam::Quat::IDENTITY,
        Vec3::ZERO,
        glam::Quat::IDENTITY,
        Vec3::Y,
    );
    assert!(
        (pos - target).length() > 0.1,
        "a third-person default should stand off its target, got {pos:?}",
    );
    let facing = rot * -Vec3::Z;
    assert!(
        facing.dot((target - pos).normalize()) > 0.999,
        "and it should be looking at it, facing {facing:?}",
    );
}

#[test]
fn simple_follow_is_the_target_plus_the_offset() {
    let mut r = vcam(FOLLOW_SIMPLE);
    r.offset = Vec3::new(0.0, 3.0, 10.0);
    let (pos, _) = r.desired(
        Vec3::new(5.0, 0.0, 0.0),
        glam::Quat::IDENTITY,
        Vec3::ZERO,
        glam::Quat::IDENTITY,
        Vec3::Y,
    );
    assert_eq!(pos, Vec3::new(5.0, 3.0, 10.0));
}

#[test]
fn the_spring_arm_keeps_its_length_at_every_yaw() {
    let mut r = vcam(FOLLOW_THIRD_PERSON);
    r.distance = 7.0;
    for yaw in [0.0, 37.0, 90.0, 180.0, -145.0] {
        r.yaw = yaw;
        let (pos, _) = r.desired(
            Vec3::ZERO,
            glam::Quat::IDENTITY,
            Vec3::ZERO,
            glam::Quat::IDENTITY,
            Vec3::Y,
        );
        assert!(
            (pos.length() - 7.0).abs() < 1e-4,
            "yaw {yaw} gave length {}",
            pos.length(),
        );
    }
}

/// Straight down is where a look-at basis degenerates and the image
/// rolls over. The clamp is what stops it.
#[test]
fn pitch_is_clamped_short_of_the_pole() {
    let mut r = vcam(FOLLOW_THIRD_PERSON);
    r.pitch = 90.0;
    let (pos, _) = r.desired(
        Vec3::ZERO,
        glam::Quat::IDENTITY,
        Vec3::ZERO,
        glam::Quat::IDENTITY,
        Vec3::Y,
    );
    assert!(
        Vec3::new(pos.x, 0.0, pos.z).length() > 1e-3,
        "a fully vertical arm leaves no horizontal basis: {pos:?}",
    );
}

/// Follow `None` with a look-at is a turret: it tracks and stays put.
#[test]
fn follow_none_leaves_the_position_alone() {
    let mut r = vcam(FOLLOW_NONE);
    r.look_at = LOOK_AT_SIMPLE;
    let here = Vec3::new(1.0, 2.0, 3.0);
    let (pos, _) = r.desired(
        Vec3::new(9.0, 0.0, 0.0),
        glam::Quat::IDENTITY,
        here,
        glam::Quat::IDENTITY,
        Vec3::Y,
    );
    assert_eq!(pos, here);
    assert!(!r.is_inert(), "look-at alone is still work to do");
}

/// The property that matters: the same elapsed time gives the same
/// result whatever the step size. A `lerp` with a constant factor
/// fails this, and that is the bug this replaces.
#[test]
fn damping_is_frame_rate_independent() {
    let mut r = VirtualCamera {
        damping: true,
        damping_value: Vec3::splat(0.25),
        ..Default::default()
    };
    r.follow = FOLLOW_SIMPLE;

    let desired = Vec3::new(10.0, 0.0, 0.0);
    let coarse = r.damped(Vec3::ZERO, desired, 1.0 / 30.0);

    let mut fine = Vec3::ZERO;
    for _ in 0..2 {
        fine = r.damped(fine, desired, 1.0 / 60.0);
    }

    assert!(
        (coarse.x - fine.x).abs() < 1e-3,
        "one 30 Hz step gave {} and two 60 Hz steps gave {}",
        coarse.x,
        fine.x,
    );
}

#[test]
fn damping_off_snaps_exactly() {
    let r = VirtualCamera {
        damping: false,
        ..Default::default()
    };
    let desired = Vec3::new(3.0, 4.0, 5.0);
    assert_eq!(r.damped(Vec3::ZERO, desired, 1.0 / 60.0), desired);
}

/// Zero on an axis is rigid on that axis, while its neighbours ease.
#[test]
fn a_zero_time_constant_is_rigid_on_that_axis_only() {
    let r = VirtualCamera {
        damping: true,
        damping_value: Vec3::new(0.0, 0.2, 0.2),
        ..Default::default()
    };
    let got = r.damped(Vec3::ZERO, Vec3::splat(10.0), 1.0 / 60.0);
    assert_eq!(got.x, 10.0, "x should be rigid");
    assert!(got.y < 10.0 && got.y > 0.0, "y should be easing: {}", got.y);
}

/// The test this file did not have, and the reason a mirrored basis
/// shipped: every rotation assertion here checked `is_finite()`, and
/// a reflection is perfectly finite. What a look-at owes you is that
/// the camera's forward axis actually points at the thing.
#[test]
fn look_at_points_the_camera_at_the_target() {
    let mut r = vcam(FOLLOW_NONE);
    r.look_at = LOOK_AT_SIMPLE;

    for (eye, target) in [
        (Vec3::ZERO, Vec3::new(0.0, 0.0, -10.0)),
        (Vec3::ZERO, Vec3::new(0.0, 0.0, 10.0)),
        (Vec3::new(3.0, 4.0, 5.0), Vec3::new(-2.0, 1.0, 8.0)),
        (Vec3::new(-7.0, 2.0, 0.0), Vec3::ZERO),
    ] {
        let (_, rot) = r.desired(
            target,
            glam::Quat::IDENTITY,
            eye,
            glam::Quat::IDENTITY,
            Vec3::Y,
        );
        // A camera looks down its own -Z.
        let forward = rot * -Vec3::Z;
        let want = (target - eye).normalize();
        assert!(
            forward.dot(want) > 0.9999,
            "from {eye:?} to {target:?}: facing {forward:?}, wanted {want:?}",
        );
    }
}

/// A mirrored basis also flips the horizon. Checking `up` catches a
/// roll of 180° that a forward-only assertion would let through.
#[test]
fn look_at_keeps_the_horizon_upright() {
    let mut r = vcam(FOLLOW_NONE);
    r.look_at = LOOK_AT_SIMPLE;
    let (_, rot) = r.desired(
        Vec3::new(0.0, 0.0, -10.0),
        glam::Quat::IDENTITY,
        Vec3::ZERO,
        glam::Quat::IDENTITY,
        Vec3::Y,
    );
    assert!(
        (rot * Vec3::Y).dot(Vec3::Y) > 0.0,
        "the camera is upside down: up is {:?}",
        rot * Vec3::Y,
    );
}

/// Looking straight at something along -Z is the identity rotation.
/// It was a reflection, which `is_finite()` happily accepted.
#[test]
fn the_canonical_look_at_is_the_identity() {
    let mut r = vcam(FOLLOW_NONE);
    r.look_at = LOOK_AT_SIMPLE;
    let (_, rot) = r.desired(
        Vec3::new(0.0, 0.0, -1.0),
        glam::Quat::IDENTITY,
        Vec3::ZERO,
        glam::Quat::IDENTITY,
        Vec3::Y,
    );
    assert!(
        rot.abs_diff_eq(glam::Quat::IDENTITY, 1e-5),
        "expected identity, got {rot:?}",
    );
}

/// `None` means leave it alone. It used to return the *target's*
/// rotation, so a follow-only vcam silently mimicked its target —
/// which is what `Mimic` is for.
#[test]
fn look_at_none_keeps_the_cameras_own_rotation() {
    let mut r = vcam(FOLLOW_SIMPLE);
    r.look_at = LOOK_AT_NONE;
    let mine = glam::Quat::from_rotation_y(0.7);
    let targets = glam::Quat::from_rotation_x(1.3);
    let (_, rot) = r.desired(Vec3::new(4.0, 0.0, 0.0), targets, Vec3::ZERO, mine, Vec3::Y);
    assert!(
        rot.abs_diff_eq(mine, 1e-6),
        "expected {mine:?}, got {rot:?}"
    );
}

/// An arbitrary up must not change what the old fixed-axis formula
/// did, or every scene authored before it would reframe itself.
#[test]
fn world_up_reproduces_the_old_fixed_axis_arm() {
    let mut r = vcam(FOLLOW_THIRD_PERSON);
    r.distance = 5.0;
    for (yaw, pitch) in [(0.0, 0.0), (30.0, 15.0), (-120.0, -40.0), (180.0, 60.0)] {
        r.yaw = yaw;
        r.pitch = pitch;
        let (pos, _) = r.desired(
            Vec3::ZERO,
            glam::Quat::IDENTITY,
            Vec3::ZERO,
            glam::Quat::IDENTITY,
            Vec3::Y,
        );
        let (sy, cy) = yaw.to_radians().sin_cos();
        let (sp, cp) = pitch.to_radians().clamp(-1.5533, 1.5533).sin_cos();
        let old = Vec3::new(sy * cp, sp, cy * cp) * 5.0;
        assert!(
            (pos - old).length() < 1e-4,
            "yaw {yaw} pitch {pitch}: got {pos:?}, old formula gave {old:?}",
        );
    }
}

/// The point of the whole feature: standing on the side of a planet,
/// the arm still sits on the local horizon and the camera still has
/// the local up over its head.
#[test]
fn the_arm_follows_an_arbitrary_up() {
    let mut r = vcam(FOLLOW_THIRD_PERSON);
    r.look_at = LOOK_AT_SIMPLE;
    r.distance = 4.0;
    r.pitch = 0.0;

    // Gravity pulling along -X means up is +X.
    let up = Vec3::X;
    let target = Vec3::new(10.0, 0.0, 0.0);
    let (pos, rot) = r.desired(
        target,
        glam::Quat::IDENTITY,
        Vec3::ZERO,
        glam::Quat::IDENTITY,
        up,
    );

    let arm = pos - target;
    assert!(
        (arm.length() - 4.0).abs() < 1e-4,
        "arm length {}",
        arm.length()
    );
    assert!(
        arm.dot(up).abs() < 1e-4,
        "a zero-pitch arm must lie on the horizon of up, got {arm:?}",
    );
    assert!(
        (rot * Vec3::Y).dot(up) > 0.99,
        "the camera's head should point along {up:?}, got {:?}",
        rot * Vec3::Y,
    );
}

/// Pitch is measured off the local horizon, not the world one.
#[test]
fn pitch_raises_the_arm_along_the_local_up() {
    let mut r = vcam(FOLLOW_THIRD_PERSON);
    r.distance = 3.0;
    r.pitch = 30.0;
    let up = Vec3::new(0.0, 0.0, 1.0);
    let (pos, _) = r.desired(
        Vec3::ZERO,
        glam::Quat::IDENTITY,
        Vec3::ZERO,
        glam::Quat::IDENTITY,
        up,
    );
    let expected_height = 3.0 * 30.0_f32.to_radians().sin();
    assert!(
        (pos.dot(up) - expected_height).abs() < 1e-4,
        "expected {expected_height} along up, got {}",
        pos.dot(up),
    );
}

/// `gravity_at` returns a zero vector where no field reaches, and a
/// normalised zero is `NaN` in every basis downstream.
#[test]
fn a_zero_up_falls_back_to_world_instead_of_nan() {
    let mut r = vcam(FOLLOW_THIRD_PERSON);
    r.look_at = LOOK_AT_SIMPLE;
    let (pos, rot) = r.desired(
        Vec3::ZERO,
        glam::Quat::IDENTITY,
        Vec3::ZERO,
        glam::Quat::IDENTITY,
        Vec3::ZERO,
    );
    assert!(pos.is_finite() && rot.is_finite(), "{pos:?} {rot:?}");
}

/// Crossing between gravity fields rotates the whole basis. Snapping
/// it throws the horizon over in one frame.
#[test]
fn rotation_damping_eases_instead_of_snapping() {
    let r = VirtualCamera {
        damping: true,
        rotation_damping_value: 0.2,
        ..Default::default()
    };
    let from = glam::Quat::IDENTITY;
    let to = glam::Quat::from_rotation_z(std::f32::consts::PI * 0.5);
    let step = r.damped_rotation(from, to, 1.0 / 60.0);
    assert!(step.angle_between(from) > 0.0, "it did not move");
    assert!(
        step.angle_between(to) > 0.0,
        "one 60 Hz step should not arrive",
    );

    // And it does arrive, given enough steps.
    let mut q = from;
    for _ in 0..600 {
        q = r.damped_rotation(q, to, 1.0 / 60.0);
    }
    assert!(q.angle_between(to) < 1e-3, "never converged: {q:?}");
}

/// A quaternion and its negation are the same rotation, so slerping
/// without picking the shorter arc rolls the horizon the long way.
#[test]
fn rotation_damping_takes_the_short_way_round() {
    let r = VirtualCamera {
        damping: true,
        rotation_damping_value: 0.2,
        ..Default::default()
    };
    let from = glam::Quat::IDENTITY;
    let to = -glam::Quat::from_rotation_y(0.2);
    let step = r.damped_rotation(from, to, 1.0 / 60.0);
    assert!(
        step.angle_between(from) < 0.2,
        "took the long arc: moved {} rad in one step",
        step.angle_between(from),
    );
}

#[test]
fn rotation_damping_off_snaps_exactly() {
    let r = VirtualCamera {
        damping: false,
        ..Default::default()
    };
    let to = glam::Quat::from_rotation_x(0.9);
    assert_eq!(r.damped_rotation(glam::Quat::IDENTITY, to, 1.0 / 60.0), to);
}

#[test]
fn a_disabled_rig_is_inert() {
    let mut r = vcam(FOLLOW_SIMPLE);
    assert!(!r.is_inert());
    r.enabled = false;
    assert!(r.is_inert(), "a switched-off vcam must not be a candidate");
}

#[test]
fn looking_at_where_you_already_are_is_not_a_nan() {
    let mut r = vcam(FOLLOW_GLUED);
    r.look_at = LOOK_AT_SIMPLE;
    let (_, rot) = r.desired(
        Vec3::splat(2.0),
        glam::Quat::IDENTITY,
        Vec3::ZERO,
        glam::Quat::IDENTITY,
        Vec3::Y,
    );
    assert!(rot.is_finite(), "a degenerate look-at produced {rot:?}");
}

/// A camera looking dead down is the other degenerate case, and it
/// has to stay finite rather than roll.
#[test]
fn looking_straight_down_stays_finite() {
    let mut r = vcam(FOLLOW_NONE);
    r.look_at = LOOK_AT_SIMPLE;
    let (_, rot) = r.desired(
        Vec3::ZERO,
        glam::Quat::IDENTITY,
        Vec3::new(0.0, 10.0, 0.0),
        glam::Quat::IDENTITY,
        Vec3::Y,
    );
    assert!(rot.is_finite(), "straight down produced {rot:?}");
}

/// The regression this whole mechanism exists for.
///
/// A yaw origin derived from `up` alone collapses where the world axis
/// it projects lines up with `up`, and the fallback axis is ninety
/// degrees away — so a target rolling over that one spot swung the
/// camera ninety degrees in and ninety back out. Carried instead, the
/// arm sweeps.
#[test]
fn rolling_over_the_pole_does_not_flip() {
    let vcam = VirtualCamera {
        follow: FOLLOW_THIRD_PERSON,
        distance: 5.0,
        pitch: 0.0,
        yaw: 0.0,
        damping: false,
        ..Default::default()
    };

    let mut up = Vec3::Y;
    let mut reference = seed_reference(up);
    let mut previous: Option<Vec3> = None;

    // A full half turn of the up axis through +Z, which is exactly where
    // the old construction swapped its reference axis.
    for step in 0..=180 {
        let angle = (step as f32).to_radians();
        let next = Vec3::new(0.0, angle.cos(), angle.sin());
        reference = transported(reference, up, next);
        up = next;

        let (position, _) = vcam.desired_with(
            Vec3::ZERO,
            glam::Quat::IDENTITY,
            Vec3::ZERO,
            glam::Quat::IDENTITY,
            up,
            reference,
        );
        if let Some(previous) = previous {
            // One degree of arc at five metres is under a tenth of a
            // metre. A flip is seven.
            assert!(
                (position - previous).length() < 0.5,
                "the arm jumped {} at {step} degrees",
                (position - previous).length(),
            );
        }
        previous = Some(position);
    }
}

#[test]
fn a_carried_reference_stays_flat() {
    let mut up = Vec3::Y;
    let mut reference = seed_reference(up);
    for step in 0..=90 {
        let angle = (step as f32).to_radians();
        let next = Vec3::new(angle.sin(), angle.cos(), 0.0);
        reference = transported(reference, up, next);
        up = next;
        assert!(reference.dot(up).abs() < 1e-4, "drifted off the horizon");
        assert!((reference.length() - 1.0).abs() < 1e-4);
    }
}

/// Reversed has no shortest arc, so there is no turn to apply. Spinning
/// through an arbitrary half turn is the flip, not the fix for it.
#[test]
fn a_reversed_up_keeps_its_reference() {
    let reference = Vec3::Z;
    let carried = transported(reference, Vec3::Y, Vec3::NEG_Y);
    assert!(carried.is_finite());
    assert!((carried - Vec3::Z).length() < 1e-5, "{carried}");
}

#[test]
fn an_unchanged_up_changes_nothing() {
    let reference = seed_reference(Vec3::Y);
    let carried = transported(reference, Vec3::Y, Vec3::Y);
    assert!((carried - reference).length() < 1e-5);
}
