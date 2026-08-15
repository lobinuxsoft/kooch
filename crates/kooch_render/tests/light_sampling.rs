//! #826's acceptance: evaluating 2 of a froxel's 15 lights must produce
//! a picture that is **noisier**, not **darker**.
//!
//! That sentence is the whole issue. `KOOCH_LIGHT_LIMIT` already
//! evaluates fewer lights and it is not this: it keeps a prefix of the
//! froxel's list and scales it by nothing, so two thirds of the light
//! simply leaves. Sampling picks in proportion to what a light
//! contributes and divides by the probability of the pick, so the few
//! that are evaluated stand in for the ones that were not.
//!
//! An assertion that only checked "close to the reference" would pass
//! for a shader that quietly walked every light, and one that only
//! checked "different from the reference" would pass for the limit. So
//! the tests below pin all three corners: **as bright as the full walk**,
//! **not identical to it**, and **darker only when the limit is used
//! instead**.
//!
//! Run with:
//!   cargo test -p kooch_render --test light_sampling

mod common;

use common::lit_scene::{render_at, rig, rig_with_caster};
use common::srgb_to_linear;
use kooch_lighting::{LightLimit, LightSamples};
use kooch_render::meshlet::ShadingRate;

/// Mean linear luminance over the covered pixels.
///
/// 🔴 Linear, through `srgb_to_linear`, and that is not pedantry: the
/// target is sRGB-encoded, so averaging the bytes weighs a dark pixel
/// far more than it weighs light. A 2x drop in radiance is about 1.4x in
/// bytes, and a test that averaged bytes would report a third of the
/// error it was looking for.
fn mean_luminance(pixels: &[u8]) -> f64 {
    let mut total = 0.0;
    let mut covered = 0usize;
    for p in pixels.chunks_exact(4) {
        if p[3] == 0 {
            continue;
        }
        covered += 1;
        total += 0.2126 * srgb_to_linear(p[0]) as f64
            + 0.7152 * srgb_to_linear(p[1]) as f64
            + 0.0722 * srgb_to_linear(p[2]) as f64;
    }
    assert!(covered > 0, "the scene rendered empty");
    total / covered as f64
}

/// Mean absolute channel difference over covered pixels, in 1/255. How
/// far an estimate is from the reference, noise included.
fn mean_delta(a: &[u8], b: &[u8]) -> f64 {
    let mut total = 0u64;
    let mut covered = 0usize;
    for (x, y) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        if x[3] == 0 || y[3] == 0 {
            continue;
        }
        covered += 1;
        total += x[..3]
            .iter()
            .zip(y[..3].iter())
            .map(|(p, q)| p.abs_diff(*q) as u64)
            .sum::<u64>();
    }
    total as f64 / (covered.max(1) * 3) as f64
}

/// 🔴 The issue's central claim, against the instrument that motivated
/// it.
///
/// Same number of lights evaluated, two ways. Sampling keeps the scene's
/// brightness because each pick is divided by its probability;
/// truncation does not, because it is divided by nothing. If this ever
/// fails in the direction of "sampled is dark too", the estimator lost
/// its weight and #826 has become #825's `light_limit` with more code.
#[test]
fn sampling_keeps_the_brightness_that_truncation_loses() {
    let Some(mut r) = rig(4, false) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };

    r.resources.insert(LightSamples(0));
    r.resources.insert(LightLimit(0));
    let full = mean_luminance(&render_at(&mut r, true, ShadingRate::Full));

    r.resources.insert(LightSamples(2));
    let sampled = mean_luminance(&render_at(&mut r, true, ShadingRate::Full));

    r.resources.insert(LightSamples(0));
    r.resources.insert(LightLimit(2));
    let limited = mean_luminance(&render_at(&mut r, true, ShadingRate::Full));

    let sampled_error = (sampled - full).abs() / full;
    let limited_error = (limited - full).abs() / full;
    eprintln!(
        "full {full:.5}  sampled(2) {sampled:.5} ({:.1} %)  limited(2) {limited:.5} ({:.1} %)",
        sampled_error * 100.0,
        limited_error * 100.0,
    );

    assert!(
        sampled_error < 0.08,
        "two sampled lights are {:.1} % away from the full walk's brightness. \
         The estimator is not dividing by the probability it picked with — it is \
         dropping light, which is what the instrument this replaces already did.",
        sampled_error * 100.0,
    );
    assert!(
        limited_error > 3.0 * sampled_error,
        "truncating to the same count cost only {:.1} % against sampling's {:.1} %. \
         Either the froxels hold too few lights for this scene to test anything, \
         or the two are doing the same thing.",
        limited_error * 100.0,
        sampled_error * 100.0,
    );
}

