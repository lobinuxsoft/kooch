use super::*;

fn vcam(_follow: u32) -> VirtualCamera {
    VirtualCamera {
        damping: Vec3::ZERO,
        rotation_damping: 0.0,
        ..Default::default()
    }
}

/// The shoulder body a vcam names, with nothing authored — every test that wants one starts here.
fn shoulder() -> crate::ThirdPersonFollow {
    crate::ThirdPersonFollow {
        shoulder_offset: Vec3::ZERO,
        vertical_arm_length: 0.0,
        camera_side: 1.0,
        // The arm the tests were written against, and the orbital body's radius through `desired`.
        camera_distance: 6.0,
    }
}

/// 🔴 #1397: "has this vcam anything to do" is asked of the **components**, because carrying one is
/// what says it does. A field that said so a second time could disagree with them, and did.
#[test]
fn a_rig_with_no_components_is_inert() {
    use kooch_ecs::component::ComponentRegistry;

    let entity = kooch_ecs::entity::Entity::new(1, 0);
    let mut registry = ComponentRegistry::new();
    registry.register_cpu::<crate::HardLockToTarget>();
    let r = VirtualCamera::default();
    assert!(
        r.is_inert(&registry, entity),
        "neither following nor looking is nothing to do",
    );

    registry
        .get_cpu_mut::<crate::HardLockToTarget>()
        .unwrap()
        .insert(entity, crate::HardLockToTarget);
    assert!(!r.is_inert(&registry, entity));
}

/// 🔴 The "Add Component" menu's promise, and what #1397 changed about it: a bare `VirtualCamera`
/// is **not** ready to work, because what it does is the components beside it. The menu entry that
/// is ready is a body, and that is the thing to check.
#[test]
fn a_body_is_what_makes_it_ready() {
    use kooch_ecs::component::ComponentRegistry;

    let entity = kooch_ecs::entity::Entity::new(1, 0);
    let mut registry = ComponentRegistry::new();
    registry.register_cpu::<crate::OrbitalFollow>();
    registry
        .get_cpu_mut::<crate::OrbitalFollow>()
        .unwrap()
        .insert(entity, crate::OrbitalFollow::default());

    let fresh = VirtualCamera::default();
    assert!(
        !fresh.is_inert(&registry, entity),
        "a vcam with a body should be waiting for a subject, not switched off",
    );
    assert_eq!(
        fresh.group, 0,
        "the default group is what a first tag lands in"
    );
    // And the body's own default places a camera without a second step.
    assert!(crate::OrbitalFollow::default().radius > 0.0);
}

/// Both quantities at once, seeding the yaw origin as a first step does — what the rig's Body and
/// Aim stages each do one of.
/// 🔴 One bag of numbers for either body. A test cares about the geometry, not about which component
/// holds it, so the helper hands `body` to whichever the vcam names (#1391).
/// 🔴 The modes are components now (#1397), so a test says which by handing them in: the body is
/// read off `body` — a shoulder authored means the shoulder rig, a bare distance means the sphere —
/// and the aim is named, because that is the one a test usually cares about.
fn desired(
    vcam: &VirtualCamera,
    body: Option<crate::ThirdPersonFollow>,
    aim: u32,
    target: Vec3,
    target_rot: glam::Quat,
    current: Vec3,
    current_rot: glam::Quat,
    up: Vec3,
) -> (Vec3, glam::Quat) {
    let up = super::normalised_up(up);
    let reference = seed_reference(up);
    // A body is a component now (#1391), so a test that wants one names it. These are the defaults;
    // a test with numbers of its own calls `on_sphere` or `on_shoulder` directly.
    // No body is no body: the position is whatever else wrote it, which is what the rig does when
    // the entity carries none (#1397).
    let position = match body {
        None => current,
        Some(body) if body.shoulder_offset != Vec3::ZERO || body.vertical_arm_length != 0.0 => {
            vcam.on_shoulder(target, body, up, reference)
        }
        Some(body) => vcam.on_sphere(
            target,
            crate::OrbitalFollow {
                radius: body.camera_distance,
                ..Default::default()
            },
            0.5,
            up,
            reference,
        ),
    };
    let rotation = match aim {
        LOOK_AT_SIMPLE => super::look_at(position, target, up, reference),
        LOOK_AT_ARM => {
            let along = vcam.along(vcam.back(up, reference), up);
            super::look_at(position, position - along, up, reference)
        }
        LOOK_AT_MIMIC => target_rot,
        _ => current_rot,
    };
    (position, rotation)
}

