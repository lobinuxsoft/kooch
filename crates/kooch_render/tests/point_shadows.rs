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