/// More samples must land closer to the answer, or what is being
/// measured is not convergence towards it.
///
/// This is the assertion that separates "unbiased and noisy" from
/// "wrong by a constant that happens to average out". A biased estimator
/// can match the mean brightness and still not improve with more
/// samples.
///
/// # 🔴 The measured curve, and the floor in it
///
/// Mean |Δ| against the full walk, and the luminance error, on the
/// parity scene:
///
/// Mean |Δ| against the full walk and the luminance error, on the parity
/// scene, with the choice made **per froxel** — and next to what the
/// same scene measured when it was made **per pixel**, because that is
/// the cost of the move:
///
/// Four estimators were built for this and three were measured wrong.
/// The columns are where the choice is made:
///
/// | samples | per pixel | per froxel | luminance only | **two stage** |
/// |---|---|---|---|---|
/// | 1 | 8.71 | 24.33 | 88.19 | **16.62** |
/// | 2 | 7.47 | 13.51 | 64.91 | **12.49** |
/// | 4 | 6.41 | 9.79 | 46.72 | **10.65** |
/// | 8 | 5.42 | 7.53 | 32.73 | **10.10** |
///
/// - **per pixel** is the best picture and the device refused it: the
///   weights cost 0.196 of an evaluation each and `(K+1) x 15` of them
///   made K=4 slower than walking all fifteen lights.
/// - **per froxel** moved the whole choice to the tile. Fast, and it
///   produced the chromatic blocking the handheld photographed.
/// - **luminance only** made the froxel's ranking stable across tiles by
///   deleting geometry from it. With every light in a scene at one
///   intensity that is uniform sampling, and it is the worst column here
///   by a factor of five.
/// - **two stage** is what shipped: the froxel proposes `2K` candidates
///   with geometry, the pixel resamples `K` of them with its own normal
///   and distance. Better than per-froxel exactly where a frame would
///   run it, and the final choice is per pixel again, so there is no
///   tile-shaped structure left to see.
///
/// ⚠️ **8 samples barely beats 4** because the froxel offers `2K`
/// candidates capped at `MAX_TILE_STRATA` = 8: at K=8 there are eight
/// candidates and eight kept, so the second stage has nothing to choose
/// between and degenerates into the first. The useful range is K = 1..4.
///
/// ⚠️ **It converges and then stops**, at 7.53/255 and −3.3 %. That floor
/// is real and it is not noise: a light whose cheap weight badly
/// underestimates what it actually contributes gets picked rarely and
/// scaled by a correspondingly large `1/w` when it is. The spike lands
/// above what the tonemap and an 8-bit target can carry, is clipped, and
/// the energy it was standing in for is lost. Always downwards, which is
/// why the residual is a darkening rather than a wobble — and deeper
/// here than per pixel for the same reason the deltas are, because a
/// tile-wide weight is a cruder estimate and its `1/w` is larger.
///
/// It is recorded rather than fixed because the fix is a better weight
/// or a clamped ratio, both of which trade one bias for another, and
/// neither is worth choosing before the device says how much of this is
/// visible at the two-to-four samples a frame would actually ship.
#[test]
fn more_samples_land_closer() {
    let Some(mut r) = rig(4, false) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };

    r.resources.insert(LightLimit(0));
    r.resources.insert(LightSamples(0));
    let full = render_at(&mut r, true, ShadingRate::Full);

    // The whole curve, printed, because the table in this comment is
    // only trustworthy if the run that would contradict it says so.
    let full_lum = mean_luminance(&full);
    let mut curve = Vec::new();
    for k in [1u32, 2, 4, 8, 16] {
        r.resources.insert(LightSamples(k));
        let shot = render_at(&mut r, true, ShadingRate::Full);
        let delta = mean_delta(&full, &shot);
        let lum = (mean_luminance(&shot) - full_lum) / full_lum * 100.0;
        eprintln!("  {k:2} samples: mean delta {delta:6.2}   luminance {lum:+.2} %");
        curve.push(delta);
    }
    let one = curve[0];
    let eight = curve[3];
    assert!(one > 0.0, "one sample matched the full walk exactly");
    assert!(
        eight < one * 0.75,
        "eight samples ({eight:.3}) are not meaningfully closer to the full walk \
         than one ({one:.3}), against the 0.62 measured. The estimate is not \
         converging on the sum it is supposed to estimate.",
    );
}