#[test]
fn the_default_frames_a_subject_correctly() {
    let v = VirtualCamera::default();

    let target = Vec3::new(0.0, 0.0, -20.0);
    let (pos, rot) = desired(
        &v,
        Some(shoulder()),
        LOOK_AT_SIMPLE,
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
    let follow = crate::Follow {
        offset: Vec3::new(0.0, 3.0, 10.0),
    };
    let target = Vec3::new(4.0, 0.0, -2.0);
    assert_eq!(target + follow.offset, Vec3::new(4.0, 3.0, 8.0));
}

#[test]
fn the_spring_arm_keeps_its_length_at_every_yaw() {
    let (mut r, mut body) = (vcam(FOLLOW_ORBITAL), shoulder());
    body.camera_distance = 7.0;
    for yaw in [0.0, 37.0, 90.0, 180.0, -145.0] {
        r.yaw = yaw;
        let (pos, _) = desired(
            &r,
            Some(body),
            LOOK_AT_SIMPLE,
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
    let (mut r, body) = (vcam(FOLLOW_ORBITAL), shoulder());
    r.pitch = 90.0;
    let (pos, _) = desired(
        &r,
        Some(body),
        LOOK_AT_SIMPLE,
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
    let r = vcam(FOLLOW_NONE);
    let here = Vec3::new(1.0, 2.0, 3.0);
    let (pos, _) = desired(
        &r,
        None,
        LOOK_AT_SIMPLE,
        Vec3::new(9.0, 0.0, 0.0),
        glam::Quat::IDENTITY,
        here,
        glam::Quat::IDENTITY,
        Vec3::Y,
    );
    assert_eq!(pos, here);
}

/// Steps the position damping towards a still `desired` for `seconds` at `fps`.
fn damped_for(r: &VirtualCamera, desired: Vec3, fps: f32, seconds: f32) -> Vec3 {
    let mut at = Vec3::ZERO;
    for _ in 0..(seconds * fps).round() as usize {
        at = r.damped(at, desired, 1.0 / fps);
    }
    at
}

/// The damping time is when the camera arrives — exactly, at 30 fps or 144 — and not before.
#[test]
fn damping_leaves_a_hundredth() {
    let r = VirtualCamera {
        damping: Vec3::splat(0.5),
        ..Default::default()
    };
    let desired = Vec3::new(10.0, -2.0, 4.0);
    // 🔴 A hundredth of the gap after `damping` seconds, at any frame rate — Cinemachine's
    // contract, and not an exact arrival. An exact arrival needs a tween that restarts when its
    // goal moves, and that restart is what stepped whenever the target started or stopped (#1336).
    for fps in [30.0_f32, 60.0, 144.0] {
        let left = (desired - damped_for(&r, desired, fps, 0.5)).length() / desired.length();
        assert!(
            (left - 0.01).abs() < 2e-3,
            "{fps} fps left {left} of the gap, wanted a hundredth",
        );
        let early = (desired - damped_for(&r, desired, fps, 0.25)).length() / desired.length();
        assert!(early > 0.05, "{fps} fps closed too much too soon: {early}");
    }
}

#[test]
fn damping_off_snaps_exactly() {
    let r = VirtualCamera {
        damping: Vec3::ZERO,
        rotation_damping: 0.0,
        ..Default::default()
    };
    let desired = Vec3::new(3.0, 4.0, 5.0);
    assert_eq!(r.damped(Vec3::ZERO, desired, 1.0 / 60.0), desired);
}

/// Zero on an axis is rigid on that axis, while its neighbours ease.
#[test]
fn a_zero_time_is_rigid_on_that_axis_only() {
    let r = VirtualCamera {
        damping: Vec3::new(0.0, 0.2, 0.2),
        ..Default::default()
    };
    let got = r.damped(Vec3::ZERO, Vec3::splat(10.0), 1.0 / 60.0);
    assert_eq!(got.x, 10.0, "x should be rigid");
    assert!(got.y < 10.0 && got.y > 0.0, "y should be easing: {}", got.y);
}

/// A look-at's forward axis must point at the target — `is_finite()` alone let a mirrored basis
/// ship.
#[test]
fn look_at_points_the_camera_at_the_target() {
    let r = vcam(FOLLOW_NONE);

    for (eye, target) in [
        (Vec3::ZERO, Vec3::new(0.0, 0.0, -10.0)),
        (Vec3::ZERO, Vec3::new(0.0, 0.0, 10.0)),
        (Vec3::new(3.0, 4.0, 5.0), Vec3::new(-2.0, 1.0, 8.0)),
        (Vec3::new(-7.0, 2.0, 0.0), Vec3::ZERO),
    ] {
        let (_, rot) = desired(
            &r,
            None,
            LOOK_AT_SIMPLE,
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
    let (r, body) = (vcam(FOLLOW_NONE), shoulder());
    let (_, rot) = desired(
        &r,
        Some(body),
        LOOK_AT_SIMPLE,
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
    let r = vcam(FOLLOW_NONE);
    let (_, rot) = desired(
        &r,
        None,
        LOOK_AT_SIMPLE,
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
    let (r, body) = (vcam(FOLLOW_SIMPLE), shoulder());
    let mine = glam::Quat::from_rotation_y(0.7);
    let targets = glam::Quat::from_rotation_x(1.3);
    let (_, rot) = desired(
        &r,
        Some(body),
        LOOK_AT_NONE,
        Vec3::new(4.0, 0.0, 0.0),
        targets,
        Vec3::ZERO,
        mine,
        Vec3::Y,
    );
    assert!(
        rot.abs_diff_eq(mine, 1e-6),
        "expected {mine:?}, got {rot:?}"
    );
}

/// An arbitrary up must not change what the old fixed-axis formula
/// did, or every scene authored before it would reframe itself.
#[test]
fn world_up_reproduces_the_old_fixed_axis_arm() {
    let (mut r, mut body) = (vcam(FOLLOW_ORBITAL), shoulder());
    body.camera_distance = 5.0;
    for (yaw, pitch) in [(0.0, 0.0), (30.0, 15.0), (-120.0, -40.0), (180.0, 60.0)] {
        r.yaw = yaw;
        r.pitch = pitch;
        let (pos, _) = desired(
            &r,
            Some(body),
            LOOK_AT_SIMPLE,
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
    let (mut r, mut body) = (vcam(FOLLOW_ORBITAL), shoulder());
    body.camera_distance = 4.0;
    r.pitch = 0.0;

    // Gravity pulling along -X means up is +X.
    let up = Vec3::X;
    let target = Vec3::new(10.0, 0.0, 0.0);
    let (pos, rot) = desired(
        &r,
        Some(body),
        LOOK_AT_SIMPLE,
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
    let (mut r, mut body) = (vcam(FOLLOW_ORBITAL), shoulder());
    body.camera_distance = 3.0;
    r.pitch = 30.0;
    let up = Vec3::new(0.0, 0.0, 1.0);
    let (pos, _) = desired(
        &r,
        Some(body),
        LOOK_AT_SIMPLE,
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
    let (r, body) = (vcam(FOLLOW_ORBITAL), shoulder());
    let (pos, rot) = desired(
        &r,
        Some(body),
        LOOK_AT_SIMPLE,
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
fn rotation_damping_eases_instead_of_snapping_() {
    let r = VirtualCamera {
        rotation_damping: 0.2,
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

    // And after its seconds a hundredth of the angle is left — the easing's contract, not an
    // arrival (#1336). 0.2 s is 12 steps, one taken above.
    let mut q = step;
    for _ in 0..11 {
        q = r.damped_rotation(q, to, 1.0 / 60.0);
    }
    let left = q.angle_between(to) / from.angle_between(to);
    assert!(
        (left - 0.01).abs() < 5e-3,
        "left {left} of the angle, wanted a hundredth",
    );
}

/// A quaternion and its negation are the same rotation, so slerping
/// without picking the shorter arc rolls the horizon the long way.
#[test]
fn rotation_damping_takes_the_short_way_round() {
    let r = VirtualCamera {
        rotation_damping: 0.2,
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
        damping: Vec3::ZERO,
        rotation_damping: 0.0,
        ..Default::default()
    };
    let to = glam::Quat::from_rotation_x(0.9);
    assert_eq!(r.damped_rotation(glam::Quat::IDENTITY, to, 1.0 / 60.0), to);
}

#[test]
fn a_disabled_rig_is_inert() {
    use kooch_ecs::component::ComponentRegistry;

    let entity = kooch_ecs::entity::Entity::new(1, 0);
    let mut registry = ComponentRegistry::new();
    registry.register_cpu::<crate::HardLockToTarget>();
    registry
        .get_cpu_mut::<crate::HardLockToTarget>()
        .unwrap()
        .insert(entity, crate::HardLockToTarget);

    let mut r = VirtualCamera::default();
    assert!(!r.is_inert(&registry, entity));
    r.enabled = false;
    assert!(
        r.is_inert(&registry, entity),
        "a switched-off vcam must not be a candidate",
    );
}

#[test]
fn looking_at_where_you_already_are_is_not_a_nan() {
    let (r, body) = (vcam(FOLLOW_GLUED), shoulder());
    let (_, rot) = desired(
        &r,
        Some(body),
        LOOK_AT_SIMPLE,
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
    let (r, body) = (vcam(FOLLOW_NONE), shoulder());
    let (_, rot) = desired(
        &r,
        Some(body),
        LOOK_AT_SIMPLE,
        Vec3::ZERO,
        glam::Quat::IDENTITY,
        Vec3::new(0.0, 10.0, 0.0),
        glam::Quat::IDENTITY,
        Vec3::Y,
    );
    assert!(rot.is_finite(), "straight down produced {rot:?}");
}

/// A yaw origin derived from `up` swung the camera ninety degrees in and out at one spot on every
/// planet; carried, the arm sweeps.
#[test]
fn rolling_over_the_pole_does_not_flip() {
    let vcam = VirtualCamera {
        pitch: 0.0,
        yaw: 0.0,
        damping: Vec3::ZERO,
        rotation_damping: 0.0,
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

        let position = vcam.on_sphere(
            Vec3::ZERO,
            crate::OrbitalFollow {
                radius: 5.0,
                ..Default::default()
            },
            0.5,
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

/// Scenes saved under `blend_time` keep their blend.
#[test]
fn blend_time_still_loads() {
    let mut vcam = VirtualCamera::default();
    vcam.reflect_set("blend_time", kooch_ecs::reflect::ReflectValue::F32(0.1))
        .unwrap();
    assert_eq!(vcam.blend_duration, 0.1);
}

/// And so do the damping times.
#[test]
fn damping_value_still_loads() {
    let mut vcam = VirtualCamera::default();
    vcam.reflect_set(
        "damping",
        kooch_ecs::reflect::ReflectValue::Vec3(Vec3::splat(0.3)),
    )
    .unwrap();
    vcam.reflect_set(
        "rotation_damping",
        kooch_ecs::reflect::ReflectValue::F32(0.2),
    )
    .unwrap();
    assert_eq!(vcam.damping, Vec3::splat(0.3));
    assert_eq!(vcam.rotation_damping, 0.2);
}

/// Every name a camera duration was saved under still loads.
#[test]
fn old_duration_names_load() {
    use kooch_ecs::reflect::ReflectValue::F32;
    let mut vcam = VirtualCamera::default();
    vcam.reflect_set(
        "damping_time",
        kooch_ecs::reflect::ReflectValue::Vec3(Vec3::splat(0.4)),
    )
    .unwrap();
    vcam.reflect_set("rotation_damping_time", F32(0.4)).unwrap();
    assert_eq!(vcam.damping, Vec3::splat(0.4));
    assert_eq!(vcam.rotation_damping, 0.4);

    // 🔴 The framing's changed shape as well as name: a scalar cannot land in a `Vec2`, so the old
    // field is still here and a migration empties it (#1367).
    let mut framing = crate::RotationComposer::default();
    framing.reflect_set("soft_time", F32(0.4)).unwrap();
    assert_eq!(framing.was_soft_time, 0.4);

    let mut collision = crate::Deoccluder::default();
    collision.reflect_set("return_time", F32(0.4)).unwrap();
    assert_eq!(collision.damping, 0.4);
    collision.reflect_set("return_duration", F32(0.6)).unwrap();
    assert_eq!(collision.damping, 0.6);
}

/// The whole point of the offset: the camera stands beside the arm, along the axis it would call
/// right, and not further back or higher up.
#[test]
fn a_shoulder_stands_beside_the_arm() {
    let (mut r, mut body) = (vcam(FOLLOW_SHOULDER), shoulder());
    body.camera_distance = 3.0;
    r.pitch = 0.0;
    r.yaw = 40.0;

    let target = Vec3::ZERO;
    let (centred, _) = desired(
        &r,
        Some(body),
        LOOK_AT_ARM,
        target,
        glam::Quat::IDENTITY,
        Vec3::ZERO,
        glam::Quat::IDENTITY,
        Vec3::Y,
    );
    body.shoulder_offset = Vec3::new(0.6, 0.0, 0.0);
    let (beside, rot) = desired(
        &r,
        Some(body),
        LOOK_AT_ARM,
        target,
        glam::Quat::IDENTITY,
        Vec3::ZERO,
        glam::Quat::IDENTITY,
        Vec3::Y,
    );

    let moved = beside - centred;
    assert!(
        (moved.length() - 0.6).abs() < 1e-4,
        "an offset of 0.6 should move the camera 0.6, got {}",
        moved.length()
    );
    assert!(
        (rot * Vec3::X).dot(moved.normalize()) > 0.999,
        "it should move along the camera's own right, got {moved:?}",
    );
}

/// Pitch turns the reach and nothing else. A shoulder that pitched with it would swing under the
/// character as the player looks down, which is what a single pivot did.
#[test]
fn pitch_leaves_the_shoulder_alone() {
    let (mut r, mut body) = (vcam(FOLLOW_SHOULDER), shoulder());
    body.camera_distance = 3.0;
    body.vertical_arm_length = 1.0;
    r.yaw = 25.0;
    body.shoulder_offset = Vec3::new(0.5, 0.3, -0.2);

    let up = Vec3::Y;
    let reference = seed_reference(up);
    // 🔴 Through the whole chain, not through `shouldered` alone: that lives on the body now and
    // takes no pitch, so calling it in a loop over pitches asks it nothing and passes for free. The
    // compiler said so — "value assigned to `r` is never read" — before anyone did.
    let (_, first, _) = r.rig_positions(Vec3::ZERO, up, reference, body);
    for pitch in [-60.0, 0.0, 35.0, 80.0] {
        r.pitch = pitch;
        let (_, shoulder, hand) = r.rig_positions(Vec3::ZERO, up, reference, body);
        assert!(
            (shoulder - first).length() < 1e-4,
            "pitch {pitch} moved the shoulder to {shoulder:?}",
        );
        assert!(
            (hand - shoulder).length() > 0.0,
            "the hand should still rise off it",
        );
    }
}

/// 🔴 #1365: the hand is **not** the shoulder. It rises along the view's own up, which pitches, so
/// looking down swings it forward and down while the shoulder stays — and that is what holds the
/// target still on screen as the view turns vertically. #1359 dropped the field believing the two
/// were one axis.
#[test]
fn the_arm_rises_with_the_pitch() {
    let (mut r, mut body) = (vcam(FOLLOW_SHOULDER), shoulder());
    body.camera_distance = 0.0;
    r.yaw = 0.0;
    r.pitch = 0.0;
    body.vertical_arm_length = 1.0;

    let up = Vec3::Y;
    let reference = seed_reference(up);
    let level = r.on_shoulder(Vec3::ZERO, body, up, reference);
    assert!(
        level.abs_diff_eq(Vec3::Y, 1e-4),
        "a level arm rises straight up: {level:?}",
    );

    r.pitch = 90.0;
    let pitched = r.on_shoulder(Vec3::ZERO, body, up, reference);
    // Looking straight down, the view's up is the way it came from — the arm's own `back`.
    assert!(
        pitched.dot(up).abs() < 0.05 && pitched.dot(r.back(up, reference)) < -0.9,
        "the arm did not pitch with the view: {pitched:?}",
    );
}

/// `side` picks the shoulder without touching the offset that says how far off it sits.
#[test]
fn the_side_mirrors_the_shoulder() {
    let (mut r, mut body) = (vcam(FOLLOW_SHOULDER), shoulder());
    body.camera_distance = 0.0;
    r.pitch = 0.0;
    body.shoulder_offset = Vec3::new(0.6, 0.0, 0.0);

    let up = Vec3::Y;
    let reference = seed_reference(up);
    let at = |r: &VirtualCamera, body| r.on_shoulder(Vec3::ZERO, body, up, reference);

    let right = at(&r, body);
    body.camera_side = 0.0;
    let left = at(&r, body);
    body.camera_side = 0.5;
    let centred = at(&r, body);

    assert!((right + left).length() < 1e-4, "{right:?} and {left:?}");
    assert!(centred.length() < 1e-4, "halfway is neither: {centred:?}");
    assert!((right.length() - 0.6).abs() < 1e-4, "{right:?}");
}

/// Without this the offset is invisible: `Simple` turns to put the target back in the middle, so
/// only the parallax shifts and the character never leaves centre screen.
#[test]
fn an_arm_aim_holds_the_target_off_centre() {
    let (r, mut body) = (vcam(FOLLOW_SHOULDER), shoulder());
    body.camera_distance = 3.0;
    body.shoulder_offset = Vec3::new(0.6, 0.0, 0.0);

    let target = Vec3::ZERO;
    let off_centre = |vcam: &VirtualCamera, aim: u32| {
        let (pos, rot) = desired(
            vcam,
            Some(body),
            aim,
            target,
            glam::Quat::IDENTITY,
            Vec3::ZERO,
            glam::Quat::IDENTITY,
            Vec3::Y,
        );
        // How far off the view axis the target sits, in the camera's right.
        (target - pos).normalize().dot(rot * Vec3::X)
    };
    assert!(
        off_centre(&r, LOOK_AT_SIMPLE).abs() < 1e-4,
        "`Simple` aims at the target, so it is centred whatever the body did"
    );
    assert!(
        off_centre(&r, LOOK_AT_ARM) < -0.15,
        "a right shoulder puts the character left of the view axis, got {}",
        off_centre(&r, LOOK_AT_ARM)
    );
}

/// The aim is a direction, not the pivot, so it says something even where there is no arm to
/// measure — a `look_at` between two coincident points returns identity.
#[test]
fn a_zero_arm_still_aims() {
    let (mut r, mut body) = (vcam(FOLLOW_ORBITAL), shoulder());
    body.camera_distance = 0.0;
    r.pitch = 0.0;
    r.yaw = 90.0;

    let (_, rot) = desired(
        &r,
        Some(body),
        LOOK_AT_ARM,
        Vec3::ZERO,
        glam::Quat::IDENTITY,
        Vec3::ZERO,
        glam::Quat::IDENTITY,
        Vec3::Y,
    );
    assert_ne!(rot, glam::Quat::IDENTITY, "a yaw of 90° is not no rotation");
    assert!(
        (rot * Vec3::NEG_Z).dot(Vec3::X) < -0.99,
        "yaw 90° looks along -X, got {:?}",
        rot * Vec3::NEG_Z
    );
}

/// 🔴 #1379: the gizmo, the Inspector and this all want the same three points. Derived twice they
/// would drift, and the one drawn would stop being the one used.
#[test]
fn the_rig_positions_lead_to_the_camera() {
    let (mut r, mut body) = (vcam(FOLLOW_SHOULDER), shoulder());
    body.camera_distance = 3.0;
    r.yaw = 35.0;
    r.pitch = 15.0;
    body.shoulder_offset = Vec3::new(0.6, -0.4, 0.2);
    body.vertical_arm_length = 1.2;

    let up = Vec3::Y;
    let reference = seed_reference(up);
    let target = Vec3::new(2.0, 1.0, -3.0);
    let (root, shoulder, hand) = r.rig_positions(target, up, reference, body);
    let camera = r.on_shoulder(target, body, up, reference);

    assert_eq!(root, target, "the chain starts on what it follows");
    assert!(
        (shoulder - root).length() > 0.1,
        "the shoulder is not offset: {shoulder:?}",
    );
    assert!(
        ((hand - shoulder).length() - 1.2).abs() < 1e-4,
        "the arm is not its own length: {}",
        (hand - shoulder).length(),
    );
    assert!(
        ((camera - hand).length() - 3.0).abs() < 1e-4,
        "the camera is not a distance back from the hand: {}",
        (camera - hand).length(),
    );
}

/// 🔴 #1391: a body is a component, so "does the orbital rig have a shoulder" is answered by the
/// component not being there — not by a gate inside the geometry. A rig with nothing authored still
/// collapses its three pivots onto the target, which is what stops the gizmo drawing a knot of stubs.
#[test]
fn a_plain_rig_has_no_chain() {
    let (r, body) = (vcam(FOLLOW_ORBITAL), shoulder());
    let up = Vec3::Y;
    let (root, shoulder, hand) = r.rig_positions(Vec3::ZERO, up, seed_reference(up), body);
    assert_eq!(root, shoulder);
    assert_eq!(shoulder, hand);
}

/// 🔴 #1391: a body is a component, and the numbers a scene wrote on the vcam have to reach it. An
/// alias maps a name inside a type; a field that moved to another component is not covered, so
/// without this every authored rig would take the defaults with nothing said.
#[test]
fn the_body_numbers_reach_their_component() {
    use kooch_ecs::component::ComponentRegistry;

    use kooch_ecs::archetype_registry::ArchetypeRegistry;

    let mut resources = kooch_core::resource::Resources::new();
    let mut archetypes = ArchetypeRegistry::new();
    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<VirtualCamera>();
    registry.register_cpu_reflected::<crate::OrbitalFollow>();
    registry.register_cpu_reflected::<crate::ThirdPersonFollow>();
    let (orbital, shouldered) = (
        kooch_ecs::entity::Entity::new(1, 0),
        kooch_ecs::entity::Entity::new(2, 0),
    );
    let storage = registry.get_cpu_mut::<VirtualCamera>().unwrap();
    storage.insert(
        orbital,
        VirtualCamera {
            was_distance: 9.0,
            was_orbit_style: ORBIT_THREE_RING,
            ..Default::default()
        },
    );
    storage.insert(
        shouldered,
        VirtualCamera {
            was_distance: 2.5,
            was_shoulder: Vec3::new(0.6, -0.4, 0.0),
            was_arm_length: 1.2,
            was_side: 0.0,
            ..Default::default()
        },
    );
    // The entities exist in the world, which is what the migration has to add to.
    let empty = archetypes.get_or_create(Default::default());
    archetypes.register_entity(orbital, empty);
    archetypes.register_entity(shouldered, empty);
    resources.insert(archetypes);
    resources.insert(registry);

    crate::virtual_camera::migrate_bodies(&mut resources);

    let registry = resources.get::<ComponentRegistry>().unwrap();
    let ring = registry
        .get_cpu::<crate::OrbitalFollow>()
        .unwrap()
        .get(orbital)
        .expect("the orbital body was never made");
    assert_eq!(ring.radius, 9.0);
    assert_eq!(ring.orbit_style, ORBIT_THREE_RING);

    let arm = registry
        .get_cpu::<crate::ThirdPersonFollow>()
        .unwrap()
        .get(shouldered)
        .expect("the shoulder body was never made");
    assert_eq!(arm.shoulder_offset, Vec3::new(0.6, -0.4, 0.0));
    assert_eq!(arm.vertical_arm_length, 1.2);
    assert_eq!(arm.camera_side, 0.0, "a left shoulder is not no shoulder");
    assert_eq!(arm.camera_distance, 2.5);

    // The hidden fields are emptied, so the migration runs once.
    let vcams = registry.get_cpu::<VirtualCamera>().unwrap();
    assert_eq!(vcams.get(orbital).unwrap().was_distance, 0.0);

    // 🔴 And the entity **has** it, not just the storage. A value written without the archetype is
    // invisible to every query and to the Inspector, which is exactly how this shipped (#1395).
    let archetypes = resources.get::<ArchetypeRegistry>().unwrap();
    for (entity, name) in [
        (orbital, std::any::TypeId::of::<crate::OrbitalFollow>()),
        (
            shouldered,
            std::any::TypeId::of::<crate::ThirdPersonFollow>(),
        ),
    ] {
        let at = archetypes.entity_archetype(entity).expect("no archetype");
        assert!(
            archetypes.get(at).unwrap().components().contains(&name),
            "the body is in the storage and not on the entity",
        );
    }
}

/// 🔴 #1397: the modes a scene named become the components that **are** them. A sabotage of the
/// `Follow` arm produced no failure at all, which is what a path nothing covers looks like.
#[test]
fn the_named_modes_become_components() {
    use kooch_ecs::archetype_registry::ArchetypeRegistry;
    use kooch_ecs::component::ComponentRegistry;

    let mut resources = kooch_core::resource::Resources::new();
    let mut archetypes = ArchetypeRegistry::new();
    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<VirtualCamera>();
    let empty = archetypes.get_or_create(Default::default());

    let (simple, glued, composed) = (
        kooch_ecs::entity::Entity::new(1, 0),
        kooch_ecs::entity::Entity::new(2, 0),
        kooch_ecs::entity::Entity::new(3, 0),
    );
    let offset = Vec3::new(0.0, 3.0, 10.0);
    for (entity, follow, look_at) in [
        (simple, FOLLOW_SIMPLE, LOOK_AT_SIMPLE),
        (glued, FOLLOW_GLUED, LOOK_AT_MIMIC),
        (composed, FOLLOW_POSITION_COMPOSER, LOOK_AT_ARM),
    ] {
        registry.get_cpu_mut::<VirtualCamera>().unwrap().insert(
            entity,
            VirtualCamera {
                was_follow: follow,
                was_look_at: look_at,
                was_offset: offset,
                ..Default::default()
            },
        );
        archetypes.register_entity(entity, empty);
    }
    resources.insert(archetypes);
    resources.insert(registry);

    crate::virtual_camera::migrate_bodies(&mut resources);

    let registry = resources.get::<ComponentRegistry>().unwrap();
    let archetypes = resources.get::<ArchetypeRegistry>().unwrap();
    let has = |entity, id| {
        archetypes
            .get(archetypes.entity_archetype(entity).expect("no archetype"))
            .unwrap()
            .components()
            .contains(&id)
    };

    // The offset went with the body that reads it, value and all.
    assert_eq!(
        crate::virtual_camera::one::<crate::Follow>(registry, simple)
            .expect("no Follow")
            .offset,
        offset,
    );
    assert!(has(simple, std::any::TypeId::of::<crate::Follow>()));
    assert!(has(
        glued,
        std::any::TypeId::of::<crate::HardLockToTarget>()
    ));
    assert!(has(
        composed,
        std::any::TypeId::of::<crate::PositionComposer>()
    ));

    // And each aim became the component that is it.
    assert!(has(simple, std::any::TypeId::of::<crate::HardLookAt>()));
    assert!(has(
        glued,
        std::any::TypeId::of::<crate::RotateWithFollowTarget>()
    ));
    assert!(has(composed, std::any::TypeId::of::<crate::PanTilt>()));

    // The hidden fields are emptied, so it runs once.
    let vcams = registry.get_cpu::<VirtualCamera>().unwrap();
    assert_eq!(vcams.get(simple).unwrap().was_follow, FOLLOW_NONE);
    assert_eq!(vcams.get(simple).unwrap().was_offset, Vec3::ZERO);
}
