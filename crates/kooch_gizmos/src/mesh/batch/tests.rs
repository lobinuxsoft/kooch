use super::*;

/// 🔴 The crash this file was fixed for: `clamp` panics when min > max, and the 5 cm head floor is
/// above `length * 0.4` for any arrow shorter than 12.5 cm. Screen-scaled gizmos draw arrows that
/// short whenever the camera is close, so this took the whole editor down on a scene with a spline.
#[test]
fn a_short_arrow_does_not_panic() {
    let mut batch = MeshBatch::default();
    // Either side of the 12.5 cm threshold, and well below it.
    for length in [0.124_2_f32, 0.124, 0.05, 0.01, 0.001, 0.125, 0.2, 1.0] {
        batch.filled_arrow(Vec3::ZERO, Vec3::Y * length, Vec4::ONE);
    }
    assert_eq!(batch.arrows.len(), 8);
}

/// An arrow too short to have a direction is skipped rather than drawn as a degenerate cone.
#[test]
fn a_zero_arrow_is_skipped() {
    let mut batch = MeshBatch::default();
    batch.filled_arrow(Vec3::ZERO, Vec3::ZERO, Vec4::ONE);
    assert!(batch.arrows.is_empty());
}
