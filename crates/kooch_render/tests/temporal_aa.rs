//! Temporal anti-aliasing, asserted on the image it produces (#481).

mod common;

use common::lit_scene::{SIZE, rig};
use kooch_render::meshlet::ShadingRate;

/// Renders `frames` in sequence and hands back the last image.
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

/// Squared-gradient energy of `resolved` over that of `plain`, counting only the pairs that are
/// among the strongest `1 - percentile` of the **plain** image.
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
#[test]
fn a_silhouette_stops_being_a_step() {
    let Some(mut r) = rig(3, true) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    let plain = accumulate(&mut r, false, 4);
    let resolved = accumulate(&mut r, true, 24);

    // First, that anything happened at all. Without this the ratio could come out at 1.0 by the
    // resolve never having run, and the message below would send the reader looking for a subtle
    // averaging bug instead of a pass that is not in the frame.
    let moved = mean_difference(&plain, &resolved);
    assert!(
        moved > 0.1,
        "the resolved image differs from the plain one by {moved:.4} — it is the same \
         image. The temporal pass is not reaching the tonemap.",
    );

    for pct in [0.90, 0.99, 0.999] {
        let r = edge_energy_ratio(&plain, &resolved, pct);
        eprintln!("edge energy, strongest {:.1}%: {r:.3}", (1.0 - pct) * 100.0);
    }
    let ratio = edge_energy_ratio(&plain, &resolved, 0.99);
    assert!(
        ratio < 0.8,
        "the strongest edges carry {ratio:.3} of the energy they did unresolved, and \
         anti-aliasing them should approach half. The resolve is changing the image \
         without resolving anything — jitter that misses the projection, a history \
         that never survives a frame, or motion vectors that carry the jitter and \
         reproject it straight back out all look exactly like this.",
    );
}

/// And a still scene must not run away from itself.
#[test]
fn a_still_scene_does_not_diverge() {
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
    // Measured at 0.181. A divergent resolve is nowhere near this
    // number — it is tens of levels a frame, because its history is
    // being reprojected somewhere the image is not.
    assert!(
        late < 0.5,
        "twenty-three frames into a scene that is not moving, the image is still \
         changing by {late:.4} of a level per channel per frame (it was {early:.4} at \
         frame 3). That is not the clip's period-eight wobble, that is a history being \
         reprojected somewhere the image is not.",
    );
}

/// Off has to mean off, all the way down to the projection.
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
    // Not exactly zero, and the slack is measured rather than guessed: 0.000006 of a level per
    // channel, which is one pixel of the forty thousand landing on the other side of a
    // `textureAtomicMax` tie between two coplanar meshlets. That race predates this feature.
    assert!(
        delta < 1e-3,
        "two renders of a still scene with the resolve off differ by {delta:.6}. The \
         sub-pixel jitter is still being applied to the projection, which without a \
         resolve to integrate it is a frame that shimmers.",
    );
}

/// Writes plain / resolved / difference as binary PPMs for eyeballing. The tool that found the
/// posterisation, kept because it found it.
#[test]
#[ignore]
fn dump_frames() {
    let Some(mut r) = rig(3, true) else { return };
    let plain = accumulate(&mut r, false, 4);
    let resolved = accumulate(&mut r, true, 24);
    let dir = std::env::var("KOOCH_DUMP_DIR").unwrap_or_else(|_| "/tmp".into());
    let write = |name: &str, px: &[u8]| {
        let mut out = format!("P6\n{SIZE} {SIZE}\n255\n").into_bytes();
        out.extend(px.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]));
        std::fs::write(format!("{dir}/{name}.ppm"), out).expect("dump directory is writable");
    };
    write("taa_plain", &plain);
    write("taa_resolved", &resolved);
    let diff: Vec<u8> = plain
        .chunks_exact(4)
        .zip(resolved.chunks_exact(4))
        .flat_map(|(a, b)| {
            let f = |i: usize| (128 + (b[i] as i32 - a[i] as i32) * 4).clamp(0, 255) as u8;
            [f(0), f(1), f(2), 255]
        })
        .collect();
    write("taa_diff", &diff);
    eprintln!("wrote {dir}/taa_plain.ppm, taa_resolved.ppm, taa_diff.ppm");
}
