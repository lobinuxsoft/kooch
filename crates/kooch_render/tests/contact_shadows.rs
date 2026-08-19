//! #735's acceptance: an object standing on a floor is grounded to it.
//!
//! The cascades are **off** in every render below. That is the whole
//! design of this file: with them on, a darker pixel beside the cube
//! proves nothing, because the cascade would darken it too. With them
//! off, the only thing in the engine that can darken the floor where the
//! cube meets it is the screen-space march.
//!
//! Every assertion compares the **same pixel** across two renders that
//! differ by one flag, for the reason `csm_shadows.rs` states: two
//! places in one image differ for a dozen legitimate reasons.
//!
//! Run with:
//!   cargo test -p kooch_render --test contact_shadows

mod common;

/// 🔴 Serialises this binary's cases, and it closes a long-standing
/// flake rather than adding caution.
///
/// `common` hands every case the SAME device — one per binary, by
/// `OnceLock`, to dodge the radv `request_adapter` race of #334 — so
/// seven cases at once means seven threads recording and submitting
/// against one device. Under radv that segfaults the PROCESS instead of
/// failing a case, intermittently, while passing every time under
/// `--test-threads=1`. This file and `gpu_scopes` are the two that were
/// known to "fail sometimes in `cargo test --workspace` and always pass
/// in isolation"; that is what this was.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|e| e.into_inner())
}

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
use kooch_render::meshlet::{
    MeshletDebugCaps, MeshletDebugMode, MeshletRenderStage, MeshletRenderStageConfig,
    build_default_meshlets,
};
use kooch_render::shadow::ShadowSettings;
use kooch_render::vbuf64::Vbuf64Support;

const SIZE: u32 = 256;

/// Where the sun shines, normalised on use. Tilted so the contact
/// shadow lands beside the cube rather than under it, where the cube
/// would hide it from the camera.
const SUN: Vec3 = Vec3::new(0.5, -1.0, 0.0);

/// Centre of the cube. It is a unit cube, so this puts its underside
/// **on** the floor — the case the whole technique exists for.
const CUBE_CENTRE: Vec3 = Vec3::new(0.0, 0.5, 0.0);

/// Floor point 10 cm from the cube's edge, on the side the sun's rays
/// travel toward. Inside the default 0.3 m ray length; the cube's side
/// face is the occluder the march has to find.
const CONTACT_POINT: Vec3 = Vec3::new(0.6, 0.0, 0.0);

/// Floor well clear of the cube. Nothing within the ray's reach, so its
/// luminance must not move.
const OPEN_FLOOR: Vec3 = Vec3::new(-5.0, 0.0, 2.0);

/// A device carrying the int64-atomic bundle the R64 path needs, or
/// `None` where the adapter has none.
///
/// 🔴 `try_acquire_device` asks for `Features::empty()`, so the shared
/// test device **cannot** take the R64 path — every render in this file
/// would go through the R32 compute deferred and the fragment path
/// would ship untested. Half of #476 went into two paths diverging with
/// no compiler between them; this is the second device that stops it
/// happening again here.
fn try_acquire_device_vbuf64() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok()?;

    // 🔴 SHADER_F16 is here because building a `Vbuf64Stage` builds
    // FSR 3.1's accumulation, which is compiled with `enable f16`. The
    // list is a copy of the engine's, and it is the fifth in the tree —
    // a device short of one of them fails inside a shader this test has
    // no interest in.
    let needed = wgpu::Features::SHADER_F16
        | wgpu::Features::FLOAT32_FILTERABLE
        | wgpu::Features::TEXTURE_ATOMIC
        | wgpu::Features::TEXTURE_INT64_ATOMIC
        | wgpu::Features::SHADER_INT64
        | wgpu::Features::SHADER_INT64_ATOMIC_MIN_MAX
        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
    if !adapter.features().contains(needed) {
        return None;
    }

    let mut limits = wgpu::Limits::default();
    limits.max_storage_textures_per_shader_stage =
        16.min(adapter.limits().max_storage_textures_per_shader_stage);
    limits.max_bind_groups = 6.min(adapter.limits().max_bind_groups);
    limits.max_storage_buffers_per_shader_stage =
        16.min(adapter.limits().max_storage_buffers_per_shader_stage);

    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("contact_shadows_vbuf64_test_device"),
        required_features: needed,
        required_limits: limits,
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()
}

