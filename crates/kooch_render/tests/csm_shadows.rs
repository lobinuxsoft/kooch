//! #476's acceptance: the sun casts.
//!
//! `cascades.rs`, `atlas.rs` and `raster.rs` each have unit tests, and
//! every one of them passed while nothing in the engine constructed a
//! `ShadowAtlas` — the scene rendered exactly as it had before the
//! branch. These are the tests that fail when the pass is not wired in.
//!
//! The scene is a cube on a floor, lit by one sun at an angle. Every
//! assertion below compares the **same pixel** across two renders that
//! differ by one flag, rather than comparing two places in one image:
//! two places differ for a dozen legitimate reasons (N·L, the cascade
//! they land in, distance), and one flag differing is the property under
//! test.
//!
//! Run with:
//!   cargo test -p kooch_render --test csm_shadows

mod common;

use common::{build_cube_mesh, luminance_at, read_rgba8, try_acquire_device};
use glam::{Mat4, Quat, Vec3};
use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::registry::ComponentRegistry;
use kooch_ecs::directional_light::DirectionalLight;
use kooch_ecs::hierarchy::global_transform::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_ecs::query::AccessTracker;
use kooch_render::ViewCamera;
use kooch_render::material::{Material, MaterialPipeline};
use kooch_render::meshlet::{MeshletRenderStage, MeshletRenderStageConfig, build_default_meshlets};
use kooch_render::shadow::ShadowSettings;

const SIZE: u32 = 256;

/// Where the sun shines, normalised. Tilted rather than straight down so
/// the shadow lands beside the cube instead of underneath it, where the
/// cube itself would hide it from the camera.
const SUN: Vec3 = Vec3::new(0.5, -1.0, 0.0);

/// Centre of the cube. Its underside is at y = 1.
const CUBE_CENTRE: Vec3 = Vec3::new(0.0, 1.5, 0.0);

struct Rig {
    device: wgpu::Device,
    queue: wgpu::Queue,
    resources: Resources,
    stage: MeshletRenderStage,
    camera: ViewCamera,
}

/// A cube floating over a wide flat floor, one sun, and a camera that
/// can see the ground beside the cube.
fn rig() -> Option<Rig> {
    let (device, queue) = try_acquire_device()?;

    let meshlet_mesh = build_default_meshlets(&build_cube_mesh()).expect("build meshlets");

    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());
    // A small cascade: this scene is metres across, and 2048² × 4
    // cascades is 64 MiB of atlas for a test that reads one pixel.
    resources.insert(ShadowSettings {
        cascade_texels: 1024,
        max_distance: 60.0,
        enabled: true,
        ..Default::default()
    });
    // Ambient down but not off. Off would make anything in shadow black,
    // and "darker than lit" would then pass for a shadow that swallowed
    // the whole floor as readily as for a correct one.
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
    // The floor: the same cube, flattened. Its top face sits at y = 0.
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
        camera: ViewCamera::looking_at(Vec3::new(0.0, 4.0, 9.0), Vec3::new(0.0, 0.5, 0.0)),
    })
}

/// Adds the sun, casting or not.
fn add_sun(resources: &mut Resources, cast_shadows: bool) {
    let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, SUN.normalize());
    let mut commands = Commands::new();
    commands
        .spawn(resources)
        .insert(DirectionalLight {
            active: true,
            color: Vec3::ONE,
            intensity: 20_000.0,
            cast_shadows,
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_quat(rotation),
        });
    commands.apply(resources);
}

fn render(rig: &mut Rig) -> Vec<u8> {
    rig.stage
        .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &rig.camera, 1.0);
    read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture())
}

/// World point → pixel, through the same matrix the stage rendered with.
///
/// Computed rather than hard-coded: a hard-coded pixel keeps passing
/// after someone moves the camera, and it passes by sampling the
/// background, which is uniformly dark and therefore satisfies every
/// "this is darker" assertion for the wrong reason.
fn project(camera: &ViewCamera, world: Vec3) -> (u32, u32) {
    let clip = camera.view_proj(1.0) * world.extend(1.0);
    let ndc = clip.truncate() / clip.w;
    let x = ((ndc.x * 0.5 + 0.5) * SIZE as f32).clamp(0.0, (SIZE - 1) as f32);
    // Row 0 is the top of the image, so y flips.
    let y = ((0.5 - ndc.y * 0.5) * SIZE as f32).clamp(0.0, (SIZE - 1) as f32);
    (x as u32, y as u32)
}

/// Where the cube's shadow lands on the floor: the cube's centre traced
/// along the sun to y = 0.
fn shadow_centre() -> Vec3 {
    let direction = SUN.normalize();
    // Along the direction the light travels, not back toward it.
    CUBE_CENTRE + direction * (CUBE_CENTRE.y / direction.y).abs()
}

fn luminance(pixels: &[u8], camera: &ViewCamera, world: Vec3) -> f32 {
    let (x, y) = project(camera, world);
    luminance_at(pixels, SIZE, x, y, 2)
}

