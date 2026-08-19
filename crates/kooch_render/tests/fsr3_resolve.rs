//! FSR 3.1 at a ratio of 1:1 (#481, the transliteration).
//!
//! 🎯 **The same oracle `sgsr2_resolve.rs` uses, for the same reason.**
//! A transliteration has nothing to diff against; at 1:1 a temporal
//! upscaler does not upscale, it resolves, so it can be run against the
//! resolve that already ships on the same frames. A port that is wrong
//! shows as a difference from a known-good image rather than as a vague
//! softness nobody can argue about.
//!
//! FSR has six dispatches where SGSR 2 has two, and five of them write
//! intermediates no eye ever sees. That makes the end-to-end assertion
//! MORE valuable here, not less: a reactivity mask that comes out NaN,
//! a lock that never decays, a luma history read at the wrong jitter —
//! all of them land in the final image and nowhere else.
//!
//! Run with:
//!   cargo test -p kooch_render --test fsr3_resolve

mod common;

/// Serialised for the reason the others are: `common` hands every case
/// the same device, and concurrent submission against it segfaults radv
/// rather than failing a case.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|e| e.into_inner())
}

use common::lit_scene::rig;
use glam::Vec3;
use kooch_render::ViewCamera;
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

/// 🔴 The six passes build a pipeline each, and this is the first thing
/// that ever creates them against a device.
///
/// Bind group layouts are checked against the shader at pipeline
/// creation, not at compile time — a binding declared `write` in WGSL
/// and `ReadWrite` in the layout, a sampled `R32Float` declared
/// filterable, a texture the entry point never uses — none of it is
/// visible until here. The image assertion below would also catch it,
/// by panicking, but it would not say which of the six.
#[test]
fn it_resolves_to_the_scene() {
    let _gpu = gpu_lock();
    let Some(plain) = settle(UpscaleTechnique::None, 1) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    let Some(fsr) = settle(UpscaleTechnique::Fsr3, 12) else {
        return;
    };

    let reference = mean_brightness(&plain);
    let resolved = mean_brightness(&fsr);
    eprintln!("brightness: fsr3 {resolved:.2}, unresolved {reference:.2}");
    assert!(
        reference > 1.0,
        "the unresolved frame is already black at {reference:.2}; the rig is broken, \
         not the technique",
    );
    assert!(
        (resolved - reference).abs() < reference * 0.25,
        "FSR 3.1 settled at mean brightness {resolved:.2} where the unresolved frame is \
         {reference:.2}. A history that never blends, an accumulation counter stuck at \
         zero, a NaN through the variance box, a tonemap that does not round-trip, or a \
         reprojection reading the wrong half of a ping-pong all land here.",
    );
}

/// 🔴 And it lands near the resolve that already ships, which is the
/// assertion that makes this a port rather than an experiment.
///
/// Two temporal techniques on a still camera, both settled, are not
/// identical — different kernels, different history weighting, and they
/// are supposed to differ or there would be no reason to have both. But
/// both are antialiasing the same scene, so they must be far closer to
/// each other than either is to a raw frame.
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
    let Some(fsr) = settle(UpscaleTechnique::Fsr3, 12) else {
        return;
    };

    let antialiasing = mean_difference(&plain, &taa);
    let between = mean_difference(&taa, &fsr);
    let fsr_effect = mean_difference(&plain, &fsr);
    eprintln!(
        "fsr3 vs taa {between:.3} | taa vs plain {antialiasing:.3} | fsr3 vs plain {fsr_effect:.3}",
    );
    assert!(
        antialiasing > 0.1,
        "the resolve barely changed the frame ({antialiasing:.3}), so this test cannot \
         tell a good port from a bad one",
    );
    assert!(
        fsr_effect > antialiasing * 0.5,
        "FSR 3.1 moved the frame {fsr_effect:.3} from unresolved where the engine's \
         resolve moves it {antialiasing:.3}. It is running and integrating almost \
         nothing: an accumulation counter that never climbs past its first frame, an \
         upsampled weight thresholded away, or a history read from the half that is \
         being written this frame.",
    );
    assert!(
        between < antialiasing,
        "FSR 3.1 and the engine's resolve differ by {between:.3}, and the whole effect \
         of resolving is only {antialiasing:.3}. Two techniques antialiasing the same \
         still scene must disagree by less than the antialiasing itself — a sign flip in \
         the reprojection, a dropped exposure, or a mis-set jitter all break this while \
         still producing a plausible-looking image.",
    );
}

