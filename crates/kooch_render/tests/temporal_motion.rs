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

/// 🔴 Serialises the cases in this binary, and it is not tidiness.
///
/// `common` hands every test the SAME device — one per binary, by
/// `OnceLock`, to dodge the radv `request_adapter` race of #334 — so
/// running four cases at once means four threads recording and
/// submitting against one device. Under radv that segfaults the process
/// rather than failing a case, intermittently, and it passes reliably
/// under `--test-threads=1`, which is the worst possible way to be told.
///
/// Same pattern as `gpu_scopes.rs`, for the same reason.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Poisoned is fine: a panicking case leaves no GPU state behind that
/// the next one reads, and swallowing the panic here would hide the
/// case that actually failed behind a second, unrelated one.
fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|e| e.into_inner())
}

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

/// The 99th-percentile per-pixel difference.
///
/// 🔴 A mean will not do here, and the first version of this test used
/// one. A disocclusion touches the pixels a silhouette uncovers, which
/// is a small part of the frame; averaged over the whole image the
/// effect is a rounding error next to the lighting, and the assertion
/// passes whether the mask fires or not. Measured 0.75 without the mask
/// against 0.33 with it — a real 2.3x, and both comfortably inside any
/// threshold worth writing.
///
/// A percentile is safe here where `worst_pixel` was not, because both
/// frames come from the same jitter phase: nothing is compared across
/// the silhouette disagreement that sank the earlier version.
fn worst_percentile(a: &[u8], b: &[u8]) -> f64 {
    let mut per_pixel: Vec<u32> = a
        .chunks_exact(4)
        .zip(b.chunks_exact(4))
        .map(|(x, y)| {
            x[..3]
                .iter()
                .zip(&y[..3])
                .map(|(p, q)| p.abs_diff(*q) as u32)
                .sum()
        })
        .collect();
    per_pixel.sort_unstable();
    per_pixel[per_pixel.len() * 99 / 100] as f64
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
    let _gpu = gpu_lock();
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
    let _gpu = gpu_lock();
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
/// Where the camera sits when it is `distance` metres out, looking at
/// the same point. Moving along the view axis is what changes every
/// depth in the frame at once, which is what the disocclusion test is
/// written against.
fn camera_at_distance(distance: f32) -> ViewCamera {
    ViewCamera::looking_at(Vec3::new(0.0, 2.5, distance), Vec3::new(0.0, 0.5, 0.0))
}

/// Settles a resolve at `from` for `SETTLED` frames, then spends two
/// more at `to`, and returns the last one.
///
/// ⚠️ Builds and drops its own rig, so only one GPU device is alive at a
/// time. Holding three at once — which the obvious version of this test
/// did — segfaults the driver when the test binary runs its cases in
/// parallel, and passes on its own, which is the worst way to find out.
///
/// Every call renders the same number of frames, so every call ends on
/// the same jitter phase. Comparing across phases disagrees about the
/// coverage of every silhouette by up to a whole pixel and says nothing
/// about history — see the note above `a_pan_leaves_no_trail`.
fn arrive_from(from: f32, to: f32) -> Option<Vec<u8>> {
    const SETTLED: u32 = 20;
    let mut r = rig(3, true)?;
    setup(&mut r, true);
    for _ in 0..SETTLED {
        r.camera = camera_at_distance(from);
        draw(&mut r);
    }
    r.camera = camera_at_distance(to);
    draw(&mut r);
    Some(draw(&mut r))
}

/// 🔴 A jump in depth must throw the history away, not blend it in.
///
/// This is the case the variance clip cannot catch on its own: the
/// history at the reprojected address is a perfectly plausible colour
/// that belongs to a surface at a different distance. Without a
/// disocclusion test it is mixed in at ninety per cent and takes about
/// twenty frames to fade, which is the ghost people describe as "the
/// image swims when I move".
///
/// The yardstick is the scene's own: how far apart the two viewpoints
/// look. A resolve that dropped its history sits near the frame it
/// arrived at; one that kept it sits most of the way back at the frame
/// it left.
///
/// 🔴 The threshold is 2 % of that, and it was chosen by measuring both
/// sides rather than by taste: **6 with the mask, 44 without it**,
/// against 612 for the two viewpoints. Anything between them would
/// discriminate; 2 % leaves 2x of headroom above the passing value and
/// 3.6x below the failing one. The commit that stubs `is_disoccluded`
/// out has to fail this test, and it does.
#[test]
fn a_depth_jump_drops_the_history() {
    let _gpu = gpu_lock();
    const FAR: f32 = 9.0;
    const NEAR: f32 = 4.0;

    let Some(jumped) = arrive_from(FAR, NEAR) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    // The same camera, reached without ever jumping.
    let Some(native) = arrive_from(NEAR, NEAR) else {
        return;
    };
    // And what a whole frame of the past would be worth, if kept.
    let Some(left_behind) = arrive_from(FAR, FAR) else {
        return;
    };

    let viewpoints = worst_percentile(&native, &left_behind);
    let residue = worst_percentile(&jumped, &native);
    eprintln!("depth jump: residue {residue:.3}, viewpoints {viewpoints:.3} apart");
    assert!(
        viewpoints > 100.0,
        "the two viewpoints only differ by {viewpoints:.3}, so this test cannot tell a \
         dropped history from a kept one. Move them further apart.",
    );
    assert!(
        residue < viewpoints * 0.02,
        "one frame after jumping from {FAR} m to {NEAR} m the resolve is {residue:.3} \
         away from the same camera reached without jumping, and the two viewpoints are \
         only {viewpoints:.3} apart. The far camera's history is still being blended \
         in: the disocclusion test is not firing, and the variance clip cannot catch \
         this on its own because the stale colour is a plausible one.",
    );
}

/// 🔴 And ordinary motion must NOT trip it, which is the other half.
///
/// A tolerance set too tight rejects the history every frame the camera
/// moves. The resolve then costs two passes and returns the raw frame,
/// which reads as "TAA does nothing here" rather than as a bad constant
/// — and no capture would say otherwise, because both passes still run
/// and still take their milliseconds.
///
/// Measured as accumulation: consecutive resolved frames during a pan
/// are closer together than consecutive unresolved ones, because the
/// resolve is averaging the jitter out. If every frame rejected its
/// history the two would be equal.
#[test]
fn a_slow_pan_keeps_accumulating() {
    let _gpu = gpu_lock();
    let Some(mut r) = rig(3, true) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };

    let steps = 24;
    let mut step_apart = |taa: bool| {
        setup(&mut r, taa);
        let mut previous = Vec::new();
        for step in 0..steps {
            r.camera = camera_at(step);
            previous = draw(&mut r);
        }
        r.camera = camera_at(steps);
        let last = draw(&mut r);
        mean_difference(&previous, &last)
    };

    let raw = step_apart(false);
    let resolved = step_apart(true);
    eprintln!("per-step change: resolved {resolved:.3}, unresolved {raw:.3}");
    assert!(
        resolved < raw,
        "during a pan the resolved image changes {resolved:.3} per step and the \
         unresolved one changes {raw:.3}. The resolve is accumulating nothing, which \
         is what a disocclusion tolerance tight enough to fire on ordinary camera \
         motion looks like from the outside.",
    );
}
