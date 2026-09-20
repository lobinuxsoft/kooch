//! GPU acceptance for the camera's culling mask (#1219): a camera draws the layers it names, and
//! the shadows do not care what it named — they belong to the light.
//!
//! Run with:
//!   cargo test -p kooch_render --test camera_layers

mod common;

use common::lit_scene::{Rig, rig_with_caster};
use glam::{Mat4, Vec3};
use kooch_ecs::commands::Commands;
use kooch_ecs::hierarchy::global_transform::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;

/// The scene's blue light, and the camera's own pixels: a caster on `layers` under the light, or
/// none, seen by a camera that keeps `mask`.
fn scene(layers: Option<u32>, mask: u32) -> Option<(u64, u64)> {
    let mut r: Rig = rig_with_caster(2)?;
    r.camera.culling_mask = mask;
    if let Some(layers) = layers {
        let mut commands = Commands::new();
        commands
            .spawn(&mut r.resources)
            .insert(MeshRenderer {
                mesh: Some(r.mesh),
                material: Some(r.material),
                visible: true,
                layers,
                ..Default::default()
            })
            .insert(GlobalTransform {
                matrix: Mat4::from_translation(Vec3::new(0.0, 1.2, 1.0))
                    * Mat4::from_scale(Vec3::new(2.5, 0.02, 2.5)),
            });
        commands.apply(&mut r.resources);
    }
    // Settled, as the shadow tests do: the pages fill over a few frames.
    let mut pixels = Vec::new();
    for _ in 0..4 {
        pixels = common::lit_scene::render(&mut r, true);
    }
    let blue = pixels.chunks_exact(4).map(|p| p[2] as u64).sum();
    let lit = pixels
        .chunks_exact(4)
        .map(|p| p[0] as u64 + p[1] as u64 + p[2] as u64)
        .sum();
    Some((blue, lit))
}

/// 🔴 The acceptance: what the camera excludes leaves the image, and its shadow stays — the light
/// was never told about the camera's mask.
#[test]
fn an_excluded_layer_still_casts() {
    let Some((open, _)) = scene(None, u32::MAX) else {
        eprintln!("no R64-capable adapter; skipping");
        return;
    };
    // The pane is on layer 1, the rest of the scene on layer 0.
    let (shaded, _) = scene(Some(0b10), u32::MAX).unwrap();
    let (hidden, _) = scene(Some(0b10), 0b01).unwrap();

    assert!(
        shaded < open,
        "the pane cast no shadow at all: {shaded} against {open}",
    );
    let taken = |after: u64| open as i64 - after as i64;
    let (with, without) = (taken(shaded), taken(hidden));
    assert!(
        without * 4 > with * 3,
        "the hidden pane stopped casting: it took {without} of the light it took while drawn, \
         {with}",
    );
}

/// A camera that keeps no layer draws nothing, which is the mask meaning what it says rather than
/// being ignored at zero.
#[test]
fn an_empty_mask_draws_nothing() {
    let Some((_, lit)) = scene(None, u32::MAX) else {
        eprintln!("no R64-capable adapter; skipping");
        return;
    };
    let (_, dark) = scene(None, 0).unwrap();
    assert!(
        dark * 10 < lit,
        "a camera keeping no layer still drew {dark} against {lit}",
    );
}
