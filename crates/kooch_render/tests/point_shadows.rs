//! A point light casts a shadow, on a real GPU (#778).

mod common;

use common::{build_cube_mesh, luminance_at, read_rgba8, try_acquire_device};
use glam::{Mat4, Vec3};
use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::registry::ComponentRegistry;
use kooch_ecs::hierarchy::global_transform::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_ecs::point_light::PointLight;
use kooch_ecs::query::AccessTracker;
use kooch_render::ViewCamera;
use kooch_render::material::{Material, MaterialPipeline};
use kooch_render::meshlet::{MeshletRenderStage, MeshletRenderStageConfig, build_default_meshlets};
use kooch_render::shadow::ShadowSettings;

const SIZE: u32 = 256;

/// Centre of the cube. Its underside is at y = 1, so a shadow of it is
/// separate from the object rather than continuous with it.
const CUBE_CENTRE: Vec3 = Vec3::new(0.0, 1.5, 0.0);

/// Floor well clear of everything. Nothing can shadow it, so it is what "lit" means in this scene.
const OPEN_FLOOR: Vec3 = Vec3::new(-6.0, 0.0, 5.0);

struct Rig {
    device: wgpu::Device,
    queue: wgpu::Queue,
    resources: Resources,
    stage: MeshletRenderStage,
    camera: ViewCamera,
    /// So a test can add geometry of its own — the suite's own cube stands ON the floor, and a
    /// light directly above it hides its shadow under it.
    mesh: Guid,
    material: Guid,
}

fn build_rig() -> Option<Rig> {
    let (device, queue) = try_acquire_device()?;
    let meshlet_mesh = build_default_meshlets(&build_cube_mesh()).expect("build meshlets");

    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());
    resources.insert(ShadowSettings {
        cascade_texels: 1024,
        max_distance: 60.0,
        enabled: true,
        ..Default::default()
    });
    // Present but low, the reason `csm_shadows` states: with no ambient
    // everything shadowed is black, and "darker than lit" then passes
    // just as readily for a shadow that swallowed the whole floor.
    resources.insert(kooch_lighting::AmbientLight {
        intensity: 200.0,
        ..Default::default()
    });

    let mut materials = MaterialPipeline::with_capacity(&device, &queue, 4);
    let material_guid = Guid::new_v4();
    materials.register(
        &queue,
        material_guid,
        &Material::new([0.8, 0.8, 0.8, 1.0], 0.0, 0.9, 0.0),
    );
    resources.insert(materials);

    let mut stage = MeshletRenderStage::new(
        &device,
        MeshletRenderStageConfig {
            size: (SIZE, SIZE),
            instance_capacity: 8,
            meshlet_capacity: 1024,
            ..Default::default()
        },
    );
    let mesh_guid = Guid::new_v4();
    stage.ensure_gpu_mesh(&device, mesh_guid, &meshlet_mesh);

    let mut commands = Commands::new();
    let mut spawn_cube = |matrix: Mat4| {
        commands
            .spawn(&mut resources)
            .insert(MeshRenderer {
                mesh: Some(mesh_guid),
                material: Some(material_guid),
                visible: true,
                ..Default::default()
            })
            .insert(GlobalTransform { matrix });
    };
    spawn_cube(
        Mat4::from_translation(Vec3::new(0.0, -0.25, 0.0))
            * Mat4::from_scale(Vec3::new(20.0, 0.5, 20.0)),
    );
    spawn_cube(Mat4::from_translation(CUBE_CENTRE));
    commands.apply(&mut resources);

    Some(Rig {
        device,
        queue,
        resources,
        stage,
        mesh: mesh_guid,
        material: material_guid,
        // 🔴 Nearly overhead, and that is a requirement rather than a framing choice.
        camera: ViewCamera::looking_at(Vec3::new(0.0, 15.0, 4.0), Vec3::new(0.0, 0.0, 0.0)),
    })
}

/// Adds a point light at `position`, casting or not.
fn add_point(resources: &mut Resources, position: Vec3, cast_shadows: bool) {
    let mut commands = Commands::new();
    commands
        .spawn(resources)
        .insert(PointLight {
            active: true,
            color: Vec3::ONE,
            intensity: 4_000_000.0,
            range: 40.0,
            // A point source: a radius widens the specular highlight and
            // this suite measures darkness, not highlights.
            radius: 0.0,
            cast_shadows,
            // The cube map alone. A contact shadow would darken the same floor for a different
            // reason and the suite would stop being about the cube map.
            contact_shadows: false,
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(position),
        });
    commands.apply(resources);
}

fn render(rig: &mut Rig) -> Vec<u8> {
    rig.stage
        .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &rig.camera, 1.0);
    read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture())
}

fn project(camera: &ViewCamera, world: Vec3) -> (u32, u32) {
    let clip = camera.view_proj(1.0) * world.extend(1.0);
    let ndc = clip.truncate() / clip.w;
    let x = ((ndc.x * 0.5 + 0.5) * SIZE as f32).clamp(0.0, (SIZE - 1) as f32);
    let y = ((0.5 - ndc.y * 0.5) * SIZE as f32).clamp(0.0, (SIZE - 1) as f32);
    (x as u32, y as u32)
}

/// Where the cube's shadow lands for a light at `light`: the cube's centre traced away from the
/// light down to the floor.
fn shadow_centre(light: Vec3) -> Vec3 {
    let direction = (CUBE_CENTRE - light).normalize();
    CUBE_CENTRE + direction * (CUBE_CENTRE.y / direction.y).abs()
}

fn luminance(pixels: &[u8], camera: &ViewCamera, world: Vec3) -> f32 {
    let (x, y) = project(camera, world);
    luminance_at(pixels, SIZE, x, y, 2)
}

/// The four lamp positions the sweep uses: one per side, all at the same
/// height and distance, so the only thing that differs between them is
/// which face of the cube map the shadow has to come from.
const SWEEP: [Vec3; 4] = [
    Vec3::new(5.0, 6.0, 0.0),
    Vec3::new(-5.0, 6.0, 0.0),
    Vec3::new(0.0, 6.0, 5.0),
    Vec3::new(0.0, 6.0, -5.0),
];

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
fn move_cube(resources: &Resources, to: Vec3) {
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