/// Which shading path a render goes through. Both march; the march is
/// the same text, bound at different indices.
#[derive(Clone, Copy, PartialEq)]
enum Path {
    /// R32Uint visibility buffer, compute deferred.
    ComputeDeferred,
    /// Atomic R64 visibility buffer, two-pass fragment shading.
    TwoPassFragment,
}

struct Rig {
    device: wgpu::Device,
    queue: wgpu::Queue,
    resources: Resources,
    stage: MeshletRenderStage,
    camera: ViewCamera,
}

/// A cube standing on a wide flat floor, one sun, **cascades off**.
fn rig(path: Path) -> Option<Rig> {
    let (device, queue) = match path {
        Path::ComputeDeferred => try_acquire_device()?,
        Path::TwoPassFragment => try_acquire_device_vbuf64()?,
    };

    let meshlet_mesh = build_default_meshlets(&build_cube_mesh()).expect("build meshlets");

    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());
    // 🔴 The point of the file. See the module docs.
    resources.insert(ShadowSettings {
        enabled: false,
        ..Default::default()
    });
    // Ambient present but low: with none, anything the march darkens
    // goes to black and "darker" would pass for a march that shadowed
    // the entire floor as readily as for a correct one.
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
            vbuf64: Vbuf64Support::from_supported(path == Path::TwoPassFragment),
            // The R64 path dereferences the density accumulator
            // unconditionally, so the two flags travel together.
            debug_caps: MeshletDebugCaps::from_flags(path == Path::TwoPassFragment),
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
    // The floor: the same cube, flattened, top face at y = 0.
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
        // Close and low: a contact shadow is centimetres wide, and from
        // far away it is a pixel that averages away.
        camera: ViewCamera::looking_at(Vec3::new(1.2, 1.4, 3.0), Vec3::new(0.3, 0.1, 0.0)),
    })
}

