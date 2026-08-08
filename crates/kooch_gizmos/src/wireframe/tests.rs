use super::*;
use crate::{GizmoBatch, MeshBatch};

fn circle_points(radius: f32) -> usize {
    let mut lines = GizmoBatch::default();
    let mut meshes = MeshBatch::default();
    {
        let mut gizmos = Gizmos::new(&mut lines, &mut meshes);
        gizmos.wire_circle(Vec3::ZERO, Vec3::X, Vec3::Z, radius, Vec3::ONE);
    }
    lines.lines.len()
}

/// A bigger circle gets more segments. A fixed count is the wrong knob:
/// it over-tessellates a small collider and still looks polygonal on a
/// large light sphere, because how round a circle looks depends on its
/// radius.
#[test]
fn segment_count_grows_with_radius() {
    assert!(
        segments_for(20.0) > segments_for(1.0),
        "a large circle is drawn no finer than a small one"
    );
    assert_eq!(circle_points(20.0), segments_for(20.0) as usize);
}

/// Sub-linear, so a planet-scale radius does not ask for thousands.
/// Doubling the radius should cost roughly 40% more, not 100%.
#[test]
fn segment_count_grows_sub_linearly() {
    let (small, large) = (segments_for(4.0), segments_for(16.0));
    // Four times the radius is twice the count, not four times — as
    // long as neither end is clamped.
    if small > MIN_CIRCLE_SEGMENTS && large < MAX_CIRCLE_SEGMENTS {
        let ratio = large as f32 / small as f32;
        assert!(
            (1.6..2.4).contains(&ratio),
            "growth is not square-root: {small} → {large}"
        );
    }
}

/// Bounded at both ends: a floor so a tiny circle still reads as round,
/// a ceiling so a huge one does not flood the line batch.
#[test]
fn segment_count_is_bounded() {
    for radius in [f32::EPSILON, 0.001, 0.5, 1.0, 1e3, 1e6] {
        let n = segments_for(radius);
        assert!(
            (MIN_CIRCLE_SEGMENTS..=MAX_CIRCLE_SEGMENTS).contains(&n),
            "radius {radius} gave {n} segments"
        );
    }
    // Zero and negative radii must not divide by nothing or take a
    // square root of a negative.
    assert!(segments_for(0.0) >= MIN_CIRCLE_SEGMENTS);
    assert!(segments_for(-5.0) >= MIN_CIRCLE_SEGMENTS);
}

/// The actual chord error stays near the target for radii between the
/// clamps — this is the property the whole scheme exists to hold, and
/// the one that makes the circles look equally smooth at every size.
#[test]
fn chord_error_stays_near_the_target() {
    for radius in [2.0f32, 5.0, 10.0] {
        let n = segments_for(radius) as f32;
        if n >= MAX_CIRCLE_SEGMENTS as f32 || n <= MIN_CIRCLE_SEGMENTS as f32 {
            continue;
        }
        let sagitta = radius * (1.0 - (std::f32::consts::PI / n).cos());
        assert!(
            sagitta <= CHORD_ERROR * 1.5,
            "radius {radius}: chord error {sagitta} exceeds the target"
        );
    }
}

/// An arc keeps the same density as the full circle it belongs to,
/// rather than spending a whole circle's segments on a quarter turn.
#[test]
fn an_arc_scales_its_segments_with_its_span() {
    let mut lines = GizmoBatch::default();
    let mut meshes = MeshBatch::default();
    {
        let mut gizmos = Gizmos::new(&mut lines, &mut meshes);
        gizmos.wire_arc(
            Vec3::ZERO,
            Vec3::X,
            Vec3::Z,
            5.0,
            0.0,
            std::f32::consts::FRAC_PI_2,
            Vec3::ONE,
        );
    }
    let quarter = lines.lines.len();
    let full = circle_points(5.0);
    assert!(
        quarter < full,
        "a quarter arc used as many segments as a full circle"
    );
    assert!(
        (quarter as f32 - full as f32 / 4.0).abs() <= 2.0,
        "a quarter arc should be about a quarter of the segments: {quarter} vs {full}"
    );
}
