//! The shading path keeps radiance the display cannot show (#732).
//!
//! Moving the tonemap into its own pass is invisible to every other test
//! in this crate: `compute_shading_parity` compares the two shading
//! paths **after** both reach the same `Rgba8Unorm` view, so it passes
//! whether the intermediate is 16-bit float or 8-bit unorm. That is the
//! regression this file exists for — the whole point of the change is
//! the range between the two, and nothing else asserts it.
//!
//! Run with:
//!   cargo test -p kooch_render --test hdr_shading

mod common;

use common::lit_scene::{render_at, rig};
use kooch_lighting::{Exposure, LightLimit};
use kooch_render::meshlet::ShadingRate;

/// Pixels that are pure white — every channel saturated, nothing left to
/// tell them apart.
fn blown_out(pixels: &[u8]) -> Vec<usize> {
    pixels
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, p)| p[3] != 0 && p[0] == 255 && p[1] == 255 && p[2] == 255)
        .map(|(i, _)| i)
        .collect()
}

/// 🔴 What an 8-bit intermediate destroys, in one assertion.
///
/// The scene is exposed so far up that a large part of the floor clips
/// to pure white. Those pixels are *identical* on screen — but the
/// radiance behind them is not, and a float target still holds the
/// difference. Bringing the exposure down has to bring that difference
/// back.
///
/// With `HDR_COLOR_FORMAT` set back to `Rgba8Unorm` the shading would
/// clamp radiance at 1.0 on the way into the texture, every one of those
/// pixels would store the same value, and no exposure would ever
/// separate them again. The picture at the default exposure would look
/// the same, which is exactly why this needs its own test.
#[test]
fn detail_survives_above_what_the_display_can_show() {
    let Some(mut r) = rig(4, true) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    r.resources.insert(LightLimit(0));

    // Low EV100 is a wide-open shutter: the scene blows out.
    r.resources.insert(Exposure { ev100: 6.0 });
    let bright = render_at(&mut r, true, ShadingRate::Full);
    let clipped = blown_out(&bright);
    assert!(
        clipped.len() > 500,
        "only {} pixels clipped at EV100 6; the scene is not bright enough for this \
         test to be about anything",
        clipped.len(),
    );

    // Nine stops down. Everything that was white is now somewhere on the
    // curve, and where it lands is decided by radiance the display never
    // showed.
    r.resources.insert(Exposure { ev100: 15.0 });
    let dim = render_at(&mut r, true, ShadingRate::Full);

    let mut lowest = 255u8;
    let mut highest = 0u8;
    for &i in &clipped {
        let luma = dim[i * 4];
        lowest = lowest.min(luma);
        highest = highest.max(luma);
    }
    let spread = highest - lowest;
    eprintln!(
        "{} clipped pixels; nine stops down they span {lowest}..{highest} ({spread}/255)",
        clipped.len(),
    );
    assert!(
        spread > 16,
        "pixels that were all pure white came back spanning only {spread}/255. \
         The shading target is not carrying radiance above 1.0 — it is clamping, \
         which is what an 8-bit intermediate does and what this change removed.",
    );
}

/// The exposure reaches the pass that applies it.
///
/// It used to be read from the Inti uniform by the shading shader and is
/// now written into the tonemap pass's own buffer. A default that was
/// never overwritten, or a scalar plumbed but ignored, would leave every
/// other test in this crate passing — they all render at one exposure.
#[test]
fn exposure_reaches_the_tonemap() {
    let Some(mut r) = rig(4, false) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    r.resources.insert(LightLimit(0));

    r.resources.insert(Exposure { ev100: 9.0 });
    let base = render_at(&mut r, true, ShadingRate::Full);
    r.resources.insert(Exposure { ev100: 13.0 });
    let darker = render_at(&mut r, true, ShadingRate::Full);

    let mean = |p: &[u8]| {
        let (mut total, mut covered) = (0u64, 0usize);
        for px in p.chunks_exact(4) {
            if px[3] != 0 {
                covered += 1;
                total += px[0] as u64 + px[1] as u64 + px[2] as u64;
            }
        }
        assert!(covered > 0, "the scene rendered empty");
        total as f64 / (covered * 3) as f64
    };
    let (a, b) = (mean(&base), mean(&darker));
    eprintln!("EV100 9 mean {a:.1}, EV100 13 mean {b:.1}");
    assert!(
        b < a * 0.75,
        "four stops down changed the mean from {a:.1} to {b:.1}. The tonemap pass is \
         not reading the exposure it was handed.",
    );
}
