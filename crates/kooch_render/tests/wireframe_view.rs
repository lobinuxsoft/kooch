//! GPU acceptance for the wireframe debug view (#452): the triangles a mesh actually rasterises,
//! read off the visibility buffer, so a polygon budget can be judged by looking.
//!
//! Run with:
//!   cargo test -p kooch_render --test wireframe_view

mod common;

use common::lit_scene::{Rig, SIZE, rig, rig_r32};
use glam::{Mat4, Vec3};
use kooch_core::Guid;
use kooch_ecs::commands::Commands;
use kooch_ecs::hierarchy::global_transform::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_render::meshlet::{MeshletDebugMode, build_default_meshlets};
use kooch_render::quality::{TemporalSettings, UpscaleTechnique};

/// A pixel the view drew a line on: the edge colour is the only green one in the plate.
fn is_line(pixel: &[u8]) -> bool {
    let (r, g, b) = (pixel[0] as i32, pixel[1] as i32, pixel[2] as i32);
    g > 128 && g > r + 20 && g > b + 20
}

/// Line pixels and pixels the view painted at all, with a sphere of `segments` in front of the
/// scene, or none.
fn lines(segments: Option<u32>) -> Option<(usize, usize)> {
    let mut r: Rig = rig(2, true)?;
    if let Some(segments) = segments {
        let guid = Guid::new_v4();
        let mesh = build_default_meshlets(&common::build_sphere_mesh(segments, segments))
            .expect("a sphere meshletises");
        r.stage.ensure_gpu_mesh(&r.device, guid, &mesh);
        let mut commands = Commands::new();
        commands
            .spawn(&mut r.resources)
            .insert(MeshRenderer {
                mesh: Some(guid),
                material: Some(r.material),
                visible: true,
                ..Default::default()
            })
            .insert(GlobalTransform {
                matrix: Mat4::from_translation(Vec3::new(0.0, 1.2, 4.0))
                    * Mat4::from_scale(Vec3::splat(1.5)),
            });
        commands.apply(&mut r.resources);
    }
    r.resources.insert(MeshletDebugMode::Wireframe);
    let pixels = common::lit_scene::render(&mut r, true);
    let painted = pixels
        .chunks_exact(4)
        .filter(|pixel| pixel[0] as u32 + pixel[1] as u32 + pixel[2] as u32 > 30)
        .count();
    Some((
        pixels.chunks_exact(4).filter(|p| is_line(p)).count(),
        painted,
    ))
}

/// 🔴 A wireframe is edges: lines where triangles meet, and a dark plate everywhere else. Filling
/// the covered area would say nothing about the polygon load.
#[test]
fn a_wireframe_paints_only_edges() {
    let Some((drawn, painted)) = lines(None) else {
        eprintln!("no capable adapter; skipping");
        return;
    };
    assert!(drawn > 0, "the view drew no line at all");
    assert!(
        drawn * 2 < painted,
        "{drawn} of {painted} painted pixels are line: that is a fill, not a wireframe",
    );
}

/// 🔴 What the view is for: more triangles on screen read as more line.
#[test]
fn a_denser_mesh_draws_more_lines() {
    let Some((coarse, _)) = lines(Some(6)) else {
        eprintln!("no capable adapter; skipping");
        return;
    };
    let (dense, _) = lines(Some(24)).unwrap();
    assert!(
        dense > coarse,
        "a 24-segment sphere drew {dense} line pixels, a 6-segment one {coarse}",
    );
}

/// The other visibility buffer draws it too, in its own shading shader rather than the overlay.
#[test]
fn the_r32_buffer_draws_lines() {
    let Some(mut r) = rig_r32(2, true) else {
        eprintln!("no capable adapter; skipping");
        return;
    };
    r.resources.insert(MeshletDebugMode::Wireframe);
    let pixels = common::lit_scene::render_any(&mut r);
    let drawn = pixels.chunks_exact(4).filter(|p| is_line(p)).count();
    assert!(drawn > 0, "the R32 path drew no line at all");
}

/// 🔴 A scaled render used to leave the view in a corner: the buffer is smaller than the image, so
/// the pass has to stretch what it reads instead of reading one to one.
#[test]
fn a_scaled_render_still_covers() {
    let Some(mut r) = rig(2, true) else {
        eprintln!("no capable adapter; skipping");
        return;
    };
    r.resources.insert(TemporalSettings {
        technique: UpscaleTechnique::Sgsr2,
        render_scale: 50,
        sharpening: 0,
    });
    r.resources.insert(MeshletDebugMode::Wireframe);
    // The first frame records the scale; the resize is what turns it into a smaller buffer.
    common::lit_scene::render(&mut r, true);
    r.stage.resize(&r.device, (SIZE, SIZE));
    let pixels = common::lit_scene::render(&mut r, true);
    // A pass that reads the buffer one to one runs off its end past the render width, and every
    // column beyond it repeats the last texel of its row. Two far-apart columns settle it.
    let column = |x: u32| -> Vec<[u8; 3]> {
        (0..SIZE)
            .map(|y| {
                let at = ((y * SIZE + x) * 4) as usize;
                [pixels[at], pixels[at + 1], pixels[at + 2]]
            })
            .collect()
    };
    assert_ne!(
        column(SIZE * 3 / 4),
        column(SIZE - 2),
        "every column past the render width is the same: the view did not stretch",
    );
}

/// 🔴 The overlay is the frame plus lines: what is not a line is exactly what was shaded.
#[test]
fn the_overlay_keeps_the_frame() {
    let Some(mut r) = rig(2, true) else {
        eprintln!("no capable adapter; skipping");
        return;
    };
    let shaded = common::lit_scene::render(&mut r, true);
    r.resources.insert(MeshletDebugMode::WireframeOver);
    let over = common::lit_scene::render(&mut r, true);
    let mut drawn = 0usize;
    let mut kept = 0usize;
    for (was, now) in shaded.chunks_exact(4).zip(over.chunks_exact(4)) {
        match is_line(now) {
            true => drawn += 1,
            false => kept += usize::from(was[..3] == now[..3]),
        }
    }
    assert!(drawn > 0, "the overlay drew no line");
    let untouched = shaded.len() / 4 - drawn;
    assert!(
        kept * 10 > untouched * 9,
        "{kept} of {untouched} pixels off the lines still show the frame",
    );
}

/// The plate is the view's own, not the scene's: nothing of the shading survives it.
#[test]
fn the_wireframe_replaces_the_shade() {
    let Some(mut r) = rig(2, true) else {
        eprintln!("no capable adapter; skipping");
        return;
    };
    let shaded = common::lit_scene::render(&mut r, true);
    r.resources.insert(MeshletDebugMode::Wireframe);
    let wired = common::lit_scene::render(&mut r, true);
    let centre = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
    assert_ne!(shaded[centre..centre + 3], wired[centre..centre + 3]);
}
