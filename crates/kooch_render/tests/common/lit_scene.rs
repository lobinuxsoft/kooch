//! A floor under a grid of point lights — the scene both shading-path test binaries render (#824,
//! #825).

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
use kooch_render::meshlet::{
    MeshletDebugCaps, MeshletRenderStage, MeshletRenderStageConfig, ShadingRate,
    build_default_meshlets,
};
use kooch_render::shadow::ShadowSettings;
use kooch_render::vbuf64::Vbuf64Support;

use super::{build_cube_mesh, read_rgba8, try_acquire_device_r64};

/// Deliberately not a multiple of the 16-pixel tile, and deliberately even — odd would additionally
/// exercise the half-rate quad that hangs off the right edge, which is worth its own case rather
/// than being mixed into every assertion.
pub const SIZE: u32 = 200;

pub struct Rig {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub resources: Resources,
    pub stage: MeshletRenderStage,
    pub camera: ViewCamera,
    /// The material every box in the scene renders with.
    pub material: Guid,
    /// The cube every box in the scene is.
    pub mesh: Guid,
}

/// A floor, `lights x lights` point lights above it, and a camera looking down the length of it.
pub fn rig(lights: u32, wall: bool) -> Option<Rig> {
    build(lights, wall, false, false, false)
}

/// [`rig`] on the legacy R32 visibility buffer and its Hi-Z two-pass cull.
pub fn rig_r32(lights: u32, wall: bool) -> Option<Rig> {
    build(lights, wall, false, false, true)
}

/// The floor and wall on two materials, and a row of blocks on two more, so most tiles along their
/// edges hold several materials (#1157).
pub fn rig_mixed(lights: u32) -> Option<Rig> {
    build(lights, true, false, true, false)
}

/// The same scene plus **one blue shadow-casting light**, and the shadow atlas switched on so it is
/// given a slot.
pub fn rig_with_caster(lights: u32) -> Option<Rig> {
    build(lights, true, true, false, false)
}

