//! What the editor's panels and debug views show of a point shadow.

use super::*;

/// The owner's `project.rendersettings`, verbatim, which no picture in this file has used.
fn owners_rig(casting: bool) -> Option<Rig> {
    let mut rig = build_with(
        &[
            (Vec3::new(-5.151, 3.477, 0.0), casting),
            (Vec3::new(-2.651, 3.477, -1.813), casting),
        ],
        true,
        true,
        true,
    )?;
    rig.resources.insert(kooch_lighting::AmbientLight {
        intensity: 300.0,
        ..Default::default()
    });
    rig.resources
        .insert(kooch_render::quality::ShadingSettings {
            compute: true,
            rate: kooch_render::meshlet::ShadingRate::Half,
            anisotropy: 1,
        });
    rig.resources
        .insert(kooch_render::quality::TemporalSettings::new(
            kooch_render::quality::UpscaleTechnique::Taa,
            100,
            0,
            true,
        ));
    rig.resources.insert(ShadowSettings {
        cascade_texels: 512,
        max_distance: 30.0,
        enabled: true,
        point_shadows: 32,
        ..Default::default()
    });
    Some(rig)
}

/// A slow orbit under those settings, differenced against the same orbit with neither lamp casting.
/// Framing, occlusion, half-rate upsampling and the temporal resolve are identical between the two
/// runs; the only difference is the shadow.
#[test]
#[ignore = "prints a table; not an assertion"]
fn the_owners_settings_orbit() {
    const BALL: Vec3 = Vec3::new(0.0, 0.5, 0.0);
    let (Some(mut on), Some(mut off)) = (owners_rig(true), owners_rig(false)) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    eprintln!("\n  step   darkened%   worst pixel");
    for i in 0..24 {
        let ang = i as f32 * std::f32::consts::TAU / 24.0;
        let eye = Vec3::new(ang.cos() * 6.0, 3.5, ang.sin() * 6.0);
        let camera = ViewCamera::looking_at(eye, BALL);
        let shot = |rig: &mut Rig| {
            rig.stage.render_with_assets_primary(
                &rig.device,
                &rig.queue,
                &rig.resources,
                &camera,
                1.0,
            );
            read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture())
        };
        let (a, b) = (shot(&mut on), shot(&mut off));
        let (worst, mean) = compare(&a, &b);
        eprintln!("  {i:2}      {:.3}       {worst:.3}", mean * 100.0);
        if i % 8 == 0 {
            shoot(&mut on, &format!("owner_{i}.png"), eye);
        }
    }
}

/// The Game panel's own image, which is what the owner is looking at.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn the_game_panel_while_its_camera_turns() {
    const BALL: Vec3 = Vec3::new(0.0, 0.5, 0.0);
    let Some(mut rig) = owners_rig(true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let game = rig.stage.create_view(&rig.device, (SIZE, SIZE));
    let editor = ViewCamera::looking_at(Vec3::new(5.0, 3.5, 5.0), BALL);

    for i in 0..8 {
        // The View panel goes first every frame, like the editor's loop.
        rig.stage
            .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &editor, 1.0);
        let ang = i as f32 * std::f32::consts::TAU / 8.0;
        let camera = ViewCamera::looking_at(Vec3::new(ang.cos() * 6.0, 3.0, ang.sin() * 6.0), BALL);
        rig.stage
            .render_with_assets(game, &rig.device, &rig.queue, &rig.resources, &camera, 1.0);
        let tex = rig.stage.view_color_texture(game).expect("live view");
        let px = read_rgba8(&rig.device, &rig.queue, tex);
        let path = format!("/tmp/kooch_point_shadows/game_{i}.png");
        image::save_buffer(&path, &px, SIZE, SIZE, image::ColorType::Rgba8).unwrap();
        eprintln!("wrote {path}");
    }
}

