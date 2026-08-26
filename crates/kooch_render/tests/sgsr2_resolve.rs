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

/// 🔴 Step 4: the scene renders SMALLER than the window, and the
/// upscaler puts it back.
///
/// This is where the technique stops being an antialiaser and starts
/// paying for itself. Everything that costs per pixel — the visibility
/// buffer, the depth target, the Hi-Z pyramids, the shading dispatch —
/// shrinks with the scale; only what the blit presents does not.
///
/// Asserted on the TEXTURES rather than on the image, because that is
/// the claim: a frame that merely looks right could still be rendering
/// at full resolution and throwing the work away.
#[test]
fn the_scene_renders_smaller_than_the_window() {
    let _gpu = gpu_lock();
    let Some(mut r) = rig(3, true) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    let full = r.stage.depth_texture().size();

    // 🔴 Required, and it used to be absent: the scale needs the compute
    // path. The fragment one tonemaps inline into the window's image and
    // has nothing at render resolution to hold a smaller frame, so a
    // scale there mixes attachment sizes in one pass and wgpu discards
    // it. This test asked for the combination that produces that, and
    // passed, because it asserts on texture sizes rather than on a frame
    // reaching the screen.
    assert!(r.stage.set_compute_shading(true) > 0);
    assert!(r.stage.set_upscale(UpscaleTechnique::Sgsr2) > 0);
    r.stage.set_render_scale(50);
    // The editor calls this every frame with the panel's size; it is
    // where a change of scale turns into textures.
    r.stage.resize(
        &r.device,
        (common::lit_scene::SIZE, common::lit_scene::SIZE),
    );

    let depth = r.stage.depth_texture().size();
    let color = r.stage.color_texture().size();
    eprintln!(
        "at 50 %: depth {}x{}, presented {}x{} (native depth was {}x{})",
        depth.width, depth.height, color.width, color.height, full.width, full.height,
    );

    assert_eq!(
        (depth.width, depth.height),
        (full.width / 2, full.height / 2),
        "the depth target did not shrink, so nothing before the resolve got cheaper and \
         the scale is a setting that costs the upscale and buys nothing",
    );
    assert_eq!(
        (color.width, color.height),
        (full.width, full.height),
        "what the blit presents must stay at the window's size, or the upscaler is \
         resolving into a target as small as the one it read",
    );
}

/// And a technique that cannot reconstruct must be refused the scale.
///
/// TAA resolves at render resolution: handed a smaller frame it returns
/// a smaller frame, the blit stretches it, and the result is softer for
/// a saving the stretch gives back. Refused at the settings boundary
/// rather than documented as a footgun — see `quality.rs`.
#[test]
fn a_plain_resolve_is_refused_the_scale() {
    let _gpu = gpu_lock();
    let Some(mut r) = rig(3, true) else {
        eprintln!("no adapter; skipping");
        return;
    };
    let full = r.stage.depth_texture().size();

    assert!(r.stage.set_compute_shading(true) > 0);
    assert!(r.stage.set_upscale(UpscaleTechnique::Taa) > 0);
    r.stage.set_render_scale(50);
    r.stage.resize(
        &r.device,
        (common::lit_scene::SIZE, common::lit_scene::SIZE),
    );

    let depth = r.stage.depth_texture().size();
    assert_eq!(
        (depth.width, depth.height),
        (full.width, full.height),
        "TAA rendered at half size, which it cannot reconstruct from",
    );
}

/// 🔴 The froxel grid has to be sized from the RENDER resolution.
///
/// Found by the owner in the editor, from the picture: at 50 % the
/// lighting broke into blocks of wrong colour. The grid is indexed from
/// `frag_coord` by the shading pass, and that pass runs at render
/// resolution — sized to the window instead, every pixel reads a froxel
/// at twice its address, so half the grid is never consulted and the
/// other half is read crossed.
///
/// ⚠️ Asserted on the MAPPING, not on the image, and not on the grid's
/// dimensions either — those come from the aspect ratio and a fixed
/// cluster budget, so they are identical at both scales and cannot
/// catch this. The first version of this
/// test compared mean brightness and could not fail: with the bug in
/// place it moved 0.45 % against a 20 % threshold, because three lamps
/// covering the whole scene light it about the same however the froxels
/// are addressed. The defect needs a hundred localised lights to show
/// up in a mean — or one assertion on the number that is actually
/// wrong, which is this one.
#[test]
fn the_froxel_grid_follows_the_render_size() {
    let _gpu = gpu_lock();
    let side = common::lit_scene::SIZE;

    let grid = |scale: u32| -> Option<glam::Vec2> {
        let mut r = rig(3, true)?;
        assert!(r.stage.set_compute_shading(true) > 0);
        assert!(r.stage.set_upscale(UpscaleTechnique::Sgsr2) > 0);
        r.stage.set_render_scale(scale);
        r.stage.resize(&r.device, (side, side));
        r.stage
            .render_with_assets_primary(&r.device, &r.queue, &r.resources, &r.camera, 1.0);
        Some(r.stage.cluster_tile_factors())
    };

    let Some(native) = grid(100) else {
        eprintln!("no adapter; skipping");
        return;
    };
    let Some(halved) = grid(50) else { return };

    eprintln!("tile factors — native {native:?}, at 50 % {halved:?}");
    assert!(native.x > 0.0, "the native grid is degenerate");
    // Half the width means a fragment coordinate covers twice the grid
    // per pixel, so the factor doubles.
    assert!(
        (halved.x / native.x - 2.0).abs() < 0.1 && (halved.y / native.y - 2.0).abs() < 0.1,
        "the froxel mapping went from {native:?} to {halved:?} when the scene dropped to \
         half the width, where it should have doubled. \
         The shading pass indexes this grid from a fragment coordinate it produces at \
         RENDER resolution, so every pixel would read a froxel at twice its address — \
         half the grid never consulted, the other half read crossed, which is blocks of \
         wrong-coloured light and not a resolution artefact.",
    );
}
