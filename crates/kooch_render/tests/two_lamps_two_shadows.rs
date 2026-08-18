//! Do two casting lamps produce TWO shadows, or two copies of one?
//!
//! 🔴 This exists because three hypotheses were argued from the code and
//! all three were wrong. The owner's report is specific and measurable —
//! *"aparecen muchas sombras que son copias exactas de una sola, no
//! tienen consideración de las otras luces"* — and a claim that precise
//! does not need another reading of the dispatcher. It needs the one
//! measurement that separates "the cubes are wrong" from everything
//! else.
//!
//! The discriminator: an occluder between two lamps on OPPOSITE sides
//! throws two shadows in different directions. If every lamp renders
//! the same cube, lighting both covers no more floor than lighting one.
//!
//! Shadow is isolated from illumination by differencing each
//! configuration against itself without an occluder — two lamps are
//! brighter than one, and comparing lit frames would measure that
//! instead.
//!
//! Run with:
//!   cargo test -p kooch_render --test two_lamps_two_shadows

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

// 🔴 1536, not 512. The cube view tiles six faces into a 3x2 grid, so
// at 512 each face gets 170x256 px and a ball 27 px across — small
// enough that its own edge reads as a crescent. Two sessions were spent
// interpreting that crescent.
const SIZE: u32 = 1536;

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
    build_with(lights, occluder, compute, false)
}

/// `contact` turns the screen-space march on, which is what the owner's
/// lamps now have. It is the one shadow in the engine that reads the
/// CAMERA's depth buffer, so an occluder that leaves the screen stops
/// occluding — and #845 marches only the brightest light per pixel, so
/// two lamps never both get one.
fn build_with(
    lights: &[(Vec3, bool)],
    occluder: bool,
    compute: bool,
    contact: bool,
) -> Option<Rig> {
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
        // 🔴 NOT the default of 4. The whole question is what happens
        // past a handful of casting lamps, and a rig that budgets four
        // measures the budget rather than the defect — which is exactly
        // what the first run of the sweep below did.
        point_shadows: kooch_lighting::MAX_POINT_SHADOWS as u32,
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
    // A dense sphere for the caster, because the owner's is a ball and
    // a cube is six quads: a defect that drops SOME meshlets of an
    // object cannot show on geometry that has almost none.
    //
    // 🔴 And with a real LOD **chain**, not `build_default_meshlets` —
    // that one lives in `builder/single_lod.rs` and produces one level.
    // The LOD selector is the only thing in the cull that can drop
    // meshlets from the MIDDLE of an object, and against a single-level
    // mesh it cannot run at all. Five reproductions came out clean for
    // that reason and for no other.
    let ball_mesh = kooch_render::meshlet::build_meshlets_lod_chain(
        &common::build_sphere_mesh(32, 48),
        kooch_render::meshlet::DEFAULT_MAX_VERTICES,
        kooch_render::meshlet::DEFAULT_MAX_TRIANGLES,
        0.5,
        Default::default(),
    )
    .expect("build sphere");
    let ball = Guid::new_v4();
    stage.ensure_gpu_mesh(&device, ball, &ball_mesh);

    let mut commands = Commands::new();
    let mut spawn = |matrix: Mat4, which: Guid| {
        commands
            .spawn(&mut resources)
            .insert(MeshRenderer {
                mesh: Some(which),
                material: Some(material),
                visible: true,
                ..Default::default()
            })
            .insert(GlobalTransform { matrix });
    };
    spawn(
        Mat4::from_translation(Vec3::new(0.0, -0.25, 0.0))
            * Mat4::from_scale(Vec3::new(20.0, 0.5, 20.0)),
        mesh,
    );
    if occluder {
        spawn(
            Mat4::from_translation(Vec3::new(0.0, 0.5, 0.0)) * Mat4::from_scale(Vec3::splat(0.5)),
            ball,
        );
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
                contact_shadows: contact,
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

/// Serialised: `common` hands every case one device.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|e| e.into_inner())
}

const EYE: Vec3 = Vec3::new(0.0, 6.0, 9.0);
/// Opposite sides, so their shadows fall in opposite directions.
const LEFT: Vec3 = Vec3::new(-4.0, 3.5, 0.0);
const RIGHT: Vec3 = Vec3::new(4.0, 3.5, 0.0);

/// Renders `lights` with and without the occluder and returns, per
/// pixel, how much the occluder darkened it.
fn shadow_mask(lights: &[(Vec3, bool)]) -> Option<Vec<f32>> {
    let render = |occluder: bool| -> Option<Vec<u8>> {
        let mut rig = build(lights, occluder, true)?;
        let camera = ViewCamera::looking_at(EYE, Vec3::new(0.0, 0.5, 0.0));
        rig.stage
            .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &camera, 1.0);
        Some(read_rgba8(
            &rig.device,
            &rig.queue,
            rig.stage.color_texture(),
        ))
    };
    let lit = render(false)?;
    let shadowed = render(true)?;
    Some(
        lit.chunks_exact(4)
            .zip(shadowed.chunks_exact(4))
            .map(|(l, s)| (l[0] as f32 - s[0] as f32).max(0.0) / 255.0)
            .collect(),
    )
}

