//! The motion-vector pass runs only when something reads it (#481).
//!
//! Measured on the OneXFly before this gate existed: **1.994 ms of a
//! 20.5 ms GPU frame**, writing a full-resolution buffer with the
//! temporal resolve off and therefore with no consumer at all —
//! `taa.wgsl` is the only shader that binds it.
//!
//! # Why this is its own test binary
//!
//! The obvious test is "the `motion vectors` GPU scope is absent", and
//! it is unsound in a shared binary. puffin's `scope_delta` is a delta:
//! whichever test first causes a scope to register drains the name from
//! the list, so a later test's `FrameView` never sees it and asserting
//! absence would be asserting which test ran first. `gpu_scopes.rs` says
//! so in as many words about its own assertions.
//!
//! So this reads **the texture** instead of the profiler. A freshly
//! allocated wgpu texture is zeroed, so "no pass ever wrote here" is
//! observable without a second frame to compare against — and the
//! resolve-on half proves the readback and the pass both work, which is
//! what stops the resolve-off half from passing vacuously.

mod common;

use glam::Vec3;
use kooch_render::ViewCamera;

/// Two camera positions: the vectors are the difference between them, so
/// a single still frame would write zeros whatever the gate did.
fn camera_at(step: u32) -> ViewCamera {
    ViewCamera::looking_at(Vec3::new(step as f32 * 0.35, 0.6, 4.0), Vec3::ZERO)
}

/// Renders `frames` frames with the resolve in the given state, moving
/// the camera every frame, and returns the motion-vector texture's bytes.
fn motion_bytes(rig: &mut common::lit_scene::Rig, taa: bool, frames: u32) -> Vec<u8> {
    assert!(
        rig.stage.set_compute_shading(true) > 0,
        "no view has the R64 stage — every assertion here would be vacuous",
    );
    assert!(
        rig.stage.set_temporal_aa(taa) > 0,
        "no view took the temporal setting — the assertion would be vacuous",
    );
    for step in 0..frames {
        rig.camera = camera_at(step);
        rig.stage.render_with_assets_primary(
            &rig.device,
            &rig.queue,
            &rig.resources,
            &rig.camera,
            1.0,
        );
    }
    let texture = rig
        .stage
        .motion_vector_texture()
        .expect("the R64 stage owns a motion-vector target");
    // `Rg16Float` is four bytes per texel, the same stride the shared
    // helper copies. Nothing here interprets the halves as numbers: the
    // question is whether anything was written at all.
    common::read_rgba8(&rig.device, &rig.queue, texture)
}

/// 🔴 Both halves in one test, in this order, and neither is optional.
///
/// Resolve **off** first: the texture has to still be zero, which is only
/// true if the pass never ran. Resolve **on** second: it has to stop
/// being zero, which is what proves the pass, the camera movement and the
/// readback all work — without it, a gate that skipped the pass
/// unconditionally would pass the first assertion and ship a renderer
/// with no motion vectors at all.
#[test]
fn the_pass_waits_for_a_reader() {
    let Some(mut rig) = common::lit_scene::rig(2, true) else {
        eprintln!("no adapter with the R64 features; skipping");
        return;
    };

    let idle = motion_bytes(&mut rig, false, 4);
    assert!(
        idle.iter().all(|&b| b == 0),
        "the motion-vector target was written with the resolve off: \
         {} of {} bytes are non-zero, so the pass is still running for \
         nobody",
        idle.iter().filter(|&&b| b != 0).count(),
        idle.len(),
    );

    let resolved = motion_bytes(&mut rig, true, 4);
    assert!(
        resolved.iter().any(|&b| b != 0),
        "the motion-vector target is still zero with the resolve ON — \
         the gate is skipping the pass unconditionally, or the camera \
         did not move",
    );
}