fn build(lights: u32, wall: bool, caster: bool, mixed: bool, r32: bool) -> Option<Rig> {
    let (device, queue) = try_acquire_device_r64()?;

    let meshlet_mesh = build_default_meshlets(&build_cube_mesh()).expect("build meshlets");

    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());
    // Shadows off: a shadow map is a second reason for two renders to
    // differ, and it is not the one under test.
    resources.insert(ShadowSettings {
        enabled: caster,
        ..Default::default()
    });
    resources.insert(kooch_lighting::AmbientLight {
        intensity: 50.0,
        ..Default::default()
    });

    let mut materials = MaterialPipeline::with_capacity(&device, &queue, 8);
    let material_guid = Guid::new_v4();
    materials.register(
        &queue,
        material_guid,
        &Material::new([0.8, 0.8, 0.8, 1.0], 0.1, 0.4, 0.0),
    );
    // Unused unless `mixed`: the other three looks, one per role.
    let others = [
        [0.9, 0.5, 0.1, 1.0],
        [0.1, 0.8, 0.4, 1.0],
        [0.2, 0.3, 0.9, 1.0],
    ]
    .map(|colour| {
        let guid = Guid::new_v4();
        materials.register(&queue, guid, &Material::new(colour, 0.0, 0.6, 0.0));
        guid
    });
    resources.insert(materials);

    // 🔴 `Vbuf64Support` defaults to *unsupported*, so a config built with `..Default::default()`
    // puts the stage on the R32 fallback however capable the device is — and the callers would then
    // compare the fragment path against itself. Detect from the device.
    let mut stage = MeshletRenderStage::new(
        &device,
        MeshletRenderStageConfig {
            size: (SIZE, SIZE),
            instance_capacity: 16,
            meshlet_capacity: 1024,
            vbuf64: match r32 {
                true => Vbuf64Support::from_supported(false),
                false => Vbuf64Support::detect(&device),
            },
            // The R64 path asserts the density accumulator exists; the
            // caps default says the device cannot have one.
            debug_caps: MeshletDebugCaps::detect(&device),
        },
    );
    let mesh_guid = Guid::new_v4();
    stage.ensure_gpu_mesh(&device, mesh_guid, &meshlet_mesh);

    let mut commands = Commands::new();
    let mut box_at = |resources: &mut Resources, matrix: Mat4, material: Guid| {
        commands
            .spawn(resources)
            .insert(MeshRenderer {
                mesh: Some(mesh_guid),
                material: Some(material),
                visible: true,
                ..Default::default()
            })
            .insert(GlobalTransform { matrix });
    };
    // 🔴 Flat, not a 20 m cube. Scaling all three axes puts the camera
    // *inside* the box, which backface-culls to nothing — and two empty
    // renders match perfectly. That is how these tests first passed.
    box_at(
        &mut resources,
        Mat4::from_translation(Vec3::new(0.0, -0.25, 0.0))
            * Mat4::from_scale(Vec3::new(20.0, 0.5, 20.0)),
        material_guid,
    );
    if wall {
        box_at(
            &mut resources,
            Mat4::from_translation(Vec3::new(0.0, 1.5, -2.0))
                * Mat4::from_scale(Vec3::new(6.0, 3.0, 0.4)),
            if mixed { others[0] } else { material_guid },
        );
    }
    if mixed {
        for i in 0..6 {
            box_at(
                &mut resources,
                Mat4::from_translation(Vec3::new(i as f32 * 0.9 - 2.25, 0.3, 1.5))
                    * Mat4::from_scale(Vec3::splat(0.6)),
                others[1 + i % 2],
            );
        }
    }

    // A grid of short-range lights: several reach any given point, and
    // which several depends on the froxel — which is the whole reason
    // the grid exists.
    let span = 3.0;
    for ix in 0..lights {
        for iz in 0..lights {
            let x = (ix as f32 - (lights as f32 - 1.0) * 0.5) * span;
            let z = (iz as f32 - (lights as f32 - 1.0) * 0.5) * span;
            commands
                .spawn(&mut resources)
                .insert(PointLight {
                    active: true,
                    color: Vec3::new(1.0, 0.9, 0.8),
                    intensity: 60_000.0,
                    range: 6.0,
                    radius: 0.1,
                    cast_shadows: false,
                    contact_shadows: false,
                })
                .insert(GlobalTransform {
                    matrix: Mat4::from_translation(Vec3::new(x, 1.6, z)),
                });
        }
    }
    if caster {
        commands
            .spawn(&mut resources)
            .insert(PointLight {
                active: true,
                // Cold, so its contribution is visible against the warm
                // grid rather than merely adding to it.
                color: Vec3::new(0.05, 0.2, 1.0),
                intensity: 60_000.0,
                // Reaches further than the grid does, so it lights
                // enough blocks for the count to mean something. At the
                // grid's 6 m it cleared only eleven.
                range: 14.0,
                radius: 0.1,
                cast_shadows: true,
                contact_shadows: false,
            })
            .insert(GlobalTransform {
                matrix: Mat4::from_translation(Vec3::new(0.0, 2.0, 1.0)),
            });
    }
    commands.apply(&mut resources);

    Some(Rig {
        device,
        queue,
        resources,
        stage,
        camera: ViewCamera::looking_at(Vec3::new(0.0, 2.5, 9.0), Vec3::new(0.0, 0.5, 0.0)),
        material: material_guid,
        mesh: mesh_guid,
    })
}

pub fn render(rig: &mut Rig, compute: bool) -> Vec<u8> {
    render_at(rig, compute, ShadingRate::Full)
}

/// Renders one frame on whatever path the rig's stage has, and reads the colour target back.
pub fn render_any(rig: &mut Rig) -> Vec<u8> {
    rig.stage
        .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &rig.camera, 1.0);
    read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture())
}

/// Renders one frame on the chosen path and rate, and reads the colour target back.
pub fn render_at(rig: &mut Rig, compute: bool, rate: ShadingRate) -> Vec<u8> {
    assert!(
        rig.stage.set_compute_shading(compute) > 0,
        "no view has the R64 stage — the shading tests would be vacuous",
    );
    assert!(
        rig.stage.set_shading_rate(rate) > 0,
        "no view took the shading rate {rate:?} — the assertion would be vacuous",
    );
    rig.stage
        .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &rig.camera, 1.0);
    read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture())
}