/// How many pixels the occluder darkened by more than a threshold that
/// ignores shading noise.
fn shadowed_pixels(mask: &[f32]) -> usize {
    mask.iter().filter(|d| **d > 0.05).count()
}

/// 🔴 Two lamps on opposite sides must shadow more floor than one.
///
/// If every casting lamp ends up rendering the same cube — the reported
/// "copias exactas de una sola" — then the second lamp contributes no
/// shadow of its own and this count barely moves.
#[test]
fn two_lamps_shadow_more_than_one() {
    let _gpu = gpu_lock();
    let Some(left_only) = shadow_mask(&[(LEFT, true), (RIGHT, false)]) else {
        eprintln!("no adapter; skipping");
        return;
    };
    let Some(right_only) = shadow_mask(&[(LEFT, false), (RIGHT, true)]).filter(|_| true) else {
        return;
    };
    let Some(both) = shadow_mask(&[(LEFT, true), (RIGHT, true)]) else {
        return;
    };

    let (l, r, b) = (
        shadowed_pixels(&left_only),
        shadowed_pixels(&right_only),
        shadowed_pixels(&both),
    );
    eprintln!("shadowed pixels — left {l}, right {r}, both {b}");

    assert!(
        l > 200 && r > 200,
        "one lamp alone shadows almost nothing (left {l}, right {r}); the rig cannot \
         tell a shared cube from a correct one",
    );
    assert!(
        b > l && b > r,
        "two lamps on opposite sides shadow {b} pixels while the left alone shadows {l} \
         and the right alone {r}. The second lamp is contributing no shadow of its own, \
         which is what every casting light rendering the SAME cube looks like from the \
         outside.",
    );
}

/// And the two single-lamp shadows must land in different places.
///
/// The guard for the test above: if both lamps happened to throw their
/// shadow onto the same pixels, the count could rise for the wrong
/// reason and the assertion would pass while the defect stands.
#[test]
fn the_two_shadows_do_not_coincide() {
    let _gpu = gpu_lock();
    let Some(left_only) = shadow_mask(&[(LEFT, true), (RIGHT, false)]) else {
        eprintln!("no adapter; skipping");
        return;
    };
    let Some(right_only) = shadow_mask(&[(LEFT, false), (RIGHT, true)]) else {
        return;
    };

    let overlap = left_only
        .iter()
        .zip(&right_only)
        .filter(|(a, b)| **a > 0.05 && **b > 0.05)
        .count();
    let l = shadowed_pixels(&left_only);
    eprintln!("left {l}, overlapping the right lamp's shadow {overlap}");
    assert!(
        (overlap as f32) < l as f32 * 0.6,
        "the two lamps' shadows overlap on {overlap} of the left lamp's {l} pixels. They \
         are supposed to fall in opposite directions, so either the lamps are not where \
         this test thinks they are — or both are already rendering the same cube.",
    );
}

/// 🔴 The sweep that answers "at how many lamps does it break".
///
/// Two lamps are correct — the assertions above say so — and the owner
/// sees copies at thirty-two. Something between those two numbers stops
/// working, and which number it is names the cause: a ring, a slot
/// array, a cube-array capacity or a per-frame dispatch bound all fail
/// at a specific count, and none of them fail gradually.
///
/// Each round adds one lamp on a circle and asks whether the newest one
/// contributes shadow the others do not. The first N where it stops is
/// the answer.
#[test]
#[ignore = "diagnostic sweep; run explicitly"]
fn find_where_the_cubes_start_repeating() {
    let _gpu = gpu_lock();
    let ring = |n: usize| -> Vec<(Vec3, bool)> {
        (0..n)
            .map(|i| {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                (Vec3::new(a.cos() * 4.0, 3.5, a.sin() * 4.0), true)
            })
            .collect()
    };

    for n in [2usize, 3, 4, 6, 8, 12, 16, 24, 32] {
        let all = ring(n);
        let mut minus_one = all.clone();
        minus_one[n - 1].1 = false;

        let Some(with) = shadow_mask(&all) else {
            return;
        };
        let Some(without) = shadow_mask(&minus_one) else {
            return;
        };
        let (a, b) = (shadowed_pixels(&with), shadowed_pixels(&without));
        let gained = a as i64 - b as i64;
        eprintln!("n={n:>2}  all {a:>6}  minus-last {b:>6}  the last lamp adds {gained:>7}");
    }
}