/// "es como si la geometría se estuviera quedando sin mesh… será que se ocluden las meshlets?"
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn does_the_shadow_close_up_at_a_finer_lod() {
    const BALL: Vec3 = Vec3::new(0.0, 0.5, 0.0);
    for target in [1.0f32, 0.25, 0.01] {
        let Some(mut rig) = owners_rig(true) else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        rig.resources
            .insert(kooch_render::meshlet::MeshletLodSettings {
                target_error_pixels: target,
                ..Default::default()
            });
        // Straight down on the lamp's own axis, so the whole silhouette of the shadow is in frame
        // and a hole in it cannot hide behind the ball.
        let eye = Vec3::new(2.6, 7.0, 4.0);
        let camera = ViewCamera::looking_at(eye, BALL);
        rig.stage
            .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &camera, 1.0);
        let px = read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture());
        let name = format!("/tmp/kooch_point_shadows/lod_{target}.png");
        image::save_buffer(&name, &px, SIZE, SIZE, image::ColorType::Rgba8).unwrap();
        eprintln!("wrote {name}");
    }
}

/// The new view (#852), on the owner's scene.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn the_point_shadow_factor_view() {
    const BALL: Vec3 = Vec3::new(0.0, 0.5, 0.0);
    let Some(mut rig) = owners_rig(true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    rig.resources
        .insert(kooch_render::meshlet::MeshletDebugMode::PointShadowFactor);
    for (name, eye) in [
        ("factor_high.png", Vec3::new(2.6, 9.0, 4.0)),
        ("factor_low.png", Vec3::new(5.0, 3.0, 5.0)),
    ] {
        let camera = ViewCamera::looking_at(eye, BALL);
        rig.stage
            .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &camera, 1.0);
        let px = read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture());
        let path = format!("/tmp/kooch_point_shadows/{name}");
        image::save_buffer(&path, &px, SIZE, SIZE, image::ColorType::Rgba8).unwrap();
        eprintln!("wrote {path}");
    }
}

/// The cube view (#852), on the owner's scene.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn the_point_cube_view() {
    const BALL: Vec3 = Vec3::new(0.0, 0.5, 0.0);
    let Some(mut rig) = owners_rig(true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    rig.resources
        .insert(kooch_render::meshlet::MeshletDebugMode::PointCubeFaces);
    // Straight down, so the floor fills the frame: the view is a SURFACE shader, so it paints only
    // where there is geometry and the sky leaves its cells blank.
    let camera = ViewCamera::looking_at(Vec3::new(0.0, 9.0, 0.2), BALL);
    rig.stage
        .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &camera, 1.0);
    let px = read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture());
    image::save_buffer(
        "/tmp/kooch_point_shadows/cube_faces.png",
        &px,
        SIZE,
        SIZE,
        image::ColorType::Rgba8,
    )
    .unwrap();
    eprintln!("wrote cube_faces.png");
}

/// The cube's six faces read **out of the depth texture**, with the contrast stretched to whatever
/// is actually in them.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn the_cube_faces_raw() {
    const BALL: Vec3 = Vec3::new(0.0, 0.5, 0.0);
    let Some(mut rig) = owners_rig(true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let camera = ViewCamera::looking_at(Vec3::new(0.0, 9.0, 0.2), BALL);
    rig.stage
        .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &camera, 1.0);

    let cubes = rig.stage.shadow_cubes_texture().expect("cubes").clone();
    let size = cubes.size().width;
    const NAMES: [&str; 6] = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];
    for face in 0..6u32 {
        let depth = read_face_depth(&rig.device, &rig.queue, &cubes, face);
        let recorded: Vec<f32> = depth.iter().copied().filter(|d| *d > 0.0).collect();
        if recorded.is_empty() {
            eprintln!("face {} — empty", NAMES[face as usize]);
            continue;
        }
        let lo = recorded.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = recorded.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        // Reversed-Z: the stored value is `near / distance`, so BIGGER is CLOSER to the lamp.
        // Painted so that closer is darker, the way an occluder reads to a human.
        let px: Vec<u8> = depth
            .iter()
            .map(|d| {
                if *d <= 0.0 {
                    return 255;
                }
                let t = ((d - lo) / (hi - lo).max(1e-9)).clamp(0.0, 1.0);
                (255.0 * (1.0 - t)) as u8
            })
            .collect();
        let path = format!("/tmp/kooch_point_shadows/raw_face_{face}.png");
        image::save_buffer(&path, &px, size, size, image::ColorType::L8).unwrap();
        eprintln!(
            "face {} — {} texels recorded, near/dist in {lo:.5}..{hi:.5} \
             ({:.2}..{:.2} m)",
            NAMES[face as usize],
            recorded.len(),
            0.1 / hi,
            0.1 / lo,
        );
    }
}
