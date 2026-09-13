//! the *silhouette does not*.

mod common;

use common::lit_scene::{SIZE, render_at, rig};
use kooch_render::meshlet::ShadingRate;

/// How the two rates' images differ. Deltas are the sum of the three
/// channels' absolute differences, so 0..765.
struct Diff {
    /// First pixel where alpha differs — geometry gained or lost.
    coverage_mismatch: Option<usize>,
    /// Mean over covered pixels.
    mean: f64,
    /// The 99.9th percentile. What separates "reconstruction is a little
    /// soft everywhere" from "a few hundred pixels are plain wrong".
    p999: u32,
    /// Covered pixels whose colour is not identical.
    changed: usize,
    covered: usize,
}

fn diff(full: &[u8], half: &[u8]) -> Diff {
    assert_eq!(full.len(), half.len(), "different sizes");
    assert!(
        full.chunks_exact(4).any(|p| p[3] != 0),
        "the scene rendered empty — every assertion here would be vacuous",
    );
    let mut d = Diff {
        coverage_mismatch: None,
        mean: 0.0,
        p999: 0,
        changed: 0,
        covered: 0,
    };
    let mut histogram = [0u32; 766];
    let mut total = 0u64;
    for (i, (a, b)) in full.chunks_exact(4).zip(half.chunks_exact(4)).enumerate() {
        if a[3] != b[3] {
            d.coverage_mismatch.get_or_insert(i);
            continue;
        }
        if a[3] == 0 {
            continue;
        }
        d.covered += 1;
        let delta: u32 = a[..3]
            .iter()
            .zip(b[..3].iter())
            .map(|(x, y)| x.abs_diff(*y) as u32)
            .sum();
        if delta != 0 {
            d.changed += 1;
        }
        total += delta as u64;
        histogram[delta as usize] += 1;
    }
    d.mean = total as f64 / d.covered.max(1) as f64;
    let cut = (d.covered as f64 * 0.999) as u32;
    let mut seen = 0;
    for (delta, count) in histogram.iter().enumerate() {
        seen += count;
        if seen >= cut {
            d.p999 = delta as u32;
            break;
        }
    }
    d
}

fn assert_coverage_is_identical(d: &Diff, width: u32, what: &str) {
    if let Some(idx) = d.coverage_mismatch {
        panic!(
            "{what}: half rate changed which pixels are covered, first at ({}, {}). \
             The raster is supposed to stay at full resolution — this is the one \
             thing #825 must never do.",
            idx as u32 % width,
            idx as u32 / width,
        );
    }
}

/// 🔴 The issue's central promise: a wall standing on the floor keeps its outline, and the pixels
/// along it keep taking their light from the surface they belong to.
#[test]
fn the_silhouette_stays_full_resolution() {
    let Some(mut r) = rig(4, true) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };

    let full = render_at(&mut r, true, ShadingRate::Full);
    let half = render_at(&mut r, true, ShadingRate::Half);
    let d = diff(&full, &half);
    assert_coverage_is_identical(&d, SIZE, "wall silhouette");
    assert!(
        d.p999 <= 40,
        "the worst 0.1% of pixels are {}/765 away from full rate, past the 17 a \
         correct reconstruction leaves. The upsample is taking light across the \
         silhouette.",
        d.p999,
    );
}

/// The other half of the bargain: the lighting inside that silhouette is an approximation, and on a
/// smoothly lit floor it has to be a close one.
#[test]
fn half_rate_tracks_the_full_rate_image() {
    let Some(mut r) = rig(4, false) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };

    let full = render_at(&mut r, true, ShadingRate::Full);
    let half = render_at(&mut r, true, ShadingRate::Half);
    let d = diff(&full, &half);
    assert_coverage_is_identical(&d, SIZE, "floor under a grid of point lights");

    assert!(
        d.changed > 0,
        "half rate produced the identical image — the rate did not take",
    );
    assert!(
        d.mean <= 2.4 && d.p999 <= 30,
        "half rate is mean {:.2}/765, p99.9 {}/765 from full rate on a smoothly \
         lit floor, past the 1.01 and 17 a correct reconstruction leaves. The \
         samples are being read from the wrong place, not blended too coarsely.",
        d.mean,
        d.p999,
    );
}

/// An odd screen has a rightmost quad with one real pixel in it, and a bottom one with one real
/// row. `div_ceil` on the dispatch and the clamp in the upsample are what keep those from being
/// dropped, and both are invisible at any even size.
#[test]
fn an_odd_screen_keeps_its_last_column() {
    let Some(mut r) = rig(4, false) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    const ODD: u32 = 199;
    r.stage.resize(&r.device, (ODD, ODD));

    let full = render_at(&mut r, true, ShadingRate::Full);
    let half = render_at(&mut r, true, ShadingRate::Half);
    let d = diff(&full, &half);
    assert_coverage_is_identical(&d, ODD, "199x199");
}

/// The fragment path shades inside its own raster, one invocation per covered pixel.
#[test]
fn the_fragment_path_refuses_a_reduced_rate() {
    let Some(mut r) = rig(1, false) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    assert!(r.stage.set_compute_shading(false) > 0);
    assert_eq!(
        r.stage.set_shading_rate(ShadingRate::Half),
        0,
        "the fragment path accepted a reduced shading rate",
    );
    assert_eq!(r.stage.shading_rate(), ShadingRate::Full);

    // Leaving the compute path must drop a rate that was already standing, and coming back must not
    // resurrect it. A quality setting that reappears without being asked for is a bug the player
    // experiences as the game changing its own settings.
    assert!(r.stage.set_compute_shading(true) > 0);
    assert!(r.stage.set_shading_rate(ShadingRate::Half) > 0);
    assert!(r.stage.set_compute_shading(false) > 0);
    assert!(r.stage.set_compute_shading(true) > 0);
    assert_eq!(r.stage.shading_rate(), ShadingRate::Full);
}
