//! #776's acceptance: a light with a size leaves a highlight with a size.
//!
//! Every assertion compares two renders that differ **only** in
//! `PointLight::radius`, for the reason `csm_shadows.rs` states: two
//! places in one image differ for a dozen legitimate reasons, two
//! renders of the same place differ for one.
//!
//! 🔴 The two that matter are `radius_widens_the_highlight` and
//! `radius_does_not_add_energy`. They are the pair: widening alone is
//! achievable by simply making the light rougher, and that is the bug
//! the normalization factor exists to prevent. A port that lands the
//! representative point and forgets the energy term passes the first
//! and fails the second.
//!
//! Cascades are off — there is no occluder here, and a shadow term
//! would be a second reason for a pixel to be dark.
//!
//! Run with:
//!   cargo test -p kooch_render --test point_radius

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

/// Directly above the floor's centre, close enough that the inverse
/// square law leaves a bright, compact highlight rather than a wash.
const LIGHT_POSITION: Vec3 = Vec3::new(0.0, 3.0, 0.0);

/// Big next to the 3 m the light stands at: `a_prime` grows with
/// `radius / (2 * distance)`, so a bulb of a few centimetres is a
/// correction below the precision of an 8-bit readback.
const BIG_RADIUS: f32 = 1.5;

/// Above which a pixel counts as inside the highlight. Chosen off the
/// rendered range rather than tuned: the floor's diffuse response here
/// sits far below it, so what is counted is specular.
const HIGHLIGHT_LUMA: f32 = 0.35;

struct Rig {
    device: wgpu::Device,
    queue: wgpu::Queue,
    resources: Resources,
    stage: MeshletRenderStage,
    camera: ViewCamera,
}

/// A wide, smooth, near-metal floor lit by one point light.
///
/// Metallic and smooth on purpose: `radius` only ever moves the
/// specular layer, so a rough dielectric floor would render the feature
/// invisible and the test would pass by measuring nothing.
fn rig(radius: f32) -> Option<Rig> {
    let (device, queue) = try_acquire_device()?;

    let meshlet_mesh = build_default_meshlets(&build_cube_mesh()).expect("build meshlets");

    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());
    resources.insert(ShadowSettings {
        enabled: false,
        ..Default::default()
    });
    // Low but present: with no ambient at all a metal reads as black
    // and every luminance below would be measuring the same zero.
    resources.insert(kooch_lighting::AmbientLight {
        intensity: 50.0,
        ..Default::default()
    });

    let mut materials = MaterialPipeline::with_capacity(&device, &queue, 4);
    let material_guid = Guid::new_v4();
    materials.register(
        &queue,
        material_guid,
        &Material::new([0.9, 0.9, 0.9, 1.0], 0.9, 0.15, 0.0),
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
    commands
        .spawn(&mut resources)
        .insert(MeshRenderer {
            mesh: Some(mesh_guid),
            material: Some(material_guid),
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(Vec3::new(0.0, -0.25, 0.0))
                * Mat4::from_scale(Vec3::new(20.0, 0.5, 20.0)),
        });
    commands
        .spawn(&mut resources)
        .insert(PointLight {
            active: true,
            color: Vec3::ONE,
            intensity: 400_000.0,
            range: 30.0,
            radius,
            cast_shadows: false,
            contact_shadows: false,
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(LIGHT_POSITION),
        });
    commands.apply(&mut resources);

    Some(Rig {
        device,
        queue,
        resources,
        stage,
        // Low and off to one side, so the mirror direction lands on the
        // floor inside the frame rather than behind the camera.
        camera: ViewCamera::looking_at(Vec3::new(0.0, 2.2, 6.0), Vec3::new(0.0, 0.0, 0.0)),
    })
}

fn render(radius: f32) -> Option<Vec<u8>> {
    let mut rig = rig(radius)?;
    rig.stage
        .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &rig.camera, 1.0);
    Some(read_rgba8(
        &rig.device,
        &rig.queue,
        rig.stage.color_texture(),
    ))
}

/// How many pixels are inside the highlight, and how bright its
/// brightest pixel is.
fn highlight(pixels: &[u8]) -> (u32, f32) {
    let mut count = 0;
    let mut peak = 0.0f32;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let luma = luminance_at(pixels, SIZE, x, y, 0);
            if luma > HIGHLIGHT_LUMA {
                count += 1;
            }
            peak = peak.max(luma);
        }
    }
    (count, peak)
}

#[test]
fn radius_widens_the_highlight() {
    let Some(point) = render(0.0) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let sphere = render(BIG_RADIUS).expect("second device");

    let (point_area, _) = highlight(&point);
    let (sphere_area, _) = highlight(&sphere);

    assert!(
        point_area > 0,
        "the point light left no highlight to compare against — \
         the rig stopped measuring the thing it exists to measure"
    );
    assert!(
        sphere_area > point_area,
        "a light of radius {BIG_RADIUS} covered {sphere_area} px against \
         the point light's {point_area}: the representative point is not \
         reaching the specular layer"
    );
}

#[test]
fn radius_does_not_add_energy() {
    let Some(point) = render(0.0) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let sphere = render(BIG_RADIUS).expect("second device");

    let (_, point_peak) = highlight(&point);
    let (_, sphere_peak) = highlight(&sphere);

    // Spreading a fixed amount of light over more surface has to leave
    // the brightest point no brighter. Without the `a / a_prime`
    // normalization the widened lobe keeps its peak and the light reads
    // as having been turned up.
    assert!(
        sphere_peak <= point_peak + 1e-3,
        "widening the light brightened its peak ({point_peak} → {sphere_peak}): \
         the normalization factor or the solid-angle term is missing"
    );
}

#[test]
fn radius_leaves_the_diffuse_alone() {
    let Some(point) = render(0.0) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let sphere = render(BIG_RADIUS).expect("second device");

    // A corner of the floor, far enough from the mirror direction that
    // what reaches it is diffuse and ambient.
    //
    // ⚠️ Unlike the two above, this one also passes when the feature is
    // absent — it is a guard against `radius` leaking into the diffuse
    // layer, not evidence that anything was implemented.
    let (x, y) = (24, 24);
    let before = luminance_at(&point, SIZE, x, y, 3);
    let after = luminance_at(&sphere, SIZE, x, y, 3);
    assert!(
        (after - before).abs() < 0.01,
        "diffuse-lit floor moved {before} → {after} when only the light's \
         radius changed; radius is specular-only"
    );
}
