//! The temporal resolve while the camera MOVES (#481).

mod common;

/// 🔴 Serialises the cases in this binary, and it is not tidiness.
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

/// 🔴 There is no `worst_pixel` here, and the reason is a failed test worth keeping.

/// 🔴 A pan must not drag the past along with it.
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
/// Where the camera sits when it is `distance` metres out, looking at the same point. Moving along
/// the view axis is what changes every depth in the frame at once, which is what the disocclusion
/// test is written against.
fn camera_at_distance(distance: f32) -> ViewCamera {
    ViewCamera::looking_at(Vec3::new(0.0, 2.5, distance), Vec3::new(0.0, 0.5, 0.0))
}

/// Settles a resolve at `from` for `SETTLED` frames, then spends two more at `to`, and returns the
/// last one.
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
