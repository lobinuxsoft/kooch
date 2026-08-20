//! The GPU marking pass, against a real device (#866).
//!
//! What these assert is the half a CPU census cannot: that the shader
//! **compiles** — `PageMarker::new` builds the pipeline, and a WGSL
//! mistake surfaces here rather than three layers away as a frame that
//! renders nothing — and that the pass reads depth the way the engine
//! writes it.
//!
//! 🔴 They deliberately do **not** assert the census's numbers. The
//! census marks per froxel cell and this marks per pixel; they are meant
//! to be close, and pinning them to each other in a unit test would
//! either be flaky or would freeze one of them as the other's
//! definition. That comparison belongs in the instrument, where a
//! disagreement is a finding rather than a red build.

use glam::{Mat4, Vec3};

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::directional_light::DirectionalLight;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::point_light::PointLight;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::spot_light::SpotLight;
use kooch_lighting::{ClusterCamera, GpuLights};
use kooch_render::projection::perspective_infinite_rh_reverse_z;
use kooch_render::shadow::pages::mark::{MarkCounts, PageMarker};
use kooch_render::shadow::{ClipmapConfig, PageConfig, SHADOW_DEPTH_FORMAT};

const SIZE: u32 = 128;
const VIEWPORT: glam::Vec2 = glam::Vec2::new(SIZE as f32, SIZE as f32);

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
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
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("page_marking_test_device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()
}

fn world() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r.insert(Commands::new());
    let registry = r.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<GlobalTransform>();
    registry.register_cpu_reflected::<DirectionalLight>();
    registry.register_cpu_reflected::<PointLight>();
    registry.register_cpu_reflected::<SpotLight>();
    r
}

fn add_point(resources: &mut Resources, position: Vec3, range: f32) {
    let mut commands = resources.remove::<Commands>().unwrap();
    let entity = commands.spawn(resources).id();
    commands.apply(resources);
    commands
        .entity(entity)
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(position),
        })
        .insert(PointLight {
            active: true,
            color: Vec3::ONE,
            intensity: 100_000.0,
            range,
            ..Default::default()
        });
    commands.apply(resources);
    resources.insert(commands);
}

fn projection() -> Mat4 {
    perspective_infinite_rh_reverse_z(std::f32::consts::FRAC_PI_3, 1.0, 0.1)
}

/// A depth texture every texel of which holds `depth`.
///
/// 🔴 Reversed-Z infinite (ADR 0002): 0 is FAR, so a cleared buffer is
/// sky and a *larger* value is *nearer*. `0.01` puts the surface ten
/// metres out, which is inside the light ranges these tests use.
fn depth_texture(device: &wgpu::Device, queue: &wgpu::Queue, depth: f32) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("page_marking_depth"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SHADOW_DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder
        .begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("page_marking_depth_clear"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(depth),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        })
        .forget_lifetime();
    queue.submit([encoder.finish()]);
    view
}

fn wait(device: &wgpu::Device) {
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
}

/// One run of the pass, returning what came back.
fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    resources: &Resources,
    depth: f32,
    sun: Option<Vec3>,
) -> MarkCounts {
    let eye = Vec3::ZERO;
    let view = Mat4::look_at_rh(eye, Vec3::NEG_Z, Vec3::Y);
    let proj = projection();
    let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);

    let mut lights = GpuLights::new(device);
    let mut frame = kooch_lighting::LightFrame::extract(resources);
    lights.update(device, queue, resources, camera, None, &mut frame);

    let depth_view = depth_texture(device, queue, depth);
    let mut marker = PageMarker::new(device, PageConfig::default(), ClipmapConfig::default());

    let mut encoder = device.create_command_encoder(&Default::default());
    lights.record_clusters(&mut encoder);
    marker.record(
        device,
        queue,
        &mut encoder,
        &lights,
        &depth_view,
        (proj * view).inverse(),
        eye,
        sun,
        (SIZE, SIZE),
    );
    queue.submit([encoder.finish()]);
    // The ring is asynchronous on purpose, so a test has to drive both
    // halves: `poll` maps what was just submitted, the wait lets wgpu
    // run the callback, and the second `poll` picks it up. In a frame
    // the answer simply arrives one or two frames later.
    marker.poll();
    wait(device);
    marker.poll();
    marker.last().expect("the counters came back")
}

#[test]
fn the_shader_compiles() {
    let Some((device, _queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    // The pipeline is built here, so a WGSL mistake fails this test
    // rather than a frame.
    let _ = PageMarker::new(&device, PageConfig::default(), ClipmapConfig::default());
}

#[test]
fn sky_marks_nothing() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);
    // A cleared reversed-Z buffer is entirely sky.
    let counts = run(&device, &queue, &resources, 0.0, None);
    assert_eq!(counts.samples, 0, "no sample landed on a surface");
    assert_eq!(counts.resident, 0);
    assert_eq!(counts.overflow, 0);
}

#[test]
fn a_surface_marks_pages() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);
    let counts = run(&device, &queue, &resources, 0.01, None);
    assert_eq!(counts.samples, SIZE * SIZE, "every pixel is a surface");
    assert!(counts.pairs > 0, "the light reaches those samples");
    assert!(counts.resident > 0, "and they need pages");
    assert_eq!(counts.overflow, 0, "no page index past the buffer");
    // A screen's worth of surface cannot need a screen's worth of pages.
    assert!(
        counts.resident < counts.samples,
        "resident {} of {} samples",
        counts.resident,
        counts.samples
    );
}

#[test]
fn a_sun_marks_without_a_grid() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    // No local light at all: whatever is marked is the clipmap's, which
    // is the case the froxel grid cannot answer because a directional
    // light has no position to cluster.
    let resources = world();
    let counts = run(
        &device,
        &queue,
        &resources,
        0.01,
        Some(Vec3::new(-0.3, -1.0, -0.2)),
    );
    assert_eq!(counts.pairs, 0, "no local light was walked");
    assert!(counts.resident > 0, "the sun still needs pages");
    assert_eq!(counts.overflow, 0);
}
