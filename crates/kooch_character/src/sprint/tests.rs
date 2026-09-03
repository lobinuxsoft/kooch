use super::*;

/// Released is walking, exactly — not "nearly walking".
#[test]
fn a_released_sprint_is_walking() {
    assert_eq!(Sprint::default().scale(), (1.0, 1.0));
}

#[test]
fn a_held_sprint_scales_both() {
    let sprint = Sprint {
        wanted: true,
        ..Default::default()
    };
    let (speed, eagerness) = sprint.scale();
    assert!(speed > 1.0 && eagerness > 1.0);
}

/// A negative multiplier would walk the character backwards, which is
/// not a sprint anybody meant to author.
#[test]
fn a_negative_sprint_is_a_stop() {
    let sprint = Sprint {
        wanted: true,
        speed: -2.0,
        eagerness: -1.0,
    };
    assert_eq!(sprint.scale(), (0.0, 0.0));
}
