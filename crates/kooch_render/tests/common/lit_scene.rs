//! A floor under a grid of point lights — the scene both shading-path
//! test binaries render (#824, #825).
//!
//! Shared rather than copied because the thing under test in both files
//! is *which path shaded these pixels*, and two scenes that drifted
//! apart would let a difference hide in the scene instead of the path.
//!
//! It is deliberately a scene where the froxels hold several lights
//! each: one light exercises the machinery and proves nothing about it.

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

/// Deliberately not a multiple of the 16-pixel tile, and deliberately
/// even — odd would additionally exercise the half-rate quad that hangs
/// off the right edge, which is worth its own case rather than being
/// mixed into every assertion.
///
/// The last column and row of workgroups run threads past the edge of
/// the screen, which is the case a compute pass has and a fullscreen
/// triangle does not — and those threads still have to reach every
/// barrier their neighbours are waiting on.
pub const SIZE: u32 = 200;

pub struct Rig {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub resources: Resources,
    pub stage: MeshletRenderStage,
    pub camera: ViewCamera,
}

/// A floor, `lights x lights` point lights above it, and a camera
/// looking down the length of it.
///
/// `wall` adds a second box standing on the floor. Its silhouette is
/// where a tile spans many z-slices at once — the case the workgroup
/// cache has a cap for, and the one half-rate shading has to
/// reconstruct across.
pub fn rig(lights: u32, wall: bool) -> Option<Rig> {
    build(lights, wall, false)
}

/// The same scene plus **one blue shadow-casting light**, and the shadow
/// atlas switched on so it is given a slot.
///
/// # Why it is as bright as its neighbours, and blue
///
/// The obvious way to test "a caster is never sampled" is to make it so
/// dim that importance sampling would never choose it. That does not
/// work, and the reason is worth writing down: **the weight and the
/// contribution are the same quantity**. A light dim enough to reliably
/// lose the race is dim enough to be invisible in the result — measured
/// at a hundredth of its neighbours, its blue tint came out at 0.0002 of
/// a channel and the test could not tell the rule from its absence.
///
/// So the discrimination is spatial instead. At one sample a tile picks
/// one light of seventeen, so without the rule roughly fifteen blocks in
/// sixteen would contain no blue at all — the caster's contribution
/// would survive in the *average* and disappear from most of the
/// *picture*. Counting blocks sees that; counting photons does not.
pub fn rig_with_caster(lights: u32) -> Option<Rig> {
    build(lights, true, true)
}

fn build(lights: u32, wall: bool, caster: bool) -> Option<Rig> {
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

    let mut materials = MaterialPipeline::with_capacity(&device, &queue, 4);
    let material_guid = Guid::new_v4();
    materials.register(
        &queue,
        material_guid,
        &Material::new([0.8, 0.8, 0.8, 1.0], 0.1, 0.4, 0.0),
    );
    resources.insert(materials);

    // 🔴 `Vbuf64Support` defaults to *unsupported*, so a config built
    // with `..Default::default()` puts the stage on the R32 fallback
    // however capable the device is — and the callers would then compare
    // the fragment path against itself. Detect from the device.
    let mut stage = MeshletRenderStage::new(
        &device,
        MeshletRenderStageConfig {
            size: (SIZE, SIZE),
            instance_capacity: 8,
            meshlet_capacity: 1024,
            vbuf64: Vbuf64Support::detect(&device),
            // The R64 path asserts the density accumulator exists; the
            // caps default says the device cannot have one.
            debug_caps: MeshletDebugCaps::detect(&device),
        },
    );
    let mesh_guid = Guid::new_v4();
    stage.ensure_gpu_mesh(&device, mesh_guid, &meshlet_mesh);

    let mut commands = Commands::new();
    let mut box_at = |resources: &mut Resources, matrix: Mat4| {
        commands
            .spawn(resources)
            .insert(MeshRenderer {
                mesh: Some(mesh_guid),
                material: Some(material_guid),
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
    );
    if wall {
        box_at(
            &mut resources,
            Mat4::from_translation(Vec3::new(0.0, 1.5, -2.0))
                * Mat4::from_scale(Vec3::new(6.0, 3.0, 0.4)),
        );
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
    })
}

pub fn render(rig: &mut Rig, compute: bool) -> Vec<u8> {
    render_at(rig, compute, ShadingRate::Full)
}

/// Renders one frame on the chosen path and rate, and reads the colour
/// target back.
///
/// 🔴 Both switches report how many views they reached, and both are
/// asserted. Zero means no view has the R64 stage, so neither argument
/// changes anything and every assertion downstream would compare the
/// fragment path with itself — passing, forever, with the compute
/// shader deleted. That is not a hypothesis: it is what
/// `compute_shading_parity` did until this assertion existed.
///
/// The order matters. `set_compute_shading(false)` drops the rate back
/// to full, because the fragment path has no reduced rate, so the rate
/// has to be set after the path and not before.
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
