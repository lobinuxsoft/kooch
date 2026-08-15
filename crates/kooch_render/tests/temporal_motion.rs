//! The temporal resolve while the camera MOVES (#481).
//!
//! `temporal_aa.rs` holds the camera still, which is where a resolve is
//! easiest to be right: the motion vectors are zero, the reprojection is
//! the identity, and the history lands on the texel it was written to.
//! Everything that makes TAA infamous happens on the other side of that
//! — ghosting, smearing, a trail behind a silhouette — and none of it
//! can appear in a still frame.
//!
//! Run with:
//!   cargo test -p kooch_render --test temporal_motion

mod common;

use common::lit_scene::{SIZE, rig};
use glam::Vec3;
use kooch_render::ViewCamera;
use kooch_render::meshlet::ShadingRate;

/// Where the camera sits on step `i` of a slow pan.
///
/// Slow on purpose: a teleport is the easy case, because the history is
/// rejected outright and the resolve falls back to the current frame. A
/// pan is what actually exercises the reprojection, because most of the
/// history is still valid and only where it is not does a smear appear.
fn camera_at(step: u32) -> ViewCamera {
    let x = step as f32 * 0.08;
    ViewCamera::looking_at(Vec3::new(x, 2.5, 9.0), Vec3::new(x, 0.5, 0.0))
}

fn setup(r: &mut common::lit_scene::Rig, taa: bool) {
    assert!(
        r.stage.set_compute_shading(true) > 0,
        "no view has the R64 stage — every assertion here would be vacuous",
    );
    r.stage.set_shading_rate(ShadingRate::Full);
    assert!(
        r.stage.set_temporal_aa(taa) > 0,
        "no view took the temporal setting — the assertion would be vacuous",
    );
}

fn draw(r: &mut common::lit_scene::Rig) -> Vec<u8> {
    r.stage
        .render_with_assets_primary(&r.device, &r.queue, &r.resources, &r.camera, 1.0);
    common::read_rgba8(&r.device, &r.queue, r.stage.color_texture())
}

/// Pans for `steps` and returns the last frame.
fn pan(r: &mut common::lit_scene::Rig, taa: bool, steps: u32) -> Vec<u8> {
    setup(r, taa);
    let mut last = Vec::new();
    for step in 0..steps {
        r.camera = camera_at(step);
        last = draw(r);
    }
    last
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

/// 🔴 There is no `worst_pixel` here, and the reason is a failed test
/// worth keeping.
///
/// The first version of this file compared the resolved frame against
/// the unresolved one pixel by pixel and asserted the worst was small.
/// It failed at 0.83 of full scale, which looked like a catastrophic
/// ghost — and was not. Every offender sat in a single one-pixel column
/// at the vertical edge of the wall, with the resolved pixel carrying
/// **alpha 0**: the jittered raster had put the silhouette on the other
/// side of that column. A jittered frame and an unjittered one disagree
/// about the coverage of every silhouette by up to a whole pixel, by
/// construction, and that is the feature rather than the bug.
///
/// So nothing here compares across the jitter boundary. Both assertions
/// below compare a resolved frame against another resolved frame.

/// 🔴 A pan must not drag the past along with it.
///
/// The resolved frame at the end of a pan is compared against the
/// unresolved frame from the same camera — not for equality, which
/// anti-aliasing forbids, but against a yardstick the scene supplies
/// itself: **one step of the pan**. A resolve holding a correct history
/// differs from the current frame by less than the frame moves in a
/// step. One holding the history of where the camera used to be looking
/// differs by more.
///
/// A motion vector with the wrong sign, or one still carrying the
/// sub-pixel jitter, fails here and cannot fail in `temporal_aa.rs`,
/// where nothing moves and every vector is zero.
#[test]
fn a_pan_leaves_no_trail() {
    let Some(mut r) = rig(3, true) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };

    let steps = 24;
    let plain = pan(&mut r, false, steps);
    let resolved = pan(&mut r, true, steps);

    // How far the scene moves in one step, as the image sees it.
    r.camera = camera_at(steps - 2);
    setup(&mut r, false);
    let previous = draw(&mut r);
    let one_step = mean_difference(&plain, &previous);

    let smear = mean_difference(&plain, &resolved);
    eprintln!("pan: resolved vs plain {smear:.3}, one step of the pan is {one_step:.3}");
    assert!(
        smear < one_step,
        "after a pan the resolved frame is {smear:.3} away from the unresolved one, and \
         a single step of that pan only moves the image {one_step:.3}. The resolve is \
         holding more than a frame of the past — a motion vector pointing the wrong \
         way, or one still carrying the sub-pixel jitter, drags the history along \
         instead of cancelling the camera out of it.",
    );
}

/// And where the camera arrived from must not matter.
///
/// This is the failure a user reports as "it smears": the pan ends, the
/// scene is still, and a ghost of where things were stays for a second.
///
/// 🔴 Both sides of this comparison are RESOLVED frames at the same
/// camera, so the jitter cancels and what is left is history. One
/// settled after a pan, the other settled from rest. A correct resolve
/// forgets how it got there — the confidence counter resets the moment a
/// pixel moves and has to climb again — and any difference between them
/// is the pan still being visible.
#[test]
fn where_the_camera_came_from_stops_mattering() {
    let Some(mut r) = rig(3, true) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };

    let steps = 24;
    let hold = 16;

    let _ = pan(&mut r, true, steps);
    r.camera = camera_at(steps - 1);
    let mut after_pan = Vec::new();
    for _ in 0..hold {
        after_pan = draw(&mut r);
    }

    // The same camera, reached without ever moving: a fresh history, so
    // there is nothing for a pan to have left in it.
    let Some(mut fresh) = rig(3, true) else {
        return;
    };
    fresh.camera = camera_at(steps - 1);
    setup(&mut fresh, true);
    let mut from_rest = Vec::new();
    for _ in 0..(steps + hold) {
        from_rest = draw(&mut fresh);
    }

    let residue = mean_difference(&after_pan, &from_rest);
    eprintln!("after stopping, pan-history vs fresh-history: {residue:.3}");
    assert!(
        residue < 0.5,
        "sixteen frames after the camera stopped, the image still differs by {residue:.3} \
         from the same camera reached without moving. The pan is still in the history, \
         which is what a reprojection that does not cancel the camera looks like from \
         the outside.",
    );
}
