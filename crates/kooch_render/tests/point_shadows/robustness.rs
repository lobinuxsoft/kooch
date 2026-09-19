//! What must not break it: falloff corners, budgets, offscreen casters, overhead lamps, a second view or lamp.

use super::*;

/// 🔴 A point light is a point. Its falloff on a flat floor is a set of circles, and anything with a
/// corner in it comes from the cube, not from the light.
#[test]
fn the_falloff_has_no_corners() {
    let Some(mut rig) = build_rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    // Straight overhead, so the four probes below are symmetric about it
    // and no cube face is favoured by geometry.
    let light = Vec3::new(0.0, 6.0, 0.0);
    add_point(&mut rig.resources, light, true);
    let pixels = render(&mut rig);

    // 🔴 Radius 7, and the number is the whole test. The downward cube face spans 90°, so from 6 m
    // up it covers the floor out to 6 m along each axis and 8.5 m along each diagonal.
    let radius = 7.0_f32;
    let diagonal = radius / 2.0_f32.sqrt();
    let axes: Vec<f32> = [
        Vec3::new(radius, 0.0, 0.0),
        Vec3::new(-radius, 0.0, 0.0),
        Vec3::new(0.0, 0.0, radius),
        Vec3::new(0.0, 0.0, -radius),
    ]
    .iter()
    .map(|p| luminance(&pixels, &rig.camera, *p))
    .collect();
    let diagonals: Vec<f32> = [
        Vec3::new(diagonal, 0.0, diagonal),
        Vec3::new(-diagonal, 0.0, diagonal),
        Vec3::new(diagonal, 0.0, -diagonal),
        Vec3::new(-diagonal, 0.0, -diagonal),
    ]
    .iter()
    .map(|p| luminance(&pixels, &rig.camera, *p))
    .collect();

    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    let on_axis = mean(&axes);
    let on_diagonal = mean(&diagonals);
    let spread = (on_axis - on_diagonal).abs() / on_axis.max(1e-4);
    assert!(
        spread < 0.10,
        "floor at radius {radius} reads {on_axis:.4} on the axes and \
         {on_diagonal:.4} on the diagonals — that difference is a square, \
         and a point light does not have corners",
    );
}

/// One lamp, but the budget the owner had set when the artifacts were reported.
#[test]
fn a_large_budget_does_not_lose_the_shadow() {
    let Some(mut rig) = build_rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    rig.resources.insert(ShadowSettings {
        cascade_texels: 512,
        max_distance: 30.0,
        enabled: true,
        point_shadows: 32,
        ..Default::default()
    });
    let light = SWEEP[0];
    add_point(&mut rig.resources, light, true);
    let pixels = render(&mut rig);

    let shadowed = luminance(&pixels, &rig.camera, shadow_centre(light));
    let lit = luminance(&pixels, &rig.camera, OPEN_FLOOR);
    assert!(
        shadowed < lit * 0.7,
        "with a budget of 32 the floor under the cube reads {shadowed} against \
         {lit} lit",
    );
}

/// 🔴🔴 A caster the camera cannot see still casts.
#[test]
fn an_offscreen_caster_still_casts() {
    let Some(mut rig) = build_rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    add_point(&mut rig.resources, Vec3::new(8.0, 3.0, 0.0), true);
    // Low enough that the frame is about 3.4 m wide: the cube at the
    // origin is four metres away and out of it.
    rig.camera = ViewCamera::looking_at(Vec3::new(-4.0, 3.0, 0.01), Vec3::new(-4.0, 0.0, 0.0));
    let pixels = render(&mut rig);

    // Inside the shadow the cube throws, and beside it in z where the
    // same lamp reaches the floor unobstructed.
    let shadowed = luminance(&pixels, &rig.camera, Vec3::new(-3.6, 0.0, 0.0));
    let lit = luminance(&pixels, &rig.camera, Vec3::new(-3.6, 0.0, 1.4));
    assert!(
        shadowed < lit * 0.7,
        "with the caster off screen the floor reads {shadowed} in its shadow and \
         {lit} beside it — the shadow pass is being fed the camera's visible set",
    );
}

/// 🔴🔴🔴 A lamp directly above its occluder.
#[test]
fn a_lamp_straight_overhead_casts_down() {
    let Some(mut rig) = build_rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mesh = rig.mesh;
    let material = rig.material;
    let mut commands = Commands::new();
    commands
        .spawn(&mut rig.resources)
        .insert(MeshRenderer {
            mesh: Some(mesh),
            material: Some(material),
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(Vec3::new(3.0, 3.0, 0.0)),
        });
    commands.apply(&mut rig.resources);

    // Straight above it. Nothing lateral about this at all.
    add_point(&mut rig.resources, Vec3::new(3.0, 6.0, 0.0), true);
    let pixels = render(&mut rig);

    let shadowed = luminance(&pixels, &rig.camera, Vec3::new(3.0, 0.0, 0.0));
    let lit = luminance(&pixels, &rig.camera, Vec3::new(3.0, 0.0, 3.0));
    assert!(
        shadowed < lit * 0.7,
        "a lamp directly overhead leaves the floor under its occluder at \
         {shadowed} against {lit} beside it — the -Y cube face is not answering",
    );
}

