use super::*;

const DT: f32 = 1.0 / 60.0;

/// Arriving slowly is not a wall run in any game that has one.
#[test]
fn it_takes_speed_to_start() {
    let run = WallRun::default();
    assert_eq!(carry(None, 1.0, &run, DT), Run::Refused);
    assert!(matches!(carry(None, 6.0, &run, DT), Run::Going(_)));
}

/// And it is asked once. Answering every frame let a character arrive
/// at walking pace and steer itself up to running speed against the
/// wall, which is a cling that turns into a run.
#[test]
fn a_refusal_sticks() {
    let run = WallRun::default();
    let refused = carry(None, 1.0, &run, DT);
    assert_eq!(carry(Some(refused), 9.0, &run, DT), Run::Refused);
}

/// Once started, the speed no longer gates it: a run that cut out the
/// moment you slowed would end in the middle for no visible reason.
#[test]
fn a_started_run_carries_on() {
    let run = WallRun::default();
    let going = carry(Some(Run::Going(0.4)), 0.0, &run, DT);
    assert!(matches!(going, Run::Going(_)));
}

/// And the clock is what ends it.
#[test]
fn the_clock_runs_out() {
    let run = WallRun::default();
    let over = carry(Some(Run::Going(run.duration)), 9.0, &run, DT);
    assert_eq!(over, Run::Refused);
}

/// Spent stays spent while it is in the air — otherwise a character
/// chains one wall for ever by letting go of it for a frame.
#[test]
fn a_spent_run_needs_the_ground() {
    let run = WallRun::default();
    let mut runs = Runs::default();
    let hero = Entity::new(1, 0);
    runs.set(hero, Run::Refused);
    assert_eq!(carry(runs.state(hero), 9.0, &run, DT), Run::Refused);
    runs.landed(hero);
    assert!(matches!(
        carry(runs.state(hero), 9.0, &run, DT),
        Run::Going(_)
    ));
}

/// Only the part along the wall counts as running speed. Straight at it
/// is an arrival.
#[test]
fn only_along_the_wall_counts() {
    let into = along(Vec3::X * 9.0, Vec3::NEG_X, Vec3::Y);
    assert!(into.length() < 1e-5, "{into}");
    let past = along(Vec3::Z * 9.0, Vec3::NEG_X, Vec3::Y);
    assert!((past.length() - 9.0).abs() < 1e-4, "{past}");
}

/// Falling down a wall is not running along it either.
#[test]
fn falling_is_not_running() {
    let dropping = along(Vec3::NEG_Y * 9.0, Vec3::NEG_X, Vec3::Y);
    assert!(dropping.length() < 1e-5, "{dropping}");
}

/// The bank is the whole read of the move from outside.
#[test]
fn it_banks_towards_the_wall() {
    let leaning = banked(Vec3::Y, Vec3::NEG_X, 0.55);
    assert!(leaning.x < -0.4, "should tip towards the wall: {leaning}");
    assert!(leaning.y > 0.4, "but not lie down on it: {leaning}");
}

#[test]
fn no_bank_stands_straight() {
    assert_eq!(banked(Vec3::Y, Vec3::NEG_X, 0.0), Vec3::Y);
}
