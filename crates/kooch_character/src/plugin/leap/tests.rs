use super::*;

const DT: f32 = 1.0 / 60.0;

fn asked() -> Jump {
    Jump {
        wanted: true,
        ..Default::default()
    }
}

#[test]
fn a_jump_on_the_ground_fires() {
    let mut tally = Tally::default();
    let leap = spend(&mut tally, &asked(), None, true, Vec3::Y, DT);
    assert_eq!(leap, Some(Leap::Ground(Vec3::Y * 5.0)));
}

/// Nothing asked is nothing spent, or a character would jump the moment
/// it touched the ground.
#[test]
fn silence_does_nothing() {
    let mut tally = Tally::default();
    assert_eq!(
        spend(&mut tally, &Jump::default(), None, true, Vec3::Y, DT),
        None
    );
}

/// The second jump is the one the component is for, and the third is
/// not — `air_jumps` is a count, not a switch.
#[test]
fn air_jumps_run_out() {
    let mut tally = Tally::default();
    let jump = Jump {
        air_jumps: 2,
        coyote: 0.0,
        ..Default::default()
    };
    let held = Jump {
        wanted: true,
        ..jump
    };
    // Off the ground long enough that coyote time is gone.
    spend(&mut tally, &jump, None, false, Vec3::Y, 1.0);
    for spent in 0..2 {
        let leap = spend(&mut tally, &held, None, false, Vec3::Y, DT);
        assert!(leap.is_some(), "air jump {spent} should have fired");
    }
    assert_eq!(spend(&mut tally, &held, None, false, Vec3::Y, DT), None);
}

/// And landing refills them.
#[test]
fn the_ground_refills_them() {
    let mut tally = Tally {
        spent: 9,
        ..Default::default()
    };
    spend(&mut tally, &Jump::default(), None, true, Vec3::Y, DT);
    assert_eq!(tally.spent, 0);
}

/// Walked off a ledge a moment ago: the jump the player thinks they
/// still have.
#[test]
fn coyote_time_still_jumps() {
    let mut tally = Tally::default();
    spend(&mut tally, &Jump::default(), None, false, Vec3::Y, 0.05);
    assert!(spend(&mut tally, &asked(), None, false, Vec3::Y, DT).is_some());
}

/// But it is a grace, not a free jump: taking it costs the ground jump,
/// or walking off a ledge would be worth more than standing still.
#[test]
fn coyote_time_is_spent_once() {
    let mut tally = Tally::default();
    spend(&mut tally, &Jump::default(), None, false, Vec3::Y, 0.05);
    let jump = Jump {
        air_jumps: 0,
        ..Default::default()
    };
    assert!(
        spend(
            &mut tally,
            &Jump {
                wanted: true,
                ..jump
            },
            None,
            false,
            Vec3::Y,
            DT
        )
        .is_some()
    );
    assert_eq!(
        spend(
            &mut tally,
            &Jump {
                wanted: true,
                ..jump
            },
            None,
            false,
            Vec3::Y,
            DT
        ),
        None
    );
}

/// Pressed a moment before landing, it fires on the frame the ground
/// arrives rather than being eaten.
#[test]
fn a_buffered_press_survives() {
    let mut tally = Tally::default();
    let jump = Jump {
        coyote: 0.0,
        air_jumps: 0,
        ..Default::default()
    };
    // Asked in mid-air, where nothing is available.
    spend(
        &mut tally,
        &Jump {
            wanted: true,
            ..jump
        },
        None,
        false,
        Vec3::Y,
        DT,
    );
    // And the ground arrives a couple of frames later.
    spend(&mut tally, &jump, None, false, Vec3::Y, DT);
    assert!(spend(&mut tally, &jump, None, true, Vec3::Y, DT).is_some());
}

/// Held too long and it stops counting, or a press is honoured a second
/// after the player gave up on it.
#[test]
fn a_stale_press_expires() {
    let mut tally = Tally::default();
    let jump = Jump {
        coyote: 0.0,
        air_jumps: 0,
        buffer: 0.1,
        ..Default::default()
    };
    spend(
        &mut tally,
        &Jump {
            wanted: true,
            ..jump
        },
        None,
        false,
        Vec3::Y,
        DT,
    );
    // Long enough that the player has stopped expecting it.
    spend(&mut tally, &jump, None, false, Vec3::Y, 0.5);
    assert_eq!(spend(&mut tally, &jump, None, true, Vec3::Y, DT), None);
}

/// Off a wall: away from it and up, and it is available when nothing
/// else is.
#[test]
fn a_wall_is_something_to_push_off() {
    let mut tally = Tally {
        spent: 9,
        ungrounded: 99.0,
        ..Default::default()
    };
    let off = WallJump::default();
    let leap = spend(
        &mut tally,
        &asked(),
        Some((&off, Vec3::X)),
        false,
        Vec3::Y,
        DT,
    );
    let Some(Leap::Wall(velocity)) = leap else {
        panic!("should have jumped off the wall: {leap:?}");
    };
    assert!(velocity.x > 0.0 && velocity.y > 0.0, "{velocity}");
}

/// A wall the character is standing beside is not a wall jump — it is
/// an ordinary one, and it should go straight up.
#[test]
fn standing_beside_a_wall_is_a_normal_jump() {
    let mut tally = Tally::default();
    let off = WallJump::default();
    let leap = spend(
        &mut tally,
        &asked(),
        Some((&off, Vec3::X)),
        true,
        Vec3::Y,
        DT,
    );
    assert_eq!(leap, Some(Leap::Ground(Vec3::Y * 5.0)));
}
