use std::time::Instant;

use super::*;

/// 🔴 The point of the whole thing: a build that never returns must not
/// hold up the caller.
///
/// This is what a Windows build did on a OneXFly under Proton (#963) —
/// gilrs went into `Windows.Gaming.Input`, met a vendor HID Wine cannot
/// open, and never came back. Input is built before the window and the
/// GPU, so the game showed nothing, printed nothing, and did not fail.
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

/// ⚠️ Refusing and hanging are different, and must stay different.
///
/// Both end with no gamepads, but one is a machine without a device
/// backend — headless, a container, no evdev — and the other is a
/// backend that is stuck. Collapsing them would put the wrong sentence
/// in the log of whichever happens next.
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