/// 🔴🔴 The floor must not shadow itself, and this suite never asked.
#[test]
fn an_empty_floor_is_not_shadowed_by_itself() {
    // Well inside the lamp's reach and spread across the boundary where a cube face hands over to
    // its neighbour — under a lamp `h` up that is a square of side 2h, and the seam ran along it.
    const PROBES: [Vec3; 5] = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(3.0, 0.0, 0.0),
        Vec3::new(6.5, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 6.5),
        Vec3::new(5.0, 0.0, 5.0),
    ];
    // A lamp 6 m up hands its -Y face over at +/- 6 m, so the probes at
    // 6.5 sit on the far side of that seam and the ones at 3 do not.
    const LAMP: Vec3 = Vec3::new(0.0, 6.0, 0.0);
    // Out of frame and out of the lamp's range, so the only geometry left is the floor itself.
    const AWAY: Vec3 = Vec3::new(100.0, 0.5, 0.0);

    let mut readings = Vec::new();
    for casting in [false, true] {
        let Some(mut rig) = build_rig() else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        move_cube(&rig.resources, AWAY);
        add_point(&mut rig.resources, LAMP, casting);
        let pixels = render(&mut rig);
        readings.push(
            PROBES
                .iter()
                .map(|p| luminance(&pixels, &rig.camera, *p))
                .collect::<Vec<_>>(),
        );
    }

    for (i, probe) in PROBES.iter().enumerate() {
        let open = readings[0][i];
        let cast = readings[1][i];
        assert!(
            cast > open - 0.02,
            "at {probe:?} an empty floor reads {cast} with the lamp casting and {open} with \
             it not — nothing in this scene can cast a shadow, so the cube map is darkening \
             the floor with itself. Point lights get their own shadow bias for this reason; \
             the sun's pair leaves them at a quarter of the depth push they need",
        );
    }
}

/// 🔴🔴 Two views on one stage — the editor's arrangement, which this suite never had.
#[test]
fn a_second_view_does_not_take_the_shadow_away() {
    let Some(mut rig) = build_rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let light = SWEEP[0];
    add_point(&mut rig.resources, light, true);

    let pixels = render(&mut rig);
    let alone = luminance(&pixels, &rig.camera, shadow_centre(light));
    let lit = luminance(&pixels, &rig.camera, OPEN_FLOOR);
    assert!(
        alone < lit * 0.7,
        "the shadow is not there before the second view even exists ({alone} against {lit})",
    );

    // A second view, looking somewhere else entirely.
    let second = rig.stage.create_view(&rig.device, (SIZE, SIZE));
    let elsewhere = ViewCamera::looking_at(Vec3::new(60.0, 2.0, 60.0), Vec3::new(70.0, 0.0, 70.0));
    rig.stage.render_with_assets(
        second,
        &rig.device,
        &rig.queue,
        &rig.resources,
        &elsewhere,
        1.0,
    );

    let pixels = render(&mut rig);
    let after = luminance(&pixels, &rig.camera, shadow_centre(light));
    assert!(
        after < lit * 0.7,
        "after a second view rendered from a camera that cannot see the lamp, this view's \
         shadow reads {after} against {lit} lit — it read {alone} a frame ago and nothing \
         moved. A cube map is drawn from the light; which lamps get one must not depend on \
         who is looking",
    );
}

/// Two casting lamps, and **both** shadows have to be there (#853).
#[test]
fn a_second_lamp_does_not_erase_the_first_shadow() {
    let (Some(mut alone), Some(mut together)) = (build_rig(), build_rig()) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    // Opposite sides, so neither shadow reaches the other's spot.
    let (first, second) = (SWEEP[0], SWEEP[1]);
    // The two rigs differ by ONE flag: whether the second lamp casts. It lights the scene
    // identically either way, and at the FIRST lamp's shadow centre it is not blocked by anything —
    // so its casting cannot legally change that pixel by any amount.
    add_point(&mut alone.resources, first, true);
    add_point(&mut alone.resources, second, false);
    add_point(&mut together.resources, first, true);
    add_point(&mut together.resources, second, true);

    let probe = shadow_centre(first);
    let one = luminance(&render(&mut alone), &alone.camera, probe);
    let two = luminance(&render(&mut together), &together.camera, probe);
    assert!(
        (one - two).abs() < 0.02,
        "the first lamp's shadow reads {one} with the second lamp not casting and {two} \
         with it casting; the second lamp took the first one's cube",
    );

    // And the shadow has to exist, or the two agree at nothing.
    let Some(mut neither) = build_rig() else {
        return;
    };
    add_point(&mut neither.resources, first, false);
    add_point(&mut neither.resources, second, false);
    let open = luminance(&render(&mut neither), &neither.camera, probe);
    // Only a few percent, and that is correct rather than weak: shadows multiply per light and
    // lights add, so blocking one of two lamps can never take more than that lamp's share of the
    // pixel.
    assert!(
        one < open * 0.95,
        "the first lamp casts no shadow at all: {one} against {open} with it not casting",
    );
}
