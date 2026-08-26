//! A point light casts a shadow, on a real GPU (#778).
//!
//! Its own file for the reason `spot_shadows.rs` has one: no sun, and a
//! different question. What it adds over that suite is the question only
//! a cube map raises — **does the shadow land on the right side of the
//! lamp** — and that one needs the light moved around the object, not a
//! single pose.
//!
//! # Why the direction sweep is the test that matters
//!
//! Cube maps are left-handed and this engine is not, so the Z faces are
//! stored swapped *and* the sampling direction is mirrored. Correct
//! either half alone and every shadow appears on the opposite side of
//! its lamp — which, from one fixed camera, looks exactly like a shadow
//! that works. `the_shadow_falls_away_from_the_light` is what separates
//! them.
//!
//! Run with:
//!   cargo test -p kooch_render --test point_shadows

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

/// Floor well clear of everything. Nothing can shadow it, so it is what
/// "lit" means in this scene.
const OPEN_FLOOR: Vec3 = Vec3::new(-6.0, 0.0, 5.0);

struct Rig {
    device: wgpu::Device,
    queue: wgpu::Queue,
    resources: Resources,
    stage: MeshletRenderStage,
    camera: ViewCamera,
    /// So a test can add geometry of its own — the suite's own cube
    /// stands ON the floor, and a light directly above it hides its
    /// shadow under it.
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
        // 🔴 Nearly overhead, and that is a requirement rather than a
        // framing choice. The sweep measures the floor on all four
        // sides of the cube, and from a low camera the cube stands
        // between the lens and the two points on its own axis: at
        // (0, 9, 11) the line of sight to (0, 0, -1.67) passes through
        // y ≈ 1.5, which is inside the cube. Those samples then read the
        // CUBE's brightness and the sweep reports a shadow bug that is
        // really an occluded sample.
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
            // The cube map alone. A contact shadow would darken the same
            // floor for a different reason and the suite would stop
            // being about the cube map.
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

/// Where the cube's shadow lands for a light at `light`: the cube's
/// centre traced away from the light down to the floor.
///
/// Computed, never hard-coded — a fixed pixel keeps passing after
/// somebody moves the camera, by sampling the background, which is dark
/// enough to satisfy every "this is darker" assertion.
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

/// 🔴 The A/B that makes the suite mean something: the same pixel, two
/// renders, one flag apart.
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
///
/// A cube map is six faces in a left-handed space this engine does not
/// otherwise use. Mirror the sampling direction without swapping the
/// stored faces — or the reverse — and every shadow lands on the
/// **opposite** side of its lamp. From one camera angle that is
/// indistinguishable from a working shadow, so the light has to move.
#[test]
fn the_shadow_falls_away_from_the_light() {
    // Every position is measured before anything is asserted. Failing on
    // the first one hides whether the fault is one face or a whole axis,
    // and those have different causes.
    let mut report = Vec::new();
    for light in SWEEP {
        let Some(mut rig) = build_rig() else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        add_point(&mut rig.resources, light, true);
        let pixels = render(&mut rig);

        let away = shadow_centre(light);
        // The mirror image of that point through the cube: where the
        // shadow would be if a sign were wrong. It is lit floor, and it
        // has to stay lit.
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
///
/// Every other test in this file renders once, and the first frame of a
/// cached cube is always drawn. What a cache can break only shows up on
/// the second frame: a shadow that stays where it was. That failure is
/// silent, survives every single-frame assertion in this suite, and gets
/// blamed on the light, the material and the camera before anyone
/// suspects the thing that skipped the work.
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
///
/// Every test above runs with `ShadingSettings::default()`, whose
/// `compute` is **false** — the fragment path. A project that turns
/// `compute_shading` on in its `.rendersettings`, which is what
/// `roll-a-ball` ships and what every performance capture on record was
/// taken with, shades through `shade_from_tile` in
/// `material_pbr_compute.wgsl`: a second copy of the light walk, with
/// its own bindings.
///
/// This suite never touched it. The engine has been caught by exactly
/// this once before — the first GPU-scope test passed with the R64
/// scopes deleted, because the device was taking the other path — and
/// the answer then was one test per path.
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
///
/// `roll-a-ball` runs `shading_rate: 2` — one shaded sample per 2x2
/// quad, upsampled back with the visibility buffer as the edge guide.
/// A cube map's shadow edge is high frequency and the upsample is the
/// only thing between it and the screen, so it gets its own test rather
/// than being assumed to follow from the full-rate one.
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
///
/// Every test above uses `range: 40` and four million lux, which is a
/// lamp that reaches the whole rig. `many_lights.scene` — the one whose
/// shadows were reported broken — authors `range: 4.0` at 60 000, a lamp
/// that reaches barely past the object it stands over. The cube's stored
/// depth is `near / major_axis` and its `depth_extent` is the range, so
/// a tenth of the reach is a tenth of the depth precision and a
/// different penumbra estimate.
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
    // Close enough that a 4 m sphere still covers the cube and the floor
    // beside it, which is what the authored scene does.
    // Close and low, because 4 m of reach is all it has: from (1.5, 2)
    // the sphere's edge lands short of the floor either side of the cube
    // and BOTH samples read pure ambient — 0.0791, the number this repo
    // has been fooled by before. A test whose light does not reach its
    // own samples reports "no shadow" for a shader that is working.
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

    // Opposite the lamp, just past the cube's edge, and a lit point at
    // the same distance from the lamp so the two differ by the shadow
    // and not by falloff.
    let shadowed = luminance(&pixels, &rig.camera, Vec3::new(-0.9, 0.0, 0.0));
    let lit = luminance(&pixels, &rig.camera, Vec3::new(0.0, 0.0, 1.3));
    // 🔴 A weaker margin than the rest of the suite, and it is the
    // finding rather than a concession. At `range: 4` the shadowed
    // sample sits far enough down the falloff that the rig's ambient
    // (200 lx, which casts nothing) is a large share of what is left, so
    // removing the lamp's contribution entirely can only darken it so
    // much: 0.198 against 0.270, a 27 % drop, where the 40 m lamp gives
    // well over 50 %. A short-range lamp's shadow is faint by
    // construction, before any shader is blamed for it.
    assert!(
        shadowed < lit * 0.8,
        "a 4 m lamp leaves the floor under the cube at {shadowed} against {lit} lit",
    );
}

/// 🔴🔴 A cube map cannot depend on where the camera is.
///
/// Six faces rendered from the LIGHT, sampled by direction. Nothing in
/// that chain has a view matrix in it, so the same floor point must come
/// back the same brightness from any angle. Reported from the editor:
/// *"dependiendo del ángulo de la cámara se muestra o no la sombra"*.
///
/// Sampled well away from the silhouette, and with a rough material, so
/// the only view-dependent term left is a specular lobe worth a few
/// percent rather than the difference between shadowed and lit.
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

/// 🔴 A point light is a point. Its falloff on a flat floor is a set of
/// circles, and anything with a corner in it comes from the cube, not
/// from the light.
///
/// Reported as a visible **square** in the lit floor around the lamp.
/// Four floor points at the same distance from the light, on the two
/// axes, are the same distance and the same angle — they can only differ
/// by which cube face answers for them.
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

    // 🔴 Radius 7, and the number is the whole test. The downward cube
    // face spans 90°, so from 6 m up it covers the floor out to 6 m
    // along each axis and 8.5 m along each diagonal. At radius 4 every
    // probe lands inside that one face and the test passes without
    // asking anything — which is what the first version of it did.
    //
    // At 7 the axis probes have crossed onto the SIDE faces while the
    // diagonal probes are still on the bottom one, at the same distance
    // from the lamp and so with the same falloff. Anything left between
    // them is the seam.
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

/// One lamp, but the budget the owner had set when the artifacts were
/// reported.
///
/// The cube array is allocated at `point_shadows` and the shader indexes
/// it by `shadow_slot`, so a budget far larger than the number of lights
/// exercises an array whose live slots are a small prefix of its layers.
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
///
/// Reported from the editor: *"si la luz no está dentro de la cámara
/// tampoco se ven las sombras"*. The classic form of this defect is a
/// shadow pass fed the CAMERA's visible set instead of the light's, and
/// it is invisible in every test that frames the occluder.
///
/// Here the lamp is low and to one side, so the cube throws a long
/// shadow, and the camera looks straight down at a patch of that shadow
/// with the cube itself outside the frame.
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
///
/// Reported from the editor with a picture: the lamp standing straight
/// over the ball casts **nothing**, while a second lamp off to the side
/// casts a clean shadow in the same frame, same scene, same settings.
///
/// The whole suite lights from `SWEEP` — (±5, 6, 0) and (0, 6, ±5) —
/// every one of them off to a side. A shadow cast straight down is
/// resolved almost entirely by the cube's **−Y face**, and no assertion
/// here had ever asked that face a question.
///
/// The occluder floats, because the suite's own cube stands on the floor
/// and its shadow at noon hides underneath it.
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
///
/// Every other test here puts an occluder in the scene and then measures
/// that the floor got darker somewhere. That question cannot fail on a
/// bias: acne makes the floor darker too, and darker was what the
/// assertion wanted. So a shadow map printing a hard stair-stepped
/// square onto an empty floor passed thirteen of them.
///
/// This asks the opposite question. The cube is moved out of the frame,
/// so **nothing in the scene can cast anything**, and the same floor is
/// rendered twice — once with the lamp casting, once not. Any darkening
/// between the two is the floor shadowing itself, and the correct amount
/// is zero.
///
/// It was the owner who saw the square, from a chair, after this file
/// had grown to thirteen green assertions. The lesson is the shape of
/// the question, not the count.
#[test]
fn an_empty_floor_is_not_shadowed_by_itself() {
    // Well inside the lamp's reach and spread across the boundary where
    // a cube face hands over to its neighbour — under a lamp `h` up
    // that is a square of side 2h, and the seam ran along it.
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
    // Out of frame and out of the lamp's range, so the only geometry
    // left is the floor itself.
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

/// 🔴🔴 Two views on one stage — the editor's arrangement, which this
/// suite never had.
///
/// `render_with_assets(view_id, ..)` runs a whole frame per view and the
/// shadow preparation takes the CAMERA, while the cube array, the cube
/// cache and the holders belong to the stage. So the selection used to
/// cull lamps against whoever was rendering, and the last view to render
/// decided for both: a lamp outside the gameplay camera lost its cube,
/// and the View panel — looking straight at it — drew no shadow.
///
/// The Game camera here is pointed forty metres away on purpose. That is
/// the case that failed, and it failed silently in every single-view
/// picture this file takes.
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
///
/// 🔴 The one this suite could not see. Every other test here lights the
/// scene with a single lamp, and with a single lamp the pass is correct.
///
/// The cube cull's parameters were uploaded with `queue.write_buffer`,
/// which is not ordered against the encoder: everything queued while a
/// frame is recorded lands before the first command runs. The six cull
/// objects belong to the cube FACE and are shared by every lamp, so the
/// second lamp's frustum overwrote the first's, both cubes were culled
/// against it, and each was still rasterised with its own matrix. The
/// first lamp lost every occluder the second one could not see.
///
/// It hid behind the cube cache, too: a lamp that MOVES is redrawn on
/// its own, one dispatch, nothing to overwrite. So the shadow appeared
/// while the lamp moved and died when it stopped, which is how it was
/// reported.
#[test]
fn a_second_lamp_does_not_erase_the_first_shadow() {
    let (Some(mut alone), Some(mut together)) = (build_rig(), build_rig()) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    // Opposite sides, so neither shadow reaches the other's spot.
    let (first, second) = (SWEEP[0], SWEEP[1]);
    // The two rigs differ by ONE flag: whether the second lamp casts.
    // It lights the scene identically either way, and at the FIRST
    // lamp's shadow centre it is not blocked by anything — so its
    // casting cannot legally change that pixel by any amount.
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
    // Only a few percent, and that is correct rather than weak: shadows
    // multiply per light and lights add, so blocking one of two lamps
    // can never take more than that lamp's share of the pixel. The
    // guard is there to catch "no shadow at all" — which is what the
    // defect produced, exactly equal to `open`.
    assert!(
        one < open * 0.95,
        "the first lamp casts no shadow at all: {one} against {open} with it not casting",
    );
}
