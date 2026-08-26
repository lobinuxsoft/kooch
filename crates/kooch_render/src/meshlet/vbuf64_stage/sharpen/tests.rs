use super::*;

/// The setting is a percentage and the shader wants `0..=1`.
///
/// 100 has to be exactly 1.0 rather than nearly it: the amount
/// multiplies a limiter that is already at the edge of looking natural,
/// and an off-by-a-fraction there is a halo nobody can trace back to a
/// unit conversion.
#[test]
fn a_percentage_becomes_a_fraction() {
    assert_eq!(sharpness_of(0), 0.0);
    assert_eq!(sharpness_of(50), 0.5);
    assert_eq!(sharpness_of(100), 1.0);
}

/// A `.rendersettings` file is text a person can edit, so the value can
/// arrive above the range. Clamped rather than trusted: the shader
/// multiplies `RCAS_LIMIT` by this, and a 500 % amount is a lobe five
/// times past the point upstream measured as the limit of natural
/// results — ringing on every edge, from one typo.
#[test]
fn a_typo_cannot_exceed_full() {
    assert_eq!(sharpness_of(500), 1.0);
    assert_eq!(sharpness_of(u32::MAX), 1.0);
}
