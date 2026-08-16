//! Not a test — a **look**. The owner's scene, rendered to PNGs (#851).
//!
//! Thirteen assertions in `point_shadows.rs` pass while the owner sees
//! shadows that die, a square drawn outward, and a different picture in
//! each camera. When that happens the rig is wrong, not the eye. This
//! reproduces the reported scene by its numbers — `intensity 320000`,
//! `range 10.0`, `ambient 0.0`, compute shading — and writes pictures
//! instead of asserting, because every assertion so far has agreed with
//! the code and disagreed with the screen.
//!
//! Run with:
//!   cargo test -p kooch_render --test point_shadow_dump -- --ignored --nocapture

mod common;

use common::{build_cube_mesh, read_rgba8, try_acquire_device};
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

const SIZE: u32 = 512;

/// The lamp the inspector showed, overhead.
const OVERHEAD: Vec3 = Vec3::new(0.0, 3.477, 0.0);
/// The reported values, not the suite's 4 000 000 over 40 m.
const INTENSITY: f32 = 320_000.0;
const RANGE: f32 = 10.0;

struct Rig {
    device: wgpu::Device,
    queue: wgpu::Queue,
    resources: Resources,
    stage: MeshletRenderStage,
}

/// `occluder` places a cube standing on the floor. Without one the floor
/// is empty, and then **any** darkening in the picture is a defect:
/// nothing in the scene can shadow anything.
fn build(lights: &[(Vec3, bool)], occluder: bool, compute: bool) -> Option<Rig> {
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
    resources.insert(kooch_lighting::AmbientLight {
        intensity: 0.0,
        ..Default::default()
    });
    // What `roll-a-ball` ships and what every capture on record used.
    resources.insert(kooch_render::quality::ShadingSettings {
        compute,
        ..Default::default()
    });

    let mut materials = MaterialPipeline::with_capacity(&device, &queue, 4);
    let material = Guid::new_v4();
    materials.register(
        &queue,
        material,
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
    let mesh = Guid::new_v4();
    stage.ensure_gpu_mesh(&device, mesh, &meshlet_mesh);

    let mut commands = Commands::new();
    let mut spawn = |matrix: Mat4| {
        commands
            .spawn(&mut resources)
            .insert(MeshRenderer {
                mesh: Some(mesh),
                material: Some(material),
                visible: true,
                ..Default::default()
            })
            .insert(GlobalTransform { matrix });
    };
    spawn(
        Mat4::from_translation(Vec3::new(0.0, -0.25, 0.0))
            * Mat4::from_scale(Vec3::new(20.0, 0.5, 20.0)),
    );
    if occluder {
        spawn(Mat4::from_translation(Vec3::new(0.0, 0.5, 0.0)));
    }

    for (position, cast_shadows) in lights {
        commands
            .spawn(&mut resources)
            .insert(PointLight {
                active: true,
                color: Vec3::ONE,
                intensity: INTENSITY,
                range: RANGE,
                radius: 0.0,
                cast_shadows: *cast_shadows,
                contact_shadows: false,
            })
            .insert(GlobalTransform {
                matrix: Mat4::from_translation(*position),
            });
    }
    commands.apply(&mut resources);

    Some(Rig {
        device,
        queue,
        resources,
        stage,
    })
}

fn shoot(rig: &mut Rig, name: &str, eye: Vec3) {
    let camera = ViewCamera::looking_at(eye, Vec3::new(0.0, 0.5, 0.0));
    rig.stage
        .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &camera, 1.0);
    let pixels = read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture());
    let path = std::path::Path::new("/tmp/kooch_point_shadows").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::save_buffer(&path, &pixels, SIZE, SIZE, image::ColorType::Rgba8).unwrap();
    eprintln!("wrote {}", path.display());
}

/// Moves the cube that stands ON the floor, leaving the floor alone.
fn move_occluder(resources: &Resources, to: Vec3) {
    kooch_ecs::query::Query::<(&MeshRenderer, &mut GlobalTransform)>::new(resources).for_each(
        |(_, transform)| {
            if transform.matrix.w_axis.y > 0.0 {
                transform.matrix = Mat4::from_translation(to);
            }
        },
    );
}