/// Renders `frames` with the camera translating sideways, so that every
/// pixel carries a real motion vector, and returns the last frame.
///
/// 0.05 units per frame at nine units of distance is a few pixels of
/// screen motion — small enough that the history is still valid and
/// large enough that reprojecting it to the wrong place is visible.
fn settle_panning(technique: UpscaleTechnique, frames: u32) -> Option<Vec<u8>> {
    let mut r = rig(3, true)?;
    assert!(r.stage.set_compute_shading(true) > 0);
    r.stage.set_shading_rate(ShadingRate::Full);
    assert!(r.stage.set_upscale(technique) > 0);

    let mut last = Vec::new();
    for frame in 0..frames {
        let x = frame as f32 * 0.05;
        r.camera = ViewCamera::looking_at(Vec3::new(x, 2.5, 9.0), Vec3::new(x, 0.5, 0.0));
        r.stage
            .render_with_assets_primary(&r.device, &r.queue, &r.resources, &r.camera, 1.0);
        last = common::read_rgba8(&r.device, &r.queue, r.stage.color_texture());
    }
    Some(last)
}

/// 🔴 **The test the still-camera pair cannot be.**
///
/// FSR reprojects with `uv + mv`; this engine's motion buffer is signed
/// so history lives at `uv - mv`, and the port negates it on load. That
/// single character is the likeliest way for a transliteration to be
/// subtly wrong — and with a STILL camera the motion vectors are zero,
/// so the sign is unobservable and every other test in this file passes
/// with it flipped. Verified by flipping it.
///
/// Under motion, a reversed reprojection fetches history from twice the
/// wrong distance in the wrong direction, and the result smears away
/// from the frame the other techniques agree on.
#[test]
fn the_reprojection_tracks_the_camera() {
    let _gpu = gpu_lock();
    let Some(plain) = settle_panning(UpscaleTechnique::None, 12) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    let Some(taa) = settle_panning(UpscaleTechnique::Taa, 12) else {
        return;
    };
    let Some(fsr) = settle_panning(UpscaleTechnique::Fsr3, 12) else {
        return;
    };

    let antialiasing = mean_difference(&plain, &taa);
    let between = mean_difference(&taa, &fsr);
    eprintln!("panning: fsr3 vs taa {between:.3} | taa vs plain {antialiasing:.3}");
    assert!(
        antialiasing > 0.1,
        "the resolve barely changed the moving frame ({antialiasing:.3}), so this test \
         cannot tell a good reprojection from a reversed one",
    );
    assert!(
        between < antialiasing,
        "under camera motion FSR 3.1 differs from the engine's resolve by {between:.3} \
         where resolving itself only moves the frame {antialiasing:.3}. The motion \
         vectors are reaching the port with the wrong sign, or the history is being \
         sampled at the current frame's jitter instead of the previous one's.",
    );
}

/// Renders at `scale` percent of the window and returns the last frame.
fn settle_at_scale(technique: UpscaleTechnique, scale: u32, frames: u32) -> Option<Vec<u8>> {
    let mut r = rig(3, true)?;
    assert!(r.stage.set_compute_shading(true) > 0);
    r.stage.set_shading_rate(ShadingRate::Full);
    assert!(r.stage.set_upscale(technique) > 0);
    r.stage.set_render_scale(scale);
    r.stage.resize(
        &r.device,
        (common::lit_scene::SIZE, common::lit_scene::SIZE),
    );

    let mut last = Vec::new();
    for _ in 0..frames {
        r.stage
            .render_with_assets_primary(&r.device, &r.queue, &r.resources, &r.camera, 1.0);
        last = common::read_rgba8(&r.device, &r.queue, r.stage.color_texture());
    }
    Some(last)
}

/// 🔴 The reproduction: at 50 % the frame comes back black with sparse
/// bright speckles, and at 100 % it does not.
#[test]
fn it_survives_the_resolution_split() {
    let _gpu = gpu_lock();
    let Some(plain) = settle_at_scale(UpscaleTechnique::None, 100, 1) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    let Some(native) = settle_at_scale(UpscaleTechnique::Fsr3, 100, 12) else {
        return;
    };
    let Some(half) = settle_at_scale(UpscaleTechnique::Fsr3, 50, 12) else {
        return;
    };
    let Some(sgsr) = settle_at_scale(UpscaleTechnique::Sgsr2, 50, 12) else {
        return;
    };

    let reference = mean_brightness(&plain);
    eprintln!(
        "brightness: unresolved {reference:.2} | fsr3@100 {:.2} | fsr3@50 {:.2} | sgsr2@50 {:.2}",
        mean_brightness(&native),
        mean_brightness(&half),
        mean_brightness(&sgsr),
    );
    assert!(
        (mean_brightness(&half) - reference).abs() < reference * 0.35,
        "FSR 3.1 at 50 % settled at {:.2} where the unresolved frame is {reference:.2}",
        mean_brightness(&half),
    );
}
