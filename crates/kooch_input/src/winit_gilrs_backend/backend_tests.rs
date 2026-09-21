use std::time::Instant;

use super::*;

/// 🔴 A build that never returns must not hold up the caller — gilrs under Proton on a OneXFly hung
/// the game before its window (#963).
#[test]
fn a_build_that_never_answers_is_abandoned() {
    let started = Instant::now();

    let outcome = build_within(Duration::from_millis(80), || {
        std::thread::sleep(Duration::from_secs(30));
        Ok::<u8, String>(1)
    });

    assert!(matches!(outcome, Built::NoAnswer));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the caller waited for it anyway: {:?}",
        started.elapsed(),
    );
}

/// A build that answers in time is handed back, deadline or no deadline.
#[test]
fn a_build_that_answers_is_returned() {
    let outcome = build_within(Duration::from_secs(5), || Ok::<u8, String>(7));

    match outcome {
        Built::Ready(value) => assert_eq!(value, 7),
        _ => panic!("a build that succeeded was thrown away"),
    }
}

/// ⚠️ Refusing and hanging must stay distinct: both mean no gamepads, but the log has to say which.
#[test]
fn a_refusal_is_not_a_silence() {
    let outcome = build_within(Duration::from_secs(5), || {
        Err::<u8, String>("no device backend".to_owned())
    });

    match outcome {
        Built::Failed(error) => assert_eq!(error, "no device backend"),
        _ => panic!("a refusal was read as something else"),
    }
}

/// Motion over the time it took (#1266): the same hand movement reads the same at any frame rate.
#[test]
fn velocity_divides_by_the_frame() {
    assert_eq!(
        velocity(glam::Vec2::new(10.0, -5.0), 0.5),
        glam::Vec2::new(20.0, -10.0)
    );
}

/// A clock that did not move is not a mouse moving at infinite speed.
#[test]
fn a_zero_frame_is_not_infinite() {
    assert!(velocity(glam::Vec2::X, 0.0).is_finite());
}