fn add_sun(resources: &mut Resources, contact_shadows: bool) {
    let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, SUN.normalize());
    let mut commands = Commands::new();
    commands
        .spawn(resources)
        .insert(DirectionalLight {
            active: true,
            color: Vec3::ONE,
            intensity: 20_000.0,
            cast_shadows: false,
            contact_shadows,
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

/// World point → pixel, through the matrix the stage rendered with, so
/// moving the camera moves the sample rather than silently pointing it
/// at the background.
fn project(camera: &ViewCamera, world: Vec3) -> (u32, u32) {
    let clip = camera.view_proj(1.0) * world.extend(1.0);
    let ndc = clip.truncate() / clip.w;
    let x = ((ndc.x * 0.5 + 0.5) * SIZE as f32).clamp(0.0, (SIZE - 1) as f32);
    let y = ((0.5 - ndc.y * 0.5) * SIZE as f32).clamp(0.0, (SIZE - 1) as f32);
    (x as u32, y as u32)
}

fn luminance(pixels: &[u8], camera: &ViewCamera, world: Vec3) -> f32 {
    let (x, y) = project(camera, world);
    luminance_at(pixels, SIZE, x, y, 1)
}

/// Renders the scene twice, differing only in the light's
/// `contact_shadows` flag.
fn with_and_without(path: Path) -> Option<(Vec<u8>, Vec<u8>, ViewCamera)> {
    let mut off = rig(path)?;
    add_sun(&mut off.resources, false);
    let without = render(&mut off);

    let mut on = rig(path).expect("device acquired once already");
    add_sun(&mut on.resources, true);
    let with = render(&mut on);

    Some((with, without, off.camera))
}

/// 🔴 The assertion the issue exists to make true, with **no shadow map
/// in the scene at all**.
#[test]
fn a_cube_standing_on_a_floor_darkens_the_floor_it_touches() {
    let _gpu = gpu_lock();
    let Some((with, without, camera)) = with_and_without(Path::ComputeDeferred) else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let lit = luminance(&without, &camera, CONTACT_POINT);
    let marched = luminance(&with, &camera, CONTACT_POINT);

    assert!(
        lit > 0.05,
        "the floor beside the cube is already dark without a march \
         ({lit:.4} linear) — this test would pass on a scene that never \
         rendered",
    );
    assert!(
        marched < lit * 0.9,
        "the floor 10 cm from the cube is {marched:.4} with contact \
         shadows and {lit:.4} without (linear) — the march found no \
         occluder where a cube is standing",
    );
}

/// The other half, and the one that catches a march that returns
/// occlusion unconditionally: floor with nothing within reach must not
/// move. Without this, a `return 0.0` passes the test above.
#[test]
fn floor_with_nothing_near_it_is_unchanged() {
    let _gpu = gpu_lock();
    let Some((with, without, camera)) = with_and_without(Path::ComputeDeferred) else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let lit = luminance(&without, &camera, OPEN_FLOOR);
    let marched = luminance(&with, &camera, OPEN_FLOOR);

    assert!(
        lit > 0.05,
        "the sample point is not on lit floor ({lit:.4} linear)",
    );
    assert!(
        (marched - lit).abs() < lit * 0.1,
        "open floor went from {lit:.4} to {marched:.4} (linear) when the \
         march was enabled — a ray that finds an occluder in empty space \
         is a bias or a thickness that swallows the scene",
    );
}

/// Zero steps is the off switch, and it has to be the off switch even
/// for a light that opted in — otherwise a project that turns the
/// feature off in its settings still pays for it and still sees it.
#[test]
fn zero_steps_in_the_settings_overrides_the_light() {
    let _gpu = gpu_lock();
    let Some(mut opted_in) = rig(Path::ComputeDeferred) else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    add_sun(&mut opted_in.resources, true);
    let marched = render(&mut opted_in);

    let mut disabled = rig(Path::ComputeDeferred).expect("device acquired once already");
    disabled
        .resources
        .insert(kooch_render::ContactShadowSettings {
            linear_steps: 0,
            ..Default::default()
        });
    add_sun(&mut disabled.resources, true);
    let off = render(&mut disabled);

    let camera = opted_in.camera;
    let with_march = luminance(&marched, &camera, CONTACT_POINT);
    let no_march = luminance(&off, &camera, CONTACT_POINT);

    assert!(
        with_march < no_march * 0.9,
        "zero steps rendered {no_march:.4} and sixteen rendered \
         {with_march:.4} (linear) — the settings are not reaching the \
         shader, so neither is any other number in them",
    );
}

/// 🔴 The same property on the **other** shading path.
///
/// The R64 route shades in a fragment shader with its own group 0 and
/// its own binding numbers; nothing but this test stands between the
/// two paths shipping different behaviour, and the compiler is not
/// going to notice.
#[test]
fn the_fragment_path_marches_too() {
    let _gpu = gpu_lock();
    let Some((with, without, camera)) = with_and_without(Path::TwoPassFragment) else {
        eprintln!("no adapter with the int64-atomic bundle; skipping");
        return;
    };

    let lit = luminance(&without, &camera, CONTACT_POINT);
    let marched = luminance(&with, &camera, CONTACT_POINT);

    assert!(
        lit > 0.05,
        "the floor beside the cube is already dark without a march \
         ({lit:.4} linear) on the R64 path",
    );
    assert!(
        marched < lit * 0.9,
        "on the R64 two-pass path the floor 10 cm from the cube is \
         {marched:.4} with contact shadows and {lit:.4} without (linear) \
         — the R32 path darkens it, so the two have diverged",
    );
}

/// And its guard: an unconditional occlusion passes the test above.
#[test]
fn the_fragment_path_leaves_open_floor_alone() {
    let _gpu = gpu_lock();
    let Some((with, without, camera)) = with_and_without(Path::TwoPassFragment) else {
        eprintln!("no adapter with the int64-atomic bundle; skipping");
        return;
    };

    let lit = luminance(&without, &camera, OPEN_FLOOR);
    let marched = luminance(&with, &camera, OPEN_FLOOR);

    assert!(
        lit > 0.05,
        "the sample point is not on lit floor ({lit:.4})"
    );
    assert!(
        (marched - lit).abs() < lit * 0.1,
        "open floor went from {lit:.4} to {marched:.4} (linear) on the \
         R64 path when the march was enabled",
    );
}

/// The debug view has to be *wired*, not merely selectable — a mode that
/// silently falls through to lit shading looks plausible and answers
/// nothing. Open floor must read as "marched, found nothing" and the
/// floor the cube stands on must not.
#[test]
fn the_debug_view_separates_a_hit_from_open_floor() {
    let _gpu = gpu_lock();
    let Some(mut rig) = rig(Path::ComputeDeferred) else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    add_sun(&mut rig.resources, true);
    rig.resources.insert(MeshletDebugMode::ContactShadows);
    let pixels = render(&mut rig);
    let camera = rig.camera;

    let (ox, oy) = project(&camera, OPEN_FLOOR);
    let (cx, cy) = project(&camera, CONTACT_POINT);
    let rgb = |x: u32, y: u32| {
        let i = ((y * SIZE + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2]]
    };
    let open = rgb(ox, oy);
    let contact = rgb(cx, cy);

    // Deliberately NOT asserting which colour open floor takes. At this
    // render size a 0.3 m ray is under two pixels long there, so the
    // honest answer is blue — "nothing to march" — and a test that
    // demanded grey would be pinning the author's guess rather than the
    // view's answer. What has to hold is that the view answers at all.
    let magenta = [255, 0, 255];
    assert_ne!(
        open, magenta,
        "open floor is magenta, which means no light in the scene opted \
         into the march — the flag is not reaching the shader",
    );
    assert_ne!(
        contact, open,
        "the floor beside the cube reads the same as open floor \
         ({contact:?}) — the view is painting one colour everywhere, \
         which is what a mode falling through to lit shading looks like",
    );
}

/// A second light, from a second direction, whose contact lands where
/// the sun's does not.
const SIDE: Vec3 = Vec3::new(0.0, -1.0, 0.5);

/// Floor point 10 cm from the cube on the side `SIDE`'s rays travel
/// toward, and visible from the camera — which the mirror of
/// `CONTACT_POINT` would not be, standing behind the cube.
const SIDE_CONTACT: Vec3 = Vec3::new(0.0, 0.0, 0.6);

fn add_light(resources: &mut Resources, direction: Vec3, intensity: f32) {
    let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, direction.normalize());
    let mut commands = Commands::new();
    commands
        .spawn(resources)
        .insert(DirectionalLight {
            active: true,
            color: Vec3::ONE,
            intensity,
            cast_shadows: false,
            contact_shadows: true,
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_quat(rotation),
        });
    commands.apply(resources);
}

/// Two lights, one march (#845).
///
/// 🔴 The assertion is on the DIM light's contact, not the bright one's.
/// Both lights reach both points, so a march that ran for both darkens
/// `SIDE_CONTACT`; a march that ran only for the strongest leaves it
/// lit. Asserting on the bright light's contact would pass either way.
#[test]
fn only_the_strongest_light_marches() {
    let _gpu = gpu_lock();
    let Some(mut dominant) = rig(Path::ComputeDeferred) else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    add_light(&mut dominant.resources, SUN, 3_000.0);
    add_light(&mut dominant.resources, SIDE, 1_200.0);
    let one_march = render(&mut dominant);

    let mut every = rig(Path::ComputeDeferred).expect("device acquired once already");
    every.resources.insert(kooch_render::ContactShadowSettings {
        dominant_only: false,
        ..Default::default()
    });
    add_light(&mut every.resources, SUN, 3_000.0);
    add_light(&mut every.resources, SIDE, 1_200.0);
    let every_march = render(&mut every);

    let camera = dominant.camera;
    let kept = luminance(&one_march, &camera, SIDE_CONTACT);
    let shadowed = luminance(&every_march, &camera, SIDE_CONTACT);
    assert!(
        shadowed < kept * 0.95,
        "the dim light's contact reads {shadowed:.4} when every light marches and \
         {kept:.4} when only the strongest does — if these agree, either both \
         marched or neither did",
    );

    // The control: the strongest light's own contact is there in both.
    // Without it this test passes just as well with the march removed
    // entirely, which is the same shape of mistake #841's control exists
    // to catch.
    let bright_one = luminance(&one_march, &camera, CONTACT_POINT);
    let bright_all = luminance(&every_march, &camera, CONTACT_POINT);
    assert!(
        (bright_one - bright_all).abs() < bright_all * 0.05,
        "the strongest light's contact moved ({bright_one:.4} vs {bright_all:.4}); \
         it is marched in both modes and must not",
    );
}
