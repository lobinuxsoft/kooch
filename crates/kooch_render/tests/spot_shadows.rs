//! A spot light casts a shadow, on a real GPU (#777).
//!
//! Its own file rather than more of `csm_shadows.rs`: the scene needs no
//! sun, and the two suites answer different questions. What they share
//! is the shape — a cube over a floor, one light, and a camera that can
//! see the ground beside the cube.
//!
//! # What the unit tests could not reach
//!
//! `shadow::spot` covers the frustum: the cone is covered, the basis
//! survives a light pointing straight down, reversed-Z runs the right
//! way. All of it passed while the first smoke showed a wedge instead of
//! a sphere's shadow, because the fault was in the cull's LOD selector —
//! two files downstream. Only rendering catches that.

mod common;

use common::{build_cube_mesh, luminance_at, read_rgba8, try_acquire_device};
use glam::{Mat4, Quat, Vec3};
use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::registry::ComponentRegistry;
use kooch_ecs::hierarchy::global_transform::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::spot_light::SpotLight;
use kooch_render::ViewCamera;
use kooch_render::material::{Material, MaterialPipeline};
use kooch_render::meshlet::{MeshletRenderStage, MeshletRenderStageConfig, build_default_meshlets};
use kooch_render::shadow::ShadowSettings;

const SIZE: u32 = 256;

/// Where the spot sits. Off to one side and above, so its shadow of the
/// cube lands beside the cube rather than under it, where the cube
/// itself would hide it from the camera.
const SPOT_POSITION: Vec3 = Vec3::new(3.0, 6.0, 0.0);

/// Centre of the cube. Its underside is at y = 1.
const CUBE_CENTRE: Vec3 = Vec3::new(0.0, 1.5, 0.0);

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
    // Down but not off, for the reason `csm_shadows` gives: with no
    // ambient, anything shadowed is black, and "darker than lit" then
    // passes just as readily for a shadow that swallowed the whole floor.
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
        camera: ViewCamera::looking_at(Vec3::new(0.0, 4.0, 9.0), Vec3::new(0.0, 0.5, 0.0)),
    })
}

/// Adds the spot, pointed at the origin, casting or not.
fn add_spot(resources: &mut Resources, cast_shadows: bool) {
    let direction = (Vec3::ZERO - SPOT_POSITION).normalize();
    let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, direction);
    let mut commands = Commands::new();
    commands
        .spawn(resources)
        .insert(SpotLight {
            active: true,
            color: Vec3::ONE,
            intensity: 4_000_000.0,
            range: 40.0,
            inner_angle: 25.0,
            outer_angle: 40.0,
            cast_shadows,
            // The map, alone. A contact shadow would darken the same
            // floor for a completely different reason and this suite
            // would stop being about the shadow map.
            contact_shadows: false,
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(SPOT_POSITION) * Mat4::from_quat(rotation),
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

/// Where the cube's shadow lands: its centre traced away from the light
/// down to the floor. Computed, never hard-coded — a fixed pixel keeps
/// passing after someone moves the camera, by sampling the background,
/// which is dark enough to satisfy every "this is darker" assertion.
fn shadow_centre() -> Vec3 {
    let direction = (CUBE_CENTRE - SPOT_POSITION).normalize();
    CUBE_CENTRE + direction * (CUBE_CENTRE.y / direction.y).abs()
}

fn luminance(pixels: &[u8], camera: &ViewCamera, world: Vec3) -> f32 {
    let (x, y) = project(camera, world);
    luminance_at(pixels, SIZE, x, y, 2)
}

/// 🔴 The assertion #777 exists to make true.
///
/// It fails on `development`, where a spot light casts nothing at all.
#[test]
fn a_spot_light_casts_the_cube_onto_the_floor() {
    let Some(mut rig) = build_rig() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    add_spot(&mut rig.resources, false);
    let without = render(&mut rig);

    // A fresh rig rather than mutating the light: the shadow pass
    // allocates its array on the first frame that something casts, and
    // reusing the rig would test the reallocation path instead.
    let Some(mut casting) = build_rig() else {
        return;
    };
    add_spot(&mut casting.resources, true);
    let with = render(&mut casting);

    let camera = casting.camera;
    let unlit = luminance(&without, &camera, shadow_centre());
    let shadowed = luminance(&with, &camera, shadow_centre());

    assert!(
        unlit > 0.05,
        "the floor where the shadow should land is already dark ({unlit:.4}) \
         without a caster — the spot is not lighting the spot being measured",
    );
    assert!(
        shadowed < unlit * 0.7,
        "turning cast_shadows on changed the floor from {unlit:.4} to \
         {shadowed:.4} — the cube is not casting",
    );
}

/// The other half, and the one that catches a shadow that swallowed the
/// whole floor: ground the light reaches and the cube does not block
/// must be as bright with casting on as with it off.
#[test]
fn the_floor_the_spot_reaches_is_unchanged_by_casting() {
    let Some(mut rig) = build_rig() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    // Well outside the cube's shadow but inside the cone.
    let lit = Vec3::new(-2.5, 0.0, 0.0);

    add_spot(&mut rig.resources, false);
    let without = render(&mut rig);
    let before = luminance(&without, &rig.camera, lit);

    let Some(mut casting) = build_rig() else {
        return;
    };
    add_spot(&mut casting.resources, true);
    let with = render(&mut casting);
    let after = luminance(&with, &casting.camera, lit);

    assert!(before > 0.05, "the sample point is not lit to begin with");
    assert!(
        after > before * 0.8,
        "lit floor went from {before:.4} to {after:.4} when casting turned \
         on — the shadow is covering ground the cube does not block",
    );
}
