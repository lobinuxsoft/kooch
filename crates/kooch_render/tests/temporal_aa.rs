//! Temporal anti-aliasing, asserted on the image it produces (#481).
//!
//! The unit tests next to the code check the two halves separately —
//! `jitter/tests.rs` that the offset is sub-pixel and evenly spread,
//! `motion_vectors.rs` that the vectors point the right way. Neither
//! would catch the failure this file exists for: both halves correct and
//! the pair wired together wrongly, which renders a plausible image that
//! anti-aliases nothing.
//!
//! Everything here reads the **final `Rgba8Unorm` image**, not the HDR
//! resolve. That is what the tonemap produced, which is what somebody
//! looking at the screen sees, and it means an assertion cannot pass
//! while the resolve is being written to a texture nobody reads.
//!
//! Run with:
//!   cargo test -p kooch_render --test temporal_aa

mod common;

use common::lit_scene::{SIZE, rig};
use kooch_render::meshlet::ShadingRate;

/// Renders `frames` in sequence and hands back the last image.
///
/// Sequence, not repetition: the whole mechanism is that frame *n* sees
/// what frames 0..n left behind. A test that rendered once and asserted
/// would be measuring the reset path, which is the one frame TAA does
/// nothing on.
fn accumulate(r: &mut common::lit_scene::Rig, taa: bool, frames: u32) -> Vec<u8> {
    assert!(
        r.stage.set_compute_shading(true) > 0,
        "no view has the R64 stage — every assertion here would be vacuous",
    );
    r.stage.set_shading_rate(ShadingRate::Full);
    assert!(
        r.stage.set_temporal_aa(taa) > 0,
        "no view took the temporal setting — the assertion would be vacuous",
    );
    let mut image = Vec::new();
    for _ in 0..frames {
        r.stage
            .render_with_assets_primary(&r.device, &r.queue, &r.resources, &r.camera, 1.0);
        image = common::read_rgba8(&r.device, &r.queue, r.stage.color_texture());
    }
    image
}

/// Mean absolute difference per colour channel, 0..255.
///
/// RGB only. Alpha is coverage, not colour, and including it would mix a
/// flag into a measurement of the picture.
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

/// Luminance difference between every horizontally and vertically
/// adjacent pair, in the same order for any image of this size.
fn gradients(image: &[u8]) -> Vec<f64> {
    let luma: Vec<f64> = image
        .chunks_exact(4)
        .map(|p| 0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64)
        .collect();
    let w = SIZE as usize;
    let mut out = Vec::with_capacity(2 * w * (w - 1));
    for y in 0..w {
        for x in 0..w - 1 {
            out.push((luma[y * w + x] - luma[y * w + x + 1]).abs());
        }
    }
    for y in 0..w - 1 {
        for x in 0..w {
            out.push((luma[y * w + x] - luma[(y + 1) * w + x]).abs());
        }
    }
    out
}

/// Squared-gradient energy of `resolved` over that of `plain`, counting
/// only the pairs that are among the strongest `1 - percentile` of the
/// **plain** image.
///
/// # 🔴 Why it is masked, and why the mask comes from the plain frame
///
/// Two earlier metrics measured nothing, both for the same reason, and
/// both are worth writing down. Counting "intermediate" pixels gave
/// 21595 against 21337 — a lit floor is already a gradient, so half the
/// frame qualifies with or without a resolve. Summing squared gradients
/// over the *whole* image gave 124 against 123, because the smooth
/// lighting falloff carries most of the total and the resolve rightly
/// leaves it alone.
///
/// Masking to the pixels that actually are edges separates the two, and
/// the mask is taken from the unresolved image so it cannot be shaped by
/// the thing being measured. Squared, because that is what turns
/// anti-aliasing into a number: one step of `d` carries `d²`, the same
/// step spread over two pixels carries `2(d/2)² = d²/2`. Halved, every
/// time, with no threshold to argue about.
///
/// Measured on the lit scene, resolved over plain:
///
///     top 10 %    0.77
///     top 1 %     0.62
///     top 0.1 %   0.46
///
/// The last is that theoretical halving, on the pixels where a step
/// really is a step.
fn edge_energy_ratio(plain: &[u8], resolved: &[u8], percentile: f64) -> f64 {
    let before = gradients(plain);
    let after = gradients(resolved);
    let mut sorted = before.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("luminance is never NaN"));
    let threshold = sorted[(sorted.len() as f64 * percentile) as usize];

    let mut plain_energy = 0.0;
    let mut resolved_energy = 0.0;
    for (b, a) in before.iter().zip(&after) {
        if *b >= threshold {
            plain_energy += b * b;
            resolved_energy += a * a;
        }
    }
    resolved_energy / plain_energy
}