/// Observation 2 — "las point lights dibujan un cuadrado de sombra hacia
/// afuera".
///
/// An EMPTY floor under one lamp. There is nothing to cast, so the
/// picture must be a smooth pool of light. A square, a diamond or a
/// diagonal seam is the cube's own face boundary printed onto the world.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn an_empty_floor_must_be_smooth() {
    let top = Vec3::new(0.0, 14.0, 0.1);
    for compute in [false, true] {
        let Some(mut rig) = build(&[(OVERHEAD, true)], false, compute) else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        let tag = if compute { "compute" } else { "fragment" };
        // From high up, so the whole footprint of the -Y face is in
        // frame: the lamp is 3.477 m up and a 90 deg face covers
        // +/- 3.477 m of floor, which is where a seam would fall.
        shoot(&mut rig, &format!("seam_{tag}_top.png"), top);
        shoot(
            &mut rig,
            &format!("seam_{tag}_angle.png"),
            Vec3::new(7.0, 6.0, 7.0),
        );
    }

    // The control. Same lamp, same floor, `cast_shadows` OFF — no cube
    // is sampled at all. If the square survives this it is not the cube
    // map and every word above is wrong.
    let Some(mut off) = build(&[(OVERHEAD, false)], false, true) else {
        return;
    };
    shoot(&mut off, "seam_control_no_cube.png", top);

    // And the same lamp at half the height. A 90 deg face covers
    // +/- h of floor, so if the square is the face footprint its side
    // must halve with it. Nothing else in the scene scales that way.
    let Some(mut low) = build(&[(Vec3::new(0.0, 1.74, 0.0), true)], false, true) else {
        return;
    };
    shoot(&mut low, "seam_half_height.png", top);

    // The number the eye cannot give. Same empty floor twice, the only
    // difference being whether a cube is sampled at all, so every
    // non-zero pixel of the difference is the floor shadowing itself.
    let with = grab(&[(OVERHEAD, true)], top);
    let without = grab(&[(OVERHEAD, false)], top);
    let (worst, mean) = compare(&with, &without);
    eprintln!("empty floor self-shadowing: worst {worst:.4}, mean {mean:.5} (0 is correct)");
}

/// Renders the empty floor once and returns the pixels.
fn grab(lights: &[(Vec3, bool)], eye: Vec3) -> Vec<u8> {
    let mut rig = build(lights, false, true).expect("device");
    let camera = ViewCamera::looking_at(eye, Vec3::new(0.0, 0.5, 0.0));
    rig.stage
        .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &camera, 1.0);
    read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture())
}

/// Worst and mean per-pixel darkening of `a` against `b`, 0 to 1.
fn compare(a: &[u8], b: &[u8]) -> (f32, f32) {
    let mut worst = 0.0f32;
    let mut total = 0.0f64;
    let count = a.len() / 4;
    for i in 0..count {
        let d = (b[i * 4] as f32 - a[i * 4] as f32).max(0.0) / 255.0;
        worst = worst.max(d);
        total += d as f64;
    }
    (worst, (total / count as f64) as f32)
}

/// Observation 1 — "si muevo la luz se actualizan las sombras, si no la
/// muevo mueren".
///
/// The lamp never moves. The caster does, three times, through ONE
/// stage so the cube cache is live. Frame 1 is always drawn; what a
/// cache breaks only shows on frame 2 and after.
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
///
/// Two cameras, one stage, one frame's worth of state, nothing moved
/// between them. The cube maps are rendered from the LIGHT, so the two
/// pictures must agree about where the shadow is. If they do not, the
/// cube's contents depend on who is looking — which is the one thing a
/// shadow map must never do.
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
    // Back to the first camera. Same eye as `agree_0_warm`, so the two
    // must be the same picture — anything else is the cube remembering
    // who looked at it last.
    shoot(&mut rig, "agree_3_back.png", Vec3::new(6.0, 5.0, 6.0));
}
