//! Motion vectors say where a surface was, not where a pixel was (#481).
//!
//! Every temporal technique on the roadmap reads this one texture — TAA,
//! FSR, DLSS, XeSS, motion blur — and all of them fail the same way when
//! it is wrong: a smear that looks like a bug in the technique rather
//! than in its input. So the assertions below are about the input.
//!
//! Run with:
//!   cargo test -p kooch_render --test motion_vectors

mod common;

use common::lit_scene::{SIZE, rig};
use glam::Vec3;
use kooch_render::ViewCamera;
use kooch_render::meshlet::ShadingRate;

/// Reads the `Rg16Float` target back as `(u, v)` pairs.
fn read_motion(r: &common::lit_scene::Rig) -> Vec<(f32, f32)> {
    let texture = r
        .stage
        .motion_vector_texture()
        .expect("the R64 path owns the motion target");
    // `Rg16Float` is four bytes a pixel, the same as `Rgba8Unorm`, so the
    // existing readback moves the right number of rows.
    let raw = common::read_rgba8(&r.device, &r.queue, texture);
    raw.chunks_exact(4)
        .map(|px| {
            (
                half_to_f32(u16::from_le_bytes([px[0], px[1]])),
                half_to_f32(u16::from_le_bytes([px[2], px[3]])),
            )
        })
        .collect()
}

/// IEEE 754 binary16 → f32. Three lines rather than a dependency, and
/// the subnormal branch matters here: a motion vector on a nearly-still
/// camera lives entirely down there.
fn half_to_f32(bits: u16) -> f32 {
    let sign = ((bits >> 15) as u32) << 31;
    let exp = ((bits >> 10) & 0x1f) as u32;
    let mant = (bits & 0x3ff) as u32;
    let out = match exp {
        0 if mant == 0 => sign,
        0 => {
            // Subnormal: normalise it by hand.
            let mut e = -1i32;
            let mut m = mant;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | (((127 - 15 + e + 1) as u32) << 23) | ((m & 0x3ff) << 13)
        }
        0x1f => sign | 0x7f80_0000 | (mant << 13),
        _ => sign | ((exp + 127 - 15) << 23) | (mant << 13),
    };
    f32::from_bits(out)
}

fn render(r: &mut common::lit_scene::Rig) -> Vec<(f32, f32)> {
    r.stage.set_compute_shading(true);
    r.stage.set_shading_rate(ShadingRate::Full);
    // 🔴 The pass is GATED on there being a consumer (#868) — writing
    // velocity nobody reads cost 1.994 ms of a 20.5 ms frame. Without
    // this line the texture is never written, every assertion below
    // reads zeros, and the failure reads as "the pass is broken" rather
    // than "the pass did not run".
    //
    // ⚠️ This file was left behind when the gate landed: it asserts on a
    // pass it did not switch on, so both of its moving-camera cases went
    // red and stayed red. A gate has to be added to every test that
    // depends on what it gates.
    assert!(
        r.stage.set_temporal_aa(true) > 0,
        "no view took the temporal setting — the motion pass would not run at all",
    );
    r.stage
        .render_with_assets_primary(&r.device, &r.queue, &r.resources, &r.camera, 1.0);
    read_motion(r)
}

/// The longest vector anywhere in the frame, in UV units.
fn peak(vectors: &[(f32, f32)]) -> f32 {
    vectors
        .iter()
        .map(|(u, v)| (u * u + v * v).sqrt())
        .fold(0.0f32, f32::max)
}

/// 🔴 Nothing moved, so nothing moved.
///
/// This is the assertion that catches the whole family of mistakes at
/// once: a previous transform keyed by array position instead of by
/// entity, a previous view-projection that was never stored, a jittered
/// matrix where an unjittered one belongs. Every one of them produces a
/// non-zero vector on a scene that did not move, and every one of them
/// looks like ghosting three features later.
///
/// Two renders, because the first frame has no history and reprojects
/// against itself — which is zero for a trivial reason rather than the
/// real one.
#[test]
fn a_still_camera_produces_no_motion() {
    let Some(mut r) = rig(3, true) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    let _ = render(&mut r);
    let second = render(&mut r);

    let worst = peak(&second);
    eprintln!("still frame, longest vector {worst:.6} uv");
    assert!(
        worst < 1e-4,
        "a scene that did not move produced a vector {worst:.6} UV long. Either the \
         previous transform belongs to another object, or the previous camera was \
         never the one that rendered the last frame.",
    );
}

/// And the converse, or the test above passes on a texture of zeros.
#[test]
fn a_moving_camera_produces_motion() {
    let Some(mut r) = rig(3, true) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    let _ = render(&mut r);
    let _ = render(&mut r);

    // Sideways, so the vectors are dominated by one axis and a sign
    // error cannot hide in a diagonal.
    r.camera = ViewCamera::looking_at(Vec3::new(1.5, 2.5, 9.0), Vec3::new(0.0, 0.5, 0.0));
    let moved = render(&mut r);

    let worst = peak(&moved);
    let covered = moved
        .iter()
        .filter(|(u, v)| u.abs() + v.abs() > 1e-4)
        .count();
    eprintln!(
        "after a 1.5 m step: longest {worst:.4} uv, {covered} of {} pixels moved",
        SIZE * SIZE,
    );
    assert!(
        worst > 0.01,
        "the camera moved 1.5 m and the longest vector is {worst:.6} UV. The pass is \
         writing zeros — the previous view-projection is being overwritten before it \
         is read, or the history is reset every frame.",
    );
    assert!(
        covered > (SIZE * SIZE / 20) as usize,
        "only {covered} pixels registered motion. The vectors exist but reach almost \
         nothing, which is a pass dispatched over the wrong extent.",
    );
}

/// A vector points where the surface *came from*, and its sign is what a
/// temporal resolve uses to walk backwards into the history. Getting it
/// inverted still produces plausible-looking magnitudes, and then every
/// temporal effect smears in the wrong direction.
///
/// # 🔴 The convention, derived rather than guessed
///
/// Bevy's resolve reads `history_uv = uv - motion_vector`
/// (`taa.wesl:124`), and every upscaler that consumes this texture wants
/// the same. Move the camera right and a static surface slides LEFT on
/// screen, so its previous UV was to the **right** — larger. For
/// `uv - motion` to land there, `motion.u` has to be **negative**.
///
/// This test was written asserting the opposite and the code was right:
/// measured −0.2253. The sign is recorded here with its derivation
/// precisely so the next person to find it surprising checks the
/// convention instead of flipping the shader.
#[test]
fn the_vector_points_where_the_surface_came_from() {
    let Some(mut r) = rig(3, true) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    let _ = render(&mut r);
    let _ = render(&mut r);
    r.camera = ViewCamera::looking_at(Vec3::new(1.5, 2.5, 9.0), Vec3::new(1.5, 0.5, 0.0));
    let moved = render(&mut r);

    let mut sum = 0.0f64;
    let mut counted = 0usize;
    for (u, _) in &moved {
        if u.abs() > 1e-3 {
            sum += *u as f64;
            counted += 1;
        }
    }
    assert!(counted > 100, "too few moving pixels to judge a direction");
    let mean = sum / counted as f64;
    eprintln!("mean U over {counted} moving pixels: {mean:+.4}");
    assert!(
        mean < 0.0,
        "the camera moved right and the mean U is {mean:+.4}, where the convention \
         needs it negative: a resolve reads `history_uv = uv - motion`, and a surface \
         that slid left was previously to the right. Positive here makes every \
         temporal pass reproject away from its own history.",
    );
}