/// 🔴 The assertion the whole feature is for.
///
/// A silhouette rendered once is a step: the pixel is the object or it
/// is the background, with nothing in between for the eye to read as a
/// smooth line. Averaging jittered frames turns each step into a ramp.
///
/// This fails for every wiring mistake at once — jitter that never
/// reaches the projection, a history that is cleared every frame, motion
/// vectors carrying the jitter and cancelling it, a resolve written to a
/// texture the tonemap does not read. All four produce an image
/// identical to the untouched one, and only this says so.
#[test]
fn a_silhouette_stops_being_a_step() {
    let Some(mut r) = rig(3, true) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    let plain = accumulate(&mut r, false, 4);
    let resolved = accumulate(&mut r, true, 24);

    // First, that anything happened at all. Without this the ratio could
    // come out at 1.0 by the resolve never having run, and the message
    // below would send the reader looking for a subtle averaging bug
    // instead of a pass that is not in the frame.
    let moved = mean_difference(&plain, &resolved);
    assert!(
        moved > 0.1,
        "the resolved image differs from the plain one by {moved:.4} — it is the same \
         image. The temporal pass is not reaching the tonemap.",
    );

    let ratio = edge_energy_ratio(&plain, &resolved, 0.99);
    eprintln!("edge energy on the strongest 1%: {ratio:.3} of the unresolved frame");
    assert!(
        ratio < 0.8,
        "the strongest edges carry {ratio:.3} of the energy they did unresolved, and \
         anti-aliasing them should approach half. The resolve is changing the image \
         without resolving anything — jitter that misses the projection, a history \
         that never survives a frame, or motion vectors that carry the jitter and \
         reproject it straight back out all look exactly like this.",
    );
}

/// And it has to settle.
///
/// A resolve that keeps moving on a scene that does not is the other
/// failure mode — jitter reaching the image without the history
/// averaging it away — and it looks like the frame quietly vibrating.
/// The blend rate is confidence-weighted, so a still pixel should be
/// changing far less by frame 24 than by frame 4.
#[test]
fn a_still_scene_settles() {
    let Some(mut r) = rig(3, true) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    assert!(r.stage.set_compute_shading(true) > 0);
    r.stage.set_shading_rate(ShadingRate::Full);
    assert!(r.stage.set_temporal_aa(true) > 0);

    let mut frames = Vec::new();
    for _ in 0..24 {
        r.stage
            .render_with_assets_primary(&r.device, &r.queue, &r.resources, &r.camera, 1.0);
        frames.push(common::read_rgba8(
            &r.device,
            &r.queue,
            r.stage.color_texture(),
        ));
    }

    let early = mean_difference(&frames[2], &frames[3]);
    let late = mean_difference(&frames[22], &frames[23]);
    eprintln!("frame-to-frame change: {early:.4} early, {late:.4} late");
    assert!(
        late <= early,
        "the image is changing MORE at frame 23 ({late:.4}) than at frame 3 \
         ({early:.4}). The jitter is reaching the projection and the history is not \
         averaging it out — a frame that vibrates rather than one that resolves.",
    );
}

/// Off has to mean off, all the way down to the projection.
///
/// The jitter and the resolve are one switch precisely so that half of
/// the pair cannot be left on, and this is what checks the switch
/// reaches both: with TAA off the same still scene must render to the
/// same bytes twice, which a jittered projection makes impossible.
#[test]
fn nothing_moves_with_the_resolve_off() {
    let Some(mut r) = rig(3, true) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    let first = accumulate(&mut r, false, 3);
    let second = accumulate(&mut r, false, 1);
    let delta = mean_difference(&first, &second);
    eprintln!("mean difference with TAA off: {delta:.6}");
    assert_eq!(
        first.len(),
        (SIZE * SIZE * 4) as usize,
        "the readback is not the frame it claims to be",
    );
    // Not exactly zero, and the slack is measured rather than guessed:
    // 0.000006 of a level per channel, which is one pixel of the forty
    // thousand landing on the other side of a `textureAtomicMax` tie
    // between two coplanar meshlets. That race predates this feature. A
    // jittered projection moves every silhouette in the frame and scores
    // three orders of magnitude above it.
    assert!(
        delta < 1e-3,
        "two renders of a still scene with the resolve off differ by {delta:.6}. The \
         sub-pixel jitter is still being applied to the projection, which without a \
         resolve to integrate it is a frame that shimmers.",
    );
}
