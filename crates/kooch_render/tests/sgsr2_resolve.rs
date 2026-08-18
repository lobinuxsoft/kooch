//! SGSR 2 at a ratio of 1:1 (#481, step 4).
//!
//! 🎯 **This file is the oracle the plan said a transliteration would
//! not have.** The risk written into #481 was that a port has nothing to
//! diff against and degenerates into "it looks wrong and I do not know
//! why". At 1:1 SGSR 2 does not upscale — it resolves — so it can be
//! run against the resolve that already ships, on the same frames, and
//! a port that is wrong shows as a difference from a known-good image.
//!
//! What it does NOT test is the resolution split, which is step 4 and is
//! not built. Separating those two questions is the point.
//!
//! Run with:
//!   cargo test -p kooch_render --test sgsr2_resolve

mod common;

/// Serialised for the reason `contact_shadows` and `temporal_motion`
/// are: `common` hands every case the same device, and concurrent
/// submission against it segfaults radv rather than failing a case.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|e| e.into_inner())
}

use common::lit_scene::rig;
use kooch_render::meshlet::ShadingRate;
use kooch_render::quality::UpscaleTechnique;

/// Settles `technique` for `frames` on a still camera and returns the
/// last frame.
fn settle(technique: UpscaleTechnique, frames: u32) -> Option<Vec<u8>> {
    let mut r = rig(3, true)?;
    assert!(r.stage.set_compute_shading(true) > 0);
    r.stage.set_shading_rate(ShadingRate::Full);
    assert!(
        r.stage.set_upscale(technique) > 0,
        "no view took the technique — every assertion would be vacuous",
    );
    let mut last = Vec::new();
    for _ in 0..frames {
        r.stage
            .render_with_assets_primary(&r.device, &r.queue, &r.resources, &r.camera, 1.0);
        last = common::read_rgba8(&r.device, &r.queue, r.stage.color_texture());
    }
    Some(last)
}

fn mean_difference(a: &[u8], b: &[u8]) -> f64 {
    let sum: u64 = a
        .chunks_exact(4)
        .zip(b.chunks_exact(4))
        .map(|(x, y)| {
            x[..3]
                .iter()
                .zip(&y[..3])
                .map(|(p, q)| p.abs_diff(*q) as u64)
                .sum::<u64>()
        })
        .sum();
    sum as f64 / (a.len() / 4 * 3) as f64
}

fn mean_brightness(a: &[u8]) -> f64 {
    let sum: u64 = a
        .chunks_exact(4)
        .map(|p| p[..3].iter().map(|c| *c as u64).sum::<u64>())
        .sum();
    sum as f64 / (a.len() / 4 * 3) as f64
}

/// 🔴 The transliteration produces an image, and it is the scene's.
///
/// The failure modes a bad port actually has are loud: a black frame
/// (the history never blends, or `expand` divides by an exposure of
/// zero), a white one (the range compressor fed raw radiance), NaN
/// propagated through the variance box, or a one-pixel black border rung
/// in by `textureLoad` returning zero out of range.
///
/// Every one of those moves the mean brightness far off the unresolved
/// frame's. Nothing subtle is claimed here — that is what the next test
/// is for.
#[test]
fn it_resolves_to_the_scene() {
    let _gpu = gpu_lock();
    let Some(plain) = settle(UpscaleTechnique::None, 1) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    let Some(sgsr) = settle(UpscaleTechnique::Sgsr2, 12) else {
        return;
    };

    let reference = mean_brightness(&plain);
    let resolved = mean_brightness(&sgsr);
    eprintln!("brightness: sgsr2 {resolved:.2}, unresolved {reference:.2}");
    assert!(
        reference > 1.0,
        "the unresolved frame is already black at {reference:.2}; the rig is broken, \
         not the technique",
    );
    assert!(
        (resolved - reference).abs() < reference * 0.25,
        "SGSR 2 settled at mean brightness {resolved:.2} where the unresolved frame is \
         {reference:.2}. A port that blends nothing, divides by a zero exposure, feeds \
         raw radiance to the range compressor, or rings a black border in from an \
         out-of-range textureLoad all land here.",
    );
}

/// 🔴 And it lands near the resolve that already ships, which is the
/// assertion that makes this a port rather than an experiment.
///
/// Two temporal techniques on a still camera, both settled, are not
/// identical — different kernels, different history weighting, and they
/// are supposed to differ or there would be no reason to have both. But
/// they are both antialiasing the same scene, so they must be far closer
/// to each other than either is to a raw frame.
///
/// The yardstick is the scene's own: how far the unresolved frame sits
/// from the settled resolve. That distance IS the antialiasing, and two
/// techniques doing the same job must disagree by less than the job.
#[test]
fn it_lands_near_the_engines_resolve() {
    let _gpu = gpu_lock();
    let Some(plain) = settle(UpscaleTechnique::None, 1) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    let Some(taa) = settle(UpscaleTechnique::Taa, 12) else {
        return;
    };
    let Some(sgsr) = settle(UpscaleTechnique::Sgsr2, 12) else {
        return;
    };

    let antialiasing = mean_difference(&plain, &taa);
    let between = mean_difference(&taa, &sgsr);
    let sgsr_effect = mean_difference(&plain, &sgsr);
    eprintln!(
        "sgsr2 vs taa {between:.3} | taa vs plain {antialiasing:.3} | sgsr2 vs plain {sgsr_effect:.3}",
    );
    assert!(
        sgsr_effect > antialiasing * 0.5,
        "SGSR 2 moved the frame {sgsr_effect:.3} from unresolved where the engine's \
         resolve moves it {antialiasing:.3}. It is running and integrating almost \
         nothing: alpha pinned near 1 (a depth-clip mask stuck at 1, so base_alpha is 0), \
         a history that never reaches the blend, or a jitter of zero reaching the pass.",
    );
    assert!(
        antialiasing > 0.1,
        "the resolve barely changed the frame ({antialiasing:.3}), so this test cannot \
         tell a good port from a bad one",
    );
    assert!(
        between < antialiasing,
        "SGSR 2 and the engine's resolve differ by {between:.3}, and the whole effect of \
         resolving is only {antialiasing:.3}. Two techniques antialiasing the same still \
         scene must disagree by less than the antialiasing itself — a sign flip in the \
         reprojection, a dropped exposure, or a mis-set jitter all break this while \
         still producing a plausible-looking image.",
    );
}