/// 🔴 The anti-flicker requirement — and the test that found out what
/// the flicker actually was.
///
/// The froxel shimmer `KOOCH_LIGHT_LIMIT` produced on the device was
/// written up as pixels crossing cell boundaries. It is not. This test
/// renders the same unchanged view twice, and before the sampling was
/// made order-independent it failed on a still camera:
///
/// | | pixels changed between two identical frames | worst channel |
/// |---|---|---|
/// | walking every light | 1 | 1 |
/// | `KOOCH_LIGHT_LIMIT=2` | 10 098 | 164 |
/// | sampling, first draft | 7 075 | 83 |
///
/// The grid fills each cell's run with `atomicAdd`, so the order inside
/// a cell is whichever thread got there first — different every frame,
/// with nothing moving. "The first two of the list" is therefore a
/// different pair of lights each frame, and so was the first draft's
/// stratified walk of the cumulative weight, because a cumulative walk
/// is an order.
///
/// The fix is that the choice is keyed on each light's **global index**
/// rather than its slot, so the run can be permuted at will. Nothing
/// about it may depend on the frame number either — seeding from a
/// frame counter is the obvious thing to write and would put the
/// shimmer straight back.
#[test]
fn the_same_view_samples_the_same_lights() {
    let Some(mut r) = rig(4, true) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    r.resources.insert(LightLimit(0));
    r.resources.insert(LightSamples(2));

    let first = render_at(&mut r, true, ShadingRate::Full);
    let second = render_at(&mut r, true, ShadingRate::Full);
    assert_eq!(
        first, second,
        "two renders of an unchanged view chose different lights. \
         Nothing about the sampling may depend on the frame number.",
    );
}

/// How much blue each 16x16 block has that the warm grid cannot account
/// for.
///
/// The grid is `(1.0, 0.9, 0.8)` and the caster `(0.05, 0.2, 1.0)`, so
/// this is a direct readout of one light's contribution with no way for
/// the others to fake it — and **per block**, because the block is the
/// unit the tile chooses in. An average over the frame would be blind to
/// the failure this looks for: a caster that survives in one tile of
/// sixteen keeps most of its energy in the mean and vanishes from the
/// picture.
fn blue_blocks(pixels: &[u8]) -> Vec<f64> {
    const TILE: usize = 16;
    let size = common::lit_scene::SIZE as usize;
    let blocks = size.div_ceil(TILE);
    let mut out = vec![0.0; blocks * blocks];
    for (b, cell) in out.iter_mut().enumerate() {
        let (bx, by) = ((b % blocks) * TILE, (b / blocks) * TILE);
        let mut total = 0.0;
        let mut covered = 0usize;
        for y in by..(by + TILE).min(size) {
            for x in bx..(bx + TILE).min(size) {
                let p = &pixels[(y * size + x) * 4..][..4];
                if p[3] == 0 {
                    continue;
                }
                covered += 1;
                total += (srgb_to_linear(p[2]) as f64 - srgb_to_linear(p[0]) as f64).max(0.0);
            }
        }
        *cell = if covered == 0 {
            0.0
        } else {
            total / covered as f64
        };
    }
    out
}

