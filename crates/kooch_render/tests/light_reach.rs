//! #835's acceptance: a light out of reach costs nothing and changes
//! nothing.
//!
//! `inti_light_contribution` returns before both BRDF layers, the shadow
//! cube and the contact march when the light's irradiance at this
//! fragment is zero. The saving is real — the froxel hands the shading
//! loop every light whose sphere touches the cell's AABB, and roughly 26
//! of the ~40 in the busiest cell reach no part of a given pixel (#820) —
//! but a saving is only worth having if the image is untouched, and that
//! is what this file pins.
//!
//! 🔴 **Clustering is off in every test here, on purpose.** With the grid
//! on, a light that reaches nothing might also have been dropped by
//! `cluster_raster.wgsl` before the shading loop ever saw it, and a test
//! that cannot tell those two apart passes whether or not the early-out
//! exists. Unclustered, the shader walks every light in the scene
//! (`inti_pbr.wgsl` — `if (inti.clustered == 0u)`), so the cut under test
//! is the only thing that can discard one.
//!
//! `a_reachable_light_changes_pixels` is not a nicety: without it, the
//! first test would also pass if the light were inactive, mispositioned,
//! or never uploaded. It is what proves the rig would have noticed.
//!
//! Run with:
//!   cargo test -p kooch_render --test light_reach

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

const SIZE: u32 = 128;

/// Lights the floor and is present in every render, so the images being
/// compared are of a lit scene rather than of the same black.
const KEY_POSITION: Vec3 = Vec3::new(0.0, 3.0, 4.0);

/// Directly above the floor's centre. The floor's top face is `y = 0`, so
/// the closest lit point is exactly [`SECOND_HEIGHT`] metres away and
/// every other one is further.
const SECOND_POSITION: Vec3 = Vec3::new(0.0, 3.0, 0.0);
const SECOND_HEIGHT: f32 = 3.0;

/// A floor, one key light, and optionally a second light whose range is
/// the variable under test. `None` leaves it out of the scene entirely.
fn render(second_range: Option<f32>) -> Option<Vec<u8>> {
    let (device, queue) = try_acquire_device()?;

    let meshlet_mesh = build_default_meshlets(&build_cube_mesh()).expect("build meshlets");

    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());
    // No cascades: an occluder-free scene has nothing to shadow, and a
    // shadow term would be a second reason for a pixel to be dark.
    resources.insert(ShadowSettings {
        enabled: false,
        ..Default::default()
    });
    // See the module docs. The grid must not get a vote on which lights
    // reach the shading loop.
    resources.insert(kooch_lighting::ClusterSettings {
        enabled: false,
        ..Default::default()
    });

    let mut materials = MaterialPipeline::with_capacity(&device, &queue, 4);
    let material_guid = Guid::new_v4();
    materials.register(
        &queue,
        material_guid,
        &Material::new([0.8, 0.8, 0.8, 1.0], 0.0, 0.6, 0.0),
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
            intensity: 300_000.0,
            range: 40.0,
            radius: 0.0,
            cast_shadows: false,
            contact_shadows: false,
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(KEY_POSITION),
        });

    if let Some(range) = second_range {
        commands
            .spawn(&mut resources)
            .insert(PointLight {
                active: true,
                color: Vec3::ONE,
                intensity: 300_000.0,
                range,
                radius: 0.0,
                cast_shadows: false,
                // On, so the march this cut skips is one the scene asked
                // for rather than one nothing would have run anyway.
                contact_shadows: true,
            })
            .insert(GlobalTransform {
                matrix: Mat4::from_translation(SECOND_POSITION),
            });
    }
    commands.apply(&mut resources);

    let camera = ViewCamera::looking_at(Vec3::new(0.0, 2.5, 7.0), Vec3::ZERO);
    stage.render_with_assets_primary(&device, &queue, &resources, &camera, 1.0);
    Some(read_rgba8(&device, &queue, stage.color_texture()))
}

/// How many of the two images disagree, and by how much at their worst.
fn difference(a: &[u8], b: &[u8]) -> (usize, u8) {
    assert_eq!(a.len(), b.len(), "two renders of the same size");
    let mut differing = 0;
    let mut worst = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        let delta = x.abs_diff(*y);
        if delta != 0 {
            differing += 1;
            worst = worst.max(delta);
        }
    }
    (differing, worst)
}

/// The assertion the issue exists to make: adding a light that reaches
/// nothing is not merely cheap, it is invisible.
///
/// Byte-for-byte rather than within a tolerance. The early-out returns
/// the value the arithmetic below it would have reached — anything
/// multiplied by an irradiance of exactly zero — so a single differing
/// byte means the cut fired where the light still had something to give.
#[test]
fn an_unreachable_light_changes_nothing() {
    let Some(without) = render(None) else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let with = render(Some(SECOND_HEIGHT - 1.0)).expect("second render");

    let (differing, worst) = difference(&without, &with);
    assert_eq!(
        differing, 0,
        "a light whose range stops a metre short of the floor changed \
         {differing} bytes, worst by {worst}",
    );
}

/// The control. Same light, same position, a range that reaches — and
/// now the image has to move, or the test above was measuring a light
/// the renderer never had.
#[test]
fn a_reachable_light_changes_pixels() {
    let Some(without) = render(None) else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let with = render(Some(SECOND_HEIGHT * 4.0)).expect("second render");

    let (differing, _) = difference(&without, &with);
    assert!(
        differing > 0,
        "a light standing {SECOND_HEIGHT} m over the floor with four \
         times the range to spare left every byte untouched",
    );
}

/// `inti_distance_attenuation` windows with `saturate(1 - factor * factor)`,
/// which is zero **at** the range and not merely near it. A light whose
/// distance to the closest lit point equals its range therefore adds
/// nothing anywhere, and the cut is allowed to take it.
///
/// Worth its own test because the alternative windows in circulation —
/// an exponential falloff, or a `smoothstep` that eases out — are all
/// asymptotic. Under any of them this cut would darken the scene.
#[test]
fn range_is_an_exact_boundary() {
    let Some(without) = render(None) else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let with = render(Some(SECOND_HEIGHT)).expect("second render");

    let (differing, worst) = difference(&without, &with);
    assert_eq!(
        differing, 0,
        "a light reaching exactly as far as the floor changed {differing} \
         bytes, worst by {worst} — the falloff does not close at zero",
    );
}
