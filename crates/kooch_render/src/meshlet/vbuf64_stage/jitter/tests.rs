use super::*;
use glam::Vec4Swizzles;

/// The three counts every assertion below is checked against: 1:1, the
/// 1.5× the Steam Deck target implies, and 2×.
const COUNTS: [u32; 3] = [8, 18, 32];

/// An offset outside the pixel is parallax, not anti-aliasing.
///
/// It would still look plausible — the image moves, the resolve blends,
/// the result is soft — which is exactly why this is asserted rather
/// than eyeballed. A sequence scaled by two produces a permanently
/// blurred frame and nothing that says why.
#[test]
fn every_offset_stays_inside_one_pixel() {
    for phases in COUNTS {
        for index in 0..phases * 3 {
            let o = offset(index, phases);
            assert!(
                o.x.abs() <= 0.5 && o.y.abs() <= 0.5,
                "{phases} phases, frame {index} offsets by {o:?}, which leaves the pixel",
            );
        }
    }
}

/// The whole point of a low-discrepancy sequence: no two frames in a
/// cycle sample the same place, and none of them sample the centre.
///
/// A sequence that repeats a point wastes the frame — the resolve
/// integrates a sample it already has — and one that includes the exact
/// centre wastes it twice over, because the unjittered image is the one
/// every other sample is being averaged against.
///
/// 🔴 Asserted at every count, not just the base one. Halton stays
/// injective however far it is taken, but a period that ever wraps to a
/// term already used would degrade silently into a shorter sequence.
#[test]
fn a_cycle_visits_distinct_points() {
    for phases in COUNTS {
        let points: Vec<_> = (0..phases).map(|i| offset(i, phases)).collect();
        for (i, a) in points.iter().enumerate() {
            assert!(
                a.length() > 1e-4,
                "{phases} phases, frame {i} sits on the pixel centre",
            );
            for (j, b) in points.iter().enumerate().skip(i + 1) {
                assert!(
                    (*a - *b).length() > 1e-4,
                    "{phases} phases, frames {i} and {j} both sample {a:?}",
                );
            }
        }
    }
}

/// And it repeats after exactly its period, so the accumulated image is
/// a fixed set of samples rather than a drifting one.
#[test]
fn the_sequence_repeats_on_period() {
    for phases in COUNTS {
        for index in 0..phases {
            assert_eq!(offset(index, phases), offset(index + phases, phases));
        }
    }
}

/// 🔴 The count scales with the SQUARE of the ratio, because what the
/// sequence covers is an area.
///
/// Scaling it linearly is the plausible-looking mistake: at 1.5× it
/// would give 24 phases where the input pixels carry 2.25× less of the
/// output, so the reconstruction is starved exactly where the upscaler
/// is working hardest — and it reads as "FSR is soft", not as a jitter
/// bug.
#[test]
fn phases_scale_with_the_area() {
    assert_eq!(phase_count(1280, 1280), JITTER_BASE_PHASES);
    // 1.5x squares to 2.25: ceil(8 x 2.25) = 18, where scaling the
    // ratio linearly would give 12.
    assert_eq!(phase_count(1280, 1920), 18);
    // 2x squares to 4: 32, against 16 for the linear misreading.
    assert_eq!(phase_count(960, 1920), 32);
}

/// Rendering above display resolution is supersampling, which needs no
/// help from the projection. Shrinking the sequence there would shorten
/// the history for nothing.
#[test]
fn supersampling_keeps_the_base_count() {
    assert_eq!(phase_count(3840, 1920), JITTER_BASE_PHASES);
    assert_eq!(phase_count(1920, 0), JITTER_BASE_PHASES);
}

/// 🔴 A degenerate width must not produce a sequence that never
/// repeats.
///
/// Zero is reachable — a minimised window, a target queried a frame
/// early — and unclamped it squares to tens of millions of phases. The
/// image would go soft because the accumulation never closes a cycle,
/// and nothing in a capture would say so.
#[test]
fn a_zero_width_stays_bounded() {
    assert_eq!(phase_count(0, 1920), JITTER_MAX_PHASES);
    assert_eq!(phase_count(0, 0), JITTER_BASE_PHASES);
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

    for index in 0..JITTER_BASE_PHASES {
        let j = Jitter::at(index, view_proj, size, JITTER_BASE_PHASES);
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
    for index in 0..JITTER_BASE_PHASES {
        assert_eq!(
            Jitter::at(index, view_proj, (800, 500), JITTER_BASE_PHASES).unjittered,
            view_proj
        );
    }
    assert_eq!(Jitter::none(view_proj).view_proj, view_proj);
    assert_eq!(Jitter::none(view_proj).pixels, Vec2::ZERO);
}
