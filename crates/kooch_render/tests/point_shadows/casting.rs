//! A point light casts, turns off, falls away, and follows its light and caster.

use super::*;

#[test]
fn a_cube_over_a_floor_casts_a_shadow_on_it() {
    let Some(mut rig) = build_rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let light = SWEEP[0];
    add_point(&mut rig.resources, light, true);
    let pixels = render(&mut rig);

    let shadowed = luminance(&pixels, &rig.camera, shadow_centre(light));
    let lit = luminance(&pixels, &rig.camera, OPEN_FLOOR);
    assert!(
        shadowed < lit * 0.7,
        "floor under the cube ({shadowed}) is not meaningfully darker than open floor ({lit})",
    );
}

/// 🔴 The A/B that makes the suite mean something: the same pixel, two renders, one flag apart.
#[test]
fn clearing_cast_shadows_turns_the_shadow_off() {
    let Some(mut casting) = build_rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let light = SWEEP[0];
    add_point(&mut casting.resources, light, true);
    let with = render(&mut casting);

    let mut plain = build_rig().expect("second rig");
    add_point(&mut plain.resources, light, false);
    let without = render(&mut plain);

    let point = shadow_centre(light);
    let shadowed = luminance(&with, &casting.camera, point);
    let unshadowed = luminance(&without, &plain.camera, point);
    assert!(
        shadowed < unshadowed * 0.7,
        "the same floor pixel reads {shadowed} casting and {unshadowed} not casting — \
         the cube map is not reaching the shading pass",
    );
}

