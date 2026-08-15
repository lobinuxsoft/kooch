use super::*;
use glam::Vec4Swizzles;

/// An offset outside the pixel is parallax, not anti-aliasing.
///
/// It would still look plausible — the image moves, the resolve blends,
/// the result is soft — which is exactly why this is asserted rather
/// than eyeballed. A sequence scaled by two produces a permanently
/// blurred frame and nothing that says why.
#[test]
fn every_offset_stays_inside_one_pixel() {
    for index in 0..JITTER_PERIOD * 3 {
        let o = offset(index);
        assert!(
            o.x.abs() <= 0.5 && o.y.abs() <= 0.5,
            "frame {index} offsets by {o:?}, which leaves the pixel",
        );
    }
}

/// The whole point of a low-discrepancy sequence: no two frames in a
/// cycle sample the same place, and none of them sample the centre.
///
/// A sequence that repeats a point wastes the frame — the resolve
/// integrates a sample it already has — and one that includes the exact
/// centre wastes it twice over, because the unjittered image is the one
/// every other sample is being averaged against.
#[test]
fn a_cycle_visits_eight_distinct_points() {
    let points: Vec<_> = (0..JITTER_PERIOD).map(offset).collect();
    for (i, a) in points.iter().enumerate() {
        assert!(
            a.length() > 1e-4,
            "frame {i} sits on the pixel centre, contributing nothing new",
        );
        for (j, b) in points.iter().enumerate().skip(i + 1) {
            assert!(
                (*a - *b).length() > 1e-4,
                "frames {i} and {j} sample the same point {a:?}",
            );
        }
    }
}

/// And it repeats after exactly eight, so the accumulated image is a
/// fixed set of samples rather than a drifting one.
#[test]
fn the_sequence_repeats_on_period() {
    for index in 0..JITTER_PERIOD {
        assert_eq!(offset(index), offset(index + JITTER_PERIOD));
    }
}

/// 🔴 The assertion that catches the sign-and-scale family at once.
///
/// A point on the near plane, projected with and without jitter: the
/// difference in NDC has to be the offset expressed in NDC units, which
/// is the offset in pixels over half the viewport. Get the doubling
/// wrong and the jitter is a quarter pixel (no anti-aliasing) or two
/// (permanent blur), and both look like TAA being "not very good".
#[test]
fn the_shift_is_the_offset_in_ndc() {
    let size = (640u32, 400u32);
    let view_proj = Mat4::perspective_rh(1.0, size.0 as f32 / size.1 as f32, 0.1, 100.0)
        * Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
    let world = glam::Vec4::new(0.3, -0.7, 0.0, 1.0);

    for index in 0..JITTER_PERIOD {
        let j = Jitter::at(index, view_proj, size);
        let plain = j.unjittered * world;
        let shifted = j.view_proj * world;
        let delta = shifted.xy() / shifted.w - plain.xy() / plain.w;
        let expected = glam::Vec2::new(
            2.0 * j.pixels.x / size.0 as f32,
            -2.0 * j.pixels.y / size.1 as f32,
        );
        assert!(
            (delta - expected).length() < 1e-5,
            "frame {index} shifted by {delta:?}, expected {expected:?}",
        );
    }
}

/// The unjittered matrix has to come back untouched, because the motion
/// vectors are computed from it. Jitter reaching them would describe
/// itself as scene motion and the reprojection would cancel the exact
/// signal the resolve accumulates — a TAA that runs, costs and does
/// nothing.
#[test]
fn the_unjittered_matrix_is_the_original() {
    let view_proj = Mat4::perspective_rh(1.0, 1.6, 0.1, 100.0);
    for index in 0..JITTER_PERIOD {
        assert_eq!(
            Jitter::at(index, view_proj, (800, 500)).unjittered,
            view_proj
        );
    }
    assert_eq!(Jitter::none(view_proj).view_proj, view_proj);
    assert_eq!(Jitter::none(view_proj).pixels, Vec2::ZERO);
}
