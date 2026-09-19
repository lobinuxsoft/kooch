//! The shadow against cameras and views: still lamps, turning cameras, two views, orbits.

use super::*;

/// Observation 1 — "si muevo la luz se actualizan las sombras, si no la muevo mueren".
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn a_still_lamp_over_a_moving_caster() {
    for compute in [false, true] {
        let Some(mut rig) = build(&[(OVERHEAD, true)], true, compute) else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        let tag = if compute { "compute" } else { "fragment" };
        let eye = Vec3::new(6.0, 5.0, 6.0);
        for (n, x) in [0.0f32, 1.2, 2.4, 3.6].iter().enumerate() {
            move_occluder(&rig.resources, Vec3::new(*x, 0.5, 0.0));
            shoot(&mut rig, &format!("move_{tag}_{n}.png"), eye);
        }
    }
}

/// Observation 3 — "en cada cámara se ve diferente".
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn two_cameras_must_agree() {
    let Some(mut rig) = build(&[(OVERHEAD, true)], true, true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    // Warm the cache the way a running editor does.
    shoot(&mut rig, "agree_0_warm.png", Vec3::new(6.0, 5.0, 6.0));
    shoot(&mut rig, "agree_1_far.png", Vec3::new(14.0, 11.0, 14.0));
    shoot(&mut rig, "agree_2_near.png", Vec3::new(3.0, 2.2, 3.0));
    // Back to the first camera. Same eye as `agree_0_warm`, so the two must be the same picture —
    // anything else is the cube remembering who looked at it last.
    shoot(&mut rig, "agree_3_back.png", Vec3::new(6.0, 5.0, 6.0));
}

/// The remaining report — "dependiendo de en qué posición esté la cámara la point light genera
/// sombras cortadas o no las genera".
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn the_shadow_must_not_depend_on_the_camera() {
    // Straight off the inspector.
    const LAMP: Vec3 = Vec3::new(-2.651, 3.477, -1.813);
    let Some(mut rig) = build(&[(LAMP, true)], true, true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    // Around the ball at a constant height and radius, so the only
    // thing that changes between frames is where the lens is.
    for i in 0..9 {
        let a = i as f32 * std::f32::consts::TAU / 8.0;
        let eye = Vec3::new(a.cos() * 6.0, 4.0, a.sin() * 6.0);
        shoot(&mut rig, &format!("orbit_{i}.png"), eye);
    }
}

/// The editor's arrangement, which no test has ever had: **two views on one stage**.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn two_views_on_one_stage() {
    const LAMP: Vec3 = Vec3::new(-2.651, 3.477, -1.813);
    let Some(mut rig) = build(&[(LAMP, true)], true, true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let second = rig.stage.create_view(&rig.device, (SIZE, SIZE));

    // The reference: the View camera alone, cold, drawn twice so the
    // second one is a cache hit with nothing else interleaved.
    let view_eye = Vec3::new(6.0, 4.0, 0.0);
    shoot(&mut rig, "views_0_alone.png", view_eye);
    shoot(&mut rig, "views_1_alone_again.png", view_eye);

    // Now alternate, the way the editor does every frame: Game looking away from the lamp, then
    // View from the same eye as above. If the last picture differs from the first two, the shadow
    // depends on who else looked this frame.
    for round in 0..3 {
        let game = ViewCamera::looking_at(Vec3::new(30.0, 2.0, 30.0), Vec3::new(40.0, 0.0, 40.0));
        rig.stage
            .render_with_assets(second, &rig.device, &rig.queue, &rig.resources, &game, 1.0);
        shoot(
            &mut rig,
            &format!("views_2_after_game_{round}.png"),
            view_eye,
        );
    }
}

/// Which half of the two-view frame does it: the camera-frustum cull, or the shared cube cache?
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn two_views_where_game_also_sees_the_lamp() {
    const LAMP: Vec3 = Vec3::new(-2.651, 3.477, -1.813);
    let Some(mut rig) = build(&[(LAMP, true)], true, true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let second = rig.stage.create_view(&rig.device, (SIZE, SIZE));
    let view_eye = Vec3::new(6.0, 4.0, 0.0);
    shoot(&mut rig, "sees_0_alone.png", view_eye);

    for round in 0..3 {
        // Looking at the origin, so the lamp's sphere is well inside.
        let game = ViewCamera::looking_at(Vec3::new(0.0, 5.0, 8.0), Vec3::new(0.0, 0.5, 0.0));
        rig.stage
            .render_with_assets(second, &rig.device, &rig.queue, &rig.resources, &game, 1.0);
        shoot(
            &mut rig,
            &format!("sees_1_after_game_{round}.png"),
            view_eye,
        );
    }
}

/// Two lamps, two views, and the Game camera turning.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn two_lamps_while_the_game_camera_turns() {
    // Both lamps, off the inspector.
    const A: Vec3 = Vec3::new(-5.151, 3.477, 0.0);
    const B: Vec3 = Vec3::new(-2.651, 3.477, -1.813);
    let Some(mut rig) = build(&[(A, true), (B, true)], true, true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let second = rig.stage.create_view(&rig.device, (SIZE, SIZE));
    let view_eye = Vec3::new(5.0, 3.5, 5.0);
    shoot(&mut rig, "turn_0_alone.png", view_eye);

    for i in 0..6 {
        let a = i as f32 * std::f32::consts::TAU / 6.0;
        let game = ViewCamera::looking_at(
            Vec3::new(a.cos() * 7.0, 3.0, a.sin() * 7.0),
            Vec3::new(0.0, 0.5, 0.0),
        );
        rig.stage
            .render_with_assets(second, &rig.device, &rig.queue, &rig.resources, &game, 1.0);
        shoot(&mut rig, &format!("turn_1_game_at_{i}.png"), view_eye);
    }
}

/// The same turning Game camera, with `contact_shadows` ON — which is what the owner's lamps carry
/// now, and what every picture in this file so far was taken without.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn two_lamps_with_contact_shadows_on() {
    const A: Vec3 = Vec3::new(-5.151, 3.477, 0.0);
    const B: Vec3 = Vec3::new(-2.651, 3.477, -1.813);
    let Some(mut rig) = build_with(&[(A, true), (B, true)], true, true, true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let second = rig.stage.create_view(&rig.device, (SIZE, SIZE));
    let view_eye = Vec3::new(5.0, 3.5, 5.0);
    shoot(&mut rig, "contact_0_alone.png", view_eye);

    for i in 0..6 {
        let a = i as f32 * std::f32::consts::TAU / 6.0;
        let game = ViewCamera::looking_at(
            Vec3::new(a.cos() * 7.0, 3.0, a.sin() * 7.0),
            Vec3::new(0.0, 0.5, 0.0),
        );
        rig.stage
            .render_with_assets(second, &rig.device, &rig.queue, &rig.resources, &game, 1.0);
        shoot(&mut rig, &format!("contact_1_game_at_{i}.png"), view_eye);
    }
}

/// The owner's configuration exactly: two lamps, both casting, both with `contact_shadows` on, and
/// the camera that MOVES is the one being looked at.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn orbiting_with_two_lamps_and_contact_on() {
    const A: Vec3 = Vec3::new(-5.151, 3.477, 0.0);
    const B: Vec3 = Vec3::new(-2.651, 3.477, -1.813);
    for contact in [false, true] {
        let Some(mut rig) = build_with(&[(A, true), (B, true)], true, true, contact) else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        let tag = if contact { "on" } else { "off" };
        for i in 0..8 {
            let a = i as f32 * std::f32::consts::TAU / 8.0;
            let eye = Vec3::new(a.cos() * 6.0, 3.5, a.sin() * 6.0);
            shoot(&mut rig, &format!("selforbit_{tag}_{i}.png"), eye);
        }
    }
}

/// The same orbit, measured instead of looked at.
#[test]
#[ignore = "prints a table; not an assertion"]
fn measure_the_orbit() {
    const A: Vec3 = Vec3::new(-5.151, 3.477, 0.0);
    const B: Vec3 = Vec3::new(-2.651, 3.477, -1.813);
    const BALL: Vec3 = Vec3::new(0.0, 0.5, 0.0);
    // Far from the ball and inside both lamps' reach: what "lit" means.
    const OPEN: Vec3 = Vec3::new(-3.0, 0.0, 2.5);

    let centre = |lamp: Vec3| {
        let d = (BALL - lamp).normalize();
        BALL + d * (BALL.y / d.y.abs())
    };

    for contact in [false, true] {
        let Some(mut rig) = build_with(&[(A, true), (B, true)], true, true, contact) else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        eprintln!("\ncontact_shadows = {contact}");
        eprintln!("  cam   shadow(A)  shadow(B)   open    A/open  B/open");
        for i in 0..8 {
            let ang = i as f32 * std::f32::consts::TAU / 8.0;
            let eye = Vec3::new(ang.cos() * 6.0, 3.5, ang.sin() * 6.0);
            let camera = ViewCamera::looking_at(eye, BALL);
            rig.stage.render_with_assets_primary(
                &rig.device,
                &rig.queue,
                &rig.resources,
                &camera,
                1.0,
            );
            let px = read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture());
            let probe = |w: Vec3| {
                let clip = camera.view_proj(1.0) * w.extend(1.0);
                let ndc = clip.truncate() / clip.w;
                let x = ((ndc.x * 0.5 + 0.5) * SIZE as f32).clamp(0.0, (SIZE - 1) as f32) as u32;
                let y = ((0.5 - ndc.y * 0.5) * SIZE as f32).clamp(0.0, (SIZE - 1) as f32) as u32;
                common::luminance_at(&px, SIZE, x, y, 2)
            };
            let (sa, sb, open) = (probe(centre(A)), probe(centre(B)), probe(OPEN));
            eprintln!(
                "  {i}     {sa:.4}     {sb:.4}    {open:.4}   {:.2}    {:.2}",
                sa / open.max(1e-4),
                sb / open.max(1e-4),
            );
        }
    }
}

/// The orbit measured so that framing cancels exactly.
#[test]
#[ignore = "prints a table; not an assertion"]
fn the_orbit_differenced() {
    const A: Vec3 = Vec3::new(-5.151, 3.477, 0.0);
    const B: Vec3 = Vec3::new(-2.651, 3.477, -1.813);
    const BALL: Vec3 = Vec3::new(0.0, 0.5, 0.0);

    for contact in [false, true] {
        let Some(mut on) = build_with(&[(A, true), (B, true)], true, true, contact) else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        let Some(mut off) = build_with(&[(A, false), (B, false)], true, true, contact) else {
            return;
        };
        eprintln!("\ncontact_shadows = {contact}");
        eprintln!("  cam    darkened%   worst pixel");
        for i in 0..8 {
            let ang = i as f32 * std::f32::consts::TAU / 8.0;
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
            eprintln!("  {i}      {:.3}       {worst:.3}", mean * 100.0);
        }
    }
}