/// 🔴 The assertion the whole issue exists to make true.
///
/// Everything on the branch was tested and correct before this passed,
/// because nothing constructed the atlas.
#[test]
fn a_cube_over_a_floor_casts_a_shadow_on_it() {
    let Some(mut base) = rig() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    add_sun(&mut base.resources, false);
    let without = render(&mut base);

    let mut rig_casting = rig().expect("device acquired once already");
    add_sun(&mut rig_casting.resources, true);
    let with = render(&mut rig_casting);

    let camera = base.camera;
    let shadowed_without = luminance(&without, &camera, shadow_centre());
    let shadowed_with = luminance(&with, &camera, shadow_centre());

    assert!(
        shadowed_without > 0.05,
        "the floor where the shadow should land is already dark without \
         one ({shadowed_without:.4} linear) — this test would pass on a \
         scene that never rendered",
    );
    assert!(
        shadowed_with < shadowed_without * 0.7,
        "the floor under the cube is {shadowed_with:.4} with shadows and \
         {shadowed_without:.4} without (linear) — nothing occluded the sun",
    );
}

/// The other half, and the one that catches an inverted comparison: a
/// sampler set to `Less` instead of `Greater` darkens everything the sun
/// *reaches* and lights what it does not, which the test above would
/// happily pass.
#[test]
fn the_floor_the_sun_reaches_is_unchanged_by_casting() {
    let Some(mut base) = rig() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    add_sun(&mut base.resources, false);
    let without = render(&mut base);

    let mut rig_casting = rig().expect("device acquired once already");
    add_sun(&mut rig_casting.resources, true);
    let with = render(&mut rig_casting);

    // Well clear of the cube and of its shadow, still inside cascade 0.
    let open_floor = Vec3::new(-5.0, 0.0, 2.0);
    let camera = base.camera;
    let lit_without = luminance(&without, &camera, open_floor);
    let lit_with = luminance(&with, &camera, open_floor);

    assert!(
        lit_without > 0.05,
        "the sample point is not on lit floor ({lit_without:.4} linear)",
    );
    assert!(
        (lit_with - lit_without).abs() < lit_without * 0.25,
        "open floor went from {lit_without:.4} to {lit_with:.4} (linear) \
         when the sun started casting — the whole scene is being \
         shadowed, which is what a reversed comparison or a clear to 1.0 \
         looks like",
    );
}

/// `DirectionalLight::cast_shadows` was authored, serialised, mirrored
/// over the remote protocol and read by nobody. This is the test that
/// says otherwise.
#[test]
fn clearing_cast_shadows_turns_the_shadow_off() {
    let Some(mut base) = rig() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    add_sun(&mut base.resources, true);
    let casting = render(&mut base);

    let mut rig_off = rig().expect("device acquired once already");
    add_sun(&mut rig_off.resources, false);
    let not_casting = render(&mut rig_off);

    assert_ne!(
        casting, not_casting,
        "flipping cast_shadows changed no pixel, so the flag is still \
         decoration",
    );
}

/// Turning shadows off in the settings has to reach the pass, not just
/// the asset — and it is also what frees the atlas.
#[test]
fn the_project_can_turn_shadows_off() {
    let Some(mut base) = rig() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    add_sun(&mut base.resources, true);
    let on = render(&mut base);

    let mut rig_off = rig().expect("device acquired once already");
    add_sun(&mut rig_off.resources, true);
    rig_off.resources.insert(ShadowSettings {
        enabled: false,
        ..ShadowSettings::default()
    });
    let off = render(&mut rig_off);

    let camera = base.camera;
    assert!(
        luminance(&off, &camera, shadow_centre()) > luminance(&on, &camera, shadow_centre()),
        "shadows stayed on with ShadowSettings::enabled = false",
    );
}

/// How many pixels along a horizontal line through the shadow are
/// neither clearly lit nor clearly shadowed — the width of the edge.
fn edge_width(pixels: &[u8], camera: &ViewCamera, centre: Vec3) -> usize {
    let (_, y) = project(camera, centre);
    let lit = luminance_at(pixels, SIZE, SIZE - 20, y, 1);
    let dark = luminance(pixels, camera, centre);
    let low = dark + (lit - dark) * 0.25;
    let high = dark + (lit - dark) * 0.75;
    (0..SIZE)
        .filter(|&x| {
            let l = luminance_at(pixels, SIZE, x, y, 0);
            l > low && l < high
        })
        .count()
}

/// 🔴 The penumbra term used to be multiplied by a magic 0.001 and lost
/// every `max()` against the fixed search radius, so PCSS was a PCF with
/// eight wasted taps. This is the test that says the estimate reaches
/// the filter: a wider sun has to blur the edge more.
#[test]
fn a_wider_sun_softens_the_shadow_edge() {
    let Some(mut sharp) = rig() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    sharp.resources.insert(ShadowSettings {
        sun_softness: 0.0,
        ..*sharp
            .resources
            .get::<ShadowSettings>()
            .expect("rig sets one")
    });
    add_sun(&mut sharp.resources, true);
    let hard = render(&mut sharp);

    let mut soft_rig = rig().expect("device acquired once already");
    soft_rig.resources.insert(ShadowSettings {
        sun_softness: 0.25,
        ..*soft_rig
            .resources
            .get::<ShadowSettings>()
            .expect("rig sets one")
    });
    add_sun(&mut soft_rig.resources, true);
    let soft = render(&mut soft_rig);

    let camera = sharp.camera;
    let hard_edge = edge_width(&hard, &camera, shadow_centre());
    let soft_edge = edge_width(&soft, &camera, shadow_centre());

    assert!(
        soft_edge > hard_edge,
        "edge was {hard_edge} px at softness 0 and {soft_edge} px at 0.25 — \
         the penumbra estimate is not reaching the filter, which is what \
         the magic 0.001 used to prevent",
    );
}
