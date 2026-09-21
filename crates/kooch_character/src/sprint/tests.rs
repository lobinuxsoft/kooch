use super::*;

/// Released is walking, exactly — not "nearly walking".
#[test]
fn a_released_sprint_is_walking() {
    assert_eq!(Sprint::default().scale(), (1.0, 1.0));
}

#[test]
fn a_held_sprint_scales_both() {
    let mut sprint = Sprint {
        wanted: true,
        ..Default::default()
    };
    sprint.step(true);
    let (speed, eagerness) = sprint.scale();
    assert!(speed > 1.0 && eagerness > 1.0);
}

/// A negative multiplier would walk the character backwards, which is
/// not a sprint anybody meant to author.
#[test]
fn a_negative_sprint_is_a_stop() {
    let mut sprint = Sprint {
        wanted: true,
        speed: -2.0,
        eagerness: -1.0,
        ..Default::default()
    };
    sprint.step(true);
    assert_eq!(sprint.scale(), (0.0, 0.0));
}

fn toggled() -> Sprint {
    Sprint {
        toggle: true,
        ..Default::default()
    }
}

/// Steps with the button `held` and the move `moving`, answering whether it runs.
fn step(sprint: &mut Sprint, held: bool, moving: bool) -> bool {
    sprint.wanted = held;
    sprint.step(moving);
    sprint.running
}

#[test]
fn a_toggle_outlives_the_release() {
    let mut sprint = toggled();
    assert!(step(&mut sprint, true, true));
    assert!(step(&mut sprint, false, true));
    assert!(step(&mut sprint, false, true));
}

#[test]
fn a_second_press_walks() {
    let mut sprint = toggled();
    step(&mut sprint, true, true);
    step(&mut sprint, false, true);
    assert!(!step(&mut sprint, true, true));
    assert!(
        !step(&mut sprint, true, true),
        "holding is not pressing again"
    );
}

#[test]
fn stopping_ends_a_toggle() {
    let mut sprint = toggled();
    step(&mut sprint, true, true);
    assert!(!step(&mut sprint, false, false));
    assert!(!step(&mut sprint, false, true), "moving again walks");
}

/// Pressed standing still there is nothing to run with, so nothing is armed for later.
#[test]
fn a_still_press_arms_nothing() {
    let mut sprint = toggled();
    assert!(!step(&mut sprint, true, false));
    assert!(!step(&mut sprint, false, true));
}

#[test]
fn a_held_sprint_ignores_moving() {
    let mut sprint = Sprint::default();
    assert!(step(&mut sprint, true, false));
    assert!(!step(&mut sprint, false, true));
}
