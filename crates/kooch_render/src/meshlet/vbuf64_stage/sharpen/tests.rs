use super::*;

/// The setting is a percentage and the shader wants `0..=1`.
#[test]
fn a_percentage_becomes_a_fraction() {
    assert_eq!(sharpness_of(0), 0.0);
    assert_eq!(sharpness_of(50), 0.5);
    assert_eq!(sharpness_of(100), 1.0);
}

/// A `.rendersettings` file is text a person can edit, so the value can arrive above the range.
#[test]
fn a_typo_cannot_exceed_full() {
    assert_eq!(sharpness_of(500), 1.0);
    assert_eq!(sharpness_of(u32::MAX), 1.0);
}