/// 🔴 The one a spot light cannot ask.
#[test]
fn the_shadow_falls_away_from_the_light() {
    // Every position is measured before anything is asserted. Failing on the first one hides
    // whether the fault is one face or a whole axis, and those have different causes.
    let mut report = Vec::new();
    for light in SWEEP {
        let Some(mut rig) = build_rig() else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        add_point(&mut rig.resources, light, true);
        let pixels = render(&mut rig);

        let away = shadow_centre(light);
        // The mirror image of that point through the cube: where the shadow would be if a sign were
        // wrong. It is lit floor, and it has to stay lit.
        let toward = Vec3::new(-away.x, away.y, -away.z);

        let shadowed = luminance(&pixels, &rig.camera, away);
        let opposite = luminance(&pixels, &rig.camera, toward);
        report.push((light, shadowed, opposite));
    }

    let failures: Vec<_> = report
        .iter()
        .filter(|(_, shadowed, opposite)| *shadowed >= opposite * 0.7)
        .collect();
    assert!(
        failures.is_empty(),
        "the shadow is not away from the lamp for {} of 4 positions.\n\
         away < toward*0.7 is the test; both bright means NO shadow, \
         toward dark means it is MIRRORED.\n{}",
        failures.len(),
        report
            .iter()
            .map(|(l, s, o)| format!("  lamp {l:?}: away={s:.3} toward={o:.3}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// Moves the one point light in the scene.
fn move_light(resources: &Resources, to: Vec3) {
    kooch_ecs::query::Query::<(&PointLight, &mut GlobalTransform)>::new(resources).for_each(
        |(_, transform)| {
            transform.matrix = Mat4::from_translation(to);
        },
    );
}

/// Moves the floating cube, leaving the floor where it is.
pub(super) fn move_cube(resources: &Resources, to: Vec3) {
    kooch_ecs::query::Query::<(&MeshRenderer, &mut GlobalTransform)>::new(resources).for_each(
        |(_, transform)| {
            if transform.matrix.w_axis.y > 0.5 {
                transform.matrix = Mat4::from_translation(to);
            }
        },
    );
}

/// 🔴 The cache tests, and they need TWO frames.
#[test]
fn moving_the_light_moves_its_shadow() {
    let Some(mut rig) = build_rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let first = SWEEP[0];
    let second = SWEEP[1];
    add_point(&mut rig.resources, first, true);
    let _ = render(&mut rig);

    move_light(&rig.resources, second);
    let pixels = render(&mut rig);

    let now = luminance(&pixels, &rig.camera, shadow_centre(second));
    let before = luminance(&pixels, &rig.camera, shadow_centre(first));
    assert!(
        now < before * 0.7,
        "after moving the lamp the shadow reads {now} at its new place and {before} at the \
         old one — the cube was reused for a light that moved",
    );
}

#[test]
fn moving_the_caster_moves_its_shadow() {
    let Some(mut rig) = build_rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let light = SWEEP[0];
    add_point(&mut rig.resources, light, true);
    let _ = render(&mut rig);

    // The lamp has not moved, so only the scene hash can invalidate the
    // cube. Without it the shadow stays under where the cube used to be.
    let moved = CUBE_CENTRE + Vec3::new(-3.0, 0.0, 0.0);
    move_cube(&rig.resources, moved);
    let pixels = render(&mut rig);

    let old_spot = luminance(&pixels, &rig.camera, shadow_centre(light));
    let lit = luminance(&pixels, &rig.camera, OPEN_FLOOR);
    assert!(
        old_spot > lit * 0.7,
        "the floor where the cube used to stand still reads {old_spot} against {lit} lit — \
         the cube map was not redrawn after the caster moved",
    );
}

/// 🔴🔴 The same assertion, in the shading path the GAME actually uses.
#[test]
fn the_compute_path_casts_the_same_shadow() {
    let Some(mut rig) = build_rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    rig.resources
        .insert(kooch_render::quality::ShadingSettings {
            compute: true,
            ..Default::default()
        });
    let light = SWEEP[0];
    add_point(&mut rig.resources, light, true);
    let pixels = render(&mut rig);

    let shadowed = luminance(&pixels, &rig.camera, shadow_centre(light));
    let lit = luminance(&pixels, &rig.camera, OPEN_FLOOR);
    assert!(
        shadowed < lit * 0.7,
        "with compute shading on, the floor under the cube ({shadowed}) is not \
         meaningfully darker than open floor ({lit}) — the cube map reaches the \
         fragment path and not this one",
    );
}

/// And at the shading rate the game ships with.
#[test]
fn half_rate_shading_keeps_the_shadow() {
    let Some(mut rig) = build_rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    rig.resources
        .insert(kooch_render::quality::ShadingSettings {
            compute: true,
            rate: kooch_render::meshlet::ShadingRate::Half,
            ..Default::default()
        });
    let light = SWEEP[0];
    add_point(&mut rig.resources, light, true);
    let pixels = render(&mut rig);

    let shadowed = luminance(&pixels, &rig.camera, shadow_centre(light));
    let lit = luminance(&pixels, &rig.camera, OPEN_FLOOR);
    assert!(
        shadowed < lit * 0.7,
        "at half shading rate the floor under the cube ({shadowed}) is not \
         meaningfully darker than open floor ({lit})",
    );
}

/// 🔴 The scene's own scale, not the suite's.
#[test]
fn a_short_range_lamp_still_casts() {
    let Some(mut rig) = build_rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    rig.resources
        .insert(kooch_render::quality::ShadingSettings {
            compute: true,
            rate: kooch_render::meshlet::ShadingRate::Half,
            ..Default::default()
        });
    // Close enough that a 4 m sphere still covers the cube and the floor beside it, which is what
    // the authored scene does.
    let light = Vec3::new(1.2, 1.2, 0.0);
    let mut commands = kooch_ecs::commands::Commands::new();
    commands
        .spawn(&mut rig.resources)
        .insert(kooch_ecs::point_light::PointLight {
            active: true,
            color: Vec3::ONE,
            intensity: 60_000.0,
            range: 4.0,
            radius: 0.0,
            cast_shadows: true,
            contact_shadows: false,
                ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(light),
        });
    commands.apply(&mut rig.resources);
    let pixels = render(&mut rig);

    // Opposite the lamp, just past the cube's edge, and a lit point at the same distance from the
    // lamp so the two differ by the shadow and not by falloff.
    let shadowed = luminance(&pixels, &rig.camera, Vec3::new(-0.9, 0.0, 0.0));
    let lit = luminance(&pixels, &rig.camera, Vec3::new(0.0, 0.0, 1.3));
    // 🔴 A weaker margin than the rest of the suite, and it is the finding rather than a concession.
    assert!(
        shadowed < lit * 0.8,
        "a 4 m lamp leaves the floor under the cube at {shadowed} against {lit} lit",
    );
}

/// 🔴🔴 A cube map cannot depend on where the camera is.
#[test]
fn the_shadow_does_not_move_with_the_camera() {
    let Some(mut high) = build_rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let light = SWEEP[0];
    let probe = shadow_centre(light);
    add_point(&mut high.resources, light, true);
    let from_high = render(&mut high);
    let high_value = luminance(&from_high, &high.camera, probe);

    let mut low = build_rig().expect("second rig");
    add_point(&mut low.resources, light, true);
    // Same scene, same light, a different place to stand.
    low.camera = ViewCamera::looking_at(Vec3::new(-9.0, 7.0, 9.0), Vec3::ZERO);
    let from_low = render(&mut low);
    let low_value = luminance(&from_low, &low.camera, probe);

    let spread = (high_value - low_value).abs() / high_value.max(1e-4);
    assert!(
        spread < 0.15,
        "the same shadowed floor point reads {high_value:.4} from above and \
         {low_value:.4} from the side — a cube map has no camera in it",
    );
}
