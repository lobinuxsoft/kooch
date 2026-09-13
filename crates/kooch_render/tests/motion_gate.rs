//! The motion-vector pass runs only when something reads it (#481).

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