/// 🔴 A light that owns a shadow map is never sampled.
///
/// This is the rule the device asked for. A shadow is a binary,
/// high-contrast signal: a caster that a tile declines to pick does not
/// read as a slightly wrong estimate, it reads as **a shadow that
/// blinks** — the same artefact `KOOCH_LIGHT_LIMIT` produced, moved onto
/// the one feature where it is least forgivable.
///
/// The caster is measured **per block**, not per frame, because at one
/// sample a tile picks one light of seventeen: without the rule the
/// caster would keep most of its energy in the average and disappear
/// from fifteen blocks in sixteen. See `rig_with_caster` for why making
/// it dim instead does not work.
#[test]
fn a_shadow_caster_is_never_sampled_away() {
    let Some(mut r) = rig_with_caster(4) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    r.resources.insert(LightLimit(0));

    r.resources.insert(LightSamples(0));
    let full = blue_blocks(&render_at(&mut r, true, ShadingRate::Full));

    r.resources.insert(LightSamples(1));
    let sampled = blue_blocks(&render_at(&mut r, true, ShadingRate::Full));

    // The blocks the caster clearly reaches on the full walk. Anywhere
    // else it contributes nothing and losing nothing proves nothing.
    let peak = full.iter().cloned().fold(0.0f64, f64::max);
    let lit: Vec<usize> = (0..full.len()).filter(|&i| full[i] > peak * 0.25).collect();
    let kept = lit.iter().filter(|&&i| sampled[i] > full[i] * 0.25).count();
    let retention = kept as f64 / lit.len().max(1) as f64;

    eprintln!(
        "caster blocks: peak {peak:.4}, {} clearly lit, {kept} kept at one sample ({:.0} %)",
        lit.len(),
        retention * 100.0,
    );
    // Fifteen on this scene. Not a round number and not worth distorting
    // the scene to make one: without the rule the caster wins about one
    // race in seventeen, so the expected survivor count is 1. Fifteen
    // against 1 discriminates with room to spare.
    assert!(
        lit.len() >= 12,
        "only {} blocks are clearly lit by the caster; the scene cannot test the rule",
        lit.len(),
    );
    assert!(
        retention > 0.9,
        "one sample kept the caster in only {:.0} % of the blocks it lights. \
         A light with a shadow map entered the draw and lost, which on screen is \
         a shadow that disappears for whole tiles at a time.",
        retention * 100.0,
    );
}

/// Sampling and half rate have to compose: they attack different terms —
/// one the lights per pixel, the other the pixels — and the budget needs
/// both. A crash or a black frame here is the interaction failing.
#[test]
fn sampling_composes_with_half_rate() {
    let Some(mut r) = rig(4, false) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    r.resources.insert(LightLimit(0));
    r.resources.insert(LightSamples(0));
    let full = mean_luminance(&render_at(&mut r, true, ShadingRate::Half));

    r.resources.insert(LightSamples(2));
    let sampled = mean_luminance(&render_at(&mut r, true, ShadingRate::Half));

    let error = (sampled - full).abs() / full;
    eprintln!(
        "half rate: full {full:.5}, sampled(2) {sampled:.5} ({:.1} %)",
        error * 100.0
    );
    assert!(
        error < 0.08,
        "at half rate, two sampled lights are {:.1} % from the full walk",
        error * 100.0,
    );
}
