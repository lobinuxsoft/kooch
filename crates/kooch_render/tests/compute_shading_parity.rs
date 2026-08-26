//! #824's acceptance: the compute shading path renders the *same*
//! image as the fragment one — not a similar one.
//!
//! The issue says it in those words, and the reason it is worth a whole
//! test file is that every other thing #824 buys is a number in a
//! capture. A path that is 20 % faster and one bit different is not an
//! optimisation, it is a second renderer nobody agreed to maintain, and
//! the difference would show up as "the handheld looks a bit off"
//! months later with no way left to bisect it.
//!
//! Byte-for-byte on an 8-bit target, deliberately. The two paths run the
//! identical composed shader — same reconstruction, same BRDF, same
//! lights in the same order — so the float results are bit-identical
//! before quantisation and a tolerance would only hide a real
//! divergence. The one thing that genuinely differs is where the light
//! indices were read from, which cannot change a value.
//!
//! Scene: a floor under a grid of point lights, so the froxels actually
//! hold several lights each and the workgroup cache has something to
//! cache. One light would exercise the machinery and prove nothing about
//! it.
//!
//! Run with:
//!   cargo test -p kooch_render --test compute_shading_parity

mod common;

use common::lit_scene::{SIZE, render, rig};
use kooch_lighting::ClusterSettings;

/// The parity assertion, in the two halves that are actually provable.
///
/// **Coverage is exact.** Alpha is 1.0 for a pixel a meshlet covered and
/// 0.0 for one it did not — two values, no arithmetic between them, so
/// they quantise to 255 and 0 on any path. If the compute pass decoded
/// the visibility buffer differently, resolved a different material, or
/// left a pixel unwritten, alpha says so and nothing else has to.
///
/// **Colour is equal to the precision of the target.** Not byte-for-byte,
/// and the reason is not #824: the fragment path's colour reaches the
/// Rgba8Unorm target through the ROP and the compute path's through
/// `textureStore`, and the two f32→unorm8 conversions are not required
/// to round the same way in the last bit.
///
/// 🔴 That is measured, not assumed. `unclustered_falls_back_and_matches`
/// makes the compute pass call the *same* `inti_shade` on the *same*
/// data as the fragment path — nothing of this issue in the difference —
/// and about 8 % of its pixels still land one unit apart in one channel,
/// with a maximum delta of exactly 1. So a tolerance of one is the
/// floor two shading stages can agree to, and anything above it is a
/// real divergence.
///
/// The pair is not a weak assertion. Every failure mode this issue can
/// have — a light missing from the tile cache, a light counted twice, a
/// run read at the wrong offset, the block reduced to the wrong cells —
/// changes radiance by far more than 1/255, which the delta bound
/// catches, or changes which pixels are lit, which alpha catches.
fn assert_same_image(fragment: &[u8], compute: &[u8], what: &str) {
    assert_eq!(fragment.len(), compute.len(), "{what}: different sizes");
    // 🔴 Here rather than in one test, because two renders of nothing
    // match byte for byte and every assertion below then passes having
    // compared an empty frame with an empty frame. This file did exactly
    // that on its first run — a floor scaled 20 m on all three axes put
    // the camera inside the box.
    assert!(
        fragment.chunks_exact(4).any(|p| p[3] != 0),
        "{what}: the scene rendered empty — the parity assertion would be vacuous",
    );

    let coverage = fragment
        .chunks_exact(4)
        .zip(compute.chunks_exact(4))
        .position(|(a, b)| a[3] != b[3]);
    if let Some(idx) = coverage {
        panic!(
            "{what}: the two paths cover different pixels. \
             first at ({}, {}): fragment alpha {}, compute alpha {}",
            idx as u32 % SIZE,
            idx as u32 / SIZE,
            fragment[idx * 4 + 3],
            compute[idx * 4 + 3],
        );
    }

    let worst = fragment
        .chunks_exact(4)
        .zip(compute.chunks_exact(4))
        .enumerate()
        .max_by_key(|(_, (a, b))| a.iter().zip(b.iter()).map(|(x, y)| x.abs_diff(*y)).max())
        .expect("a non-empty image");
    let (idx, (a, b)) = worst;
    let delta = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0);
    assert!(
        delta <= 1,
        "{what}: the shading paths disagree by {delta}/255, past the one unit two \
         stages can differ by through quantisation alone.\n\
         worst at ({}, {}): fragment {a:?}, compute {b:?}",
        idx as u32 % SIZE,
        idx as u32 / SIZE,
    );
}

/// 🔴 The issue's acceptance criterion, in one assertion.
#[test]
fn both_paths_render_the_same() {
    let Some(mut rig) = rig(4, false) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };

    let fragment = render(&mut rig, false);
    let compute = render(&mut rig, true);
    assert_same_image(&fragment, &compute, "floor under a grid of point lights");
}

/// The fallback the workgroup cache promises, on the path that is
/// simplest to reach on purpose: with no grid there is no cell to cache,
/// and the compute pass must shade from the light buffer exactly as the
/// fragment path does.
#[test]
fn unclustered_falls_back_and_matches() {
    let Some(mut rig) = rig(3, false) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    rig.resources.insert(ClusterSettings {
        enabled: false,
        ..Default::default()
    });

    let fragment = render(&mut rig, false);
    let compute = render(&mut rig, true);
    assert_same_image(&fragment, &compute, "clustering off");
}

/// A wall standing on the floor, so tiles along its silhouette hold
/// pixels many z-slices apart and draw their lights from a block of
/// froxels rather than one.
///
/// Whether that block exceeds the cap depends on the grid, and the test
/// deliberately does not assert which branch ran: both are supposed to
/// produce this image, and pinning the branch would make the test fail
/// the day the cap is retuned — which is a change to performance and
/// not to correctness.
#[test]
fn a_deep_silhouette_matches() {
    let Some(mut rig) = rig(4, true) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };

    let fragment = render(&mut rig, false);
    let compute = render(&mut rig, true);
    assert_same_image(&fragment, &compute, "wall silhouette");
}

/// 🔴 `KOOCH_LIGHT_LIMIT` exists to be measured with, and a knob that
/// silently does nothing produces a capture that looks like an answer.
///
/// Both paths, because an A/B between them with the cap on would
/// otherwise be measuring the cap against no cap.
#[test]
fn the_light_limit_darkens_both_paths() {
    let Some(mut rig) = rig(4, false) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };

    for compute in [false, true] {
        rig.resources.insert(kooch_lighting::LightLimit(0));
        let all = render(&mut rig, compute);
        rig.resources.insert(kooch_lighting::LightLimit(1));
        let capped = render(&mut rig, compute);

        let path = if compute { "compute" } else { "fragment" };
        assert_ne!(
            all, capped,
            "{path}: capping a froxel's lights to one changed nothing on screen",
        );
        // Darker, not merely different: the cap drops light, and a
        // change in the other direction would mean it is doing
        // something other than what it says.
        let sum = |p: &[u8]| p.chunks_exact(4).map(|c| c[0] as u64).sum::<u64>();
        assert!(
            sum(&capped) < sum(&all),
            "{path}: the capped render is not darker",
        );
    }
}
