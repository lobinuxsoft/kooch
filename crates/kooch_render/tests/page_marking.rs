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
use kooch_render::meshlet::DEFERRED_COLOR_FORMAT;
use kooch_render::projection::perspective_infinite_rh_reverse_z;
use kooch_render::shadow::pages::mark::{MarkCounts, PAINT_FORMAT, PageMarker, Paint};
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

/// A radiance target for the debug view to paint into.
fn paint_target(device: &wgpu::Device) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("page_marking_color"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEFERRED_COLOR_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
        .create_view(&Default::default())
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
        /* rate */ 1,
        Paint {
            target: &paint_target(device),
            on: false,
            size: (SIZE, SIZE),
        },
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

#[test]
fn a_stopped_pass_reports_nothing() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);

    let eye = Vec3::ZERO;
    let view = Mat4::look_at_rh(eye, Vec3::NEG_Z, Vec3::Y);
    let proj = projection();
    let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);
    let mut lights = GpuLights::new(&device);
    let mut frame = kooch_lighting::LightFrame::extract(&resources);
    lights.update(&device, &queue, &resources, camera, None, &mut frame);
    let depth_view = depth_texture(&device, &queue, 0.01);
    let mut marker = PageMarker::new(&device, PageConfig::default(), ClipmapConfig::default());

    let mut encoder = device.create_command_encoder(&Default::default());
    lights.record_clusters(&mut encoder);
    marker.record(
        &device,
        &queue,
        &mut encoder,
        &lights,
        &depth_view,
        (proj * view).inverse(),
        eye,
        None,
        (SIZE, SIZE),
        1,
        Paint {
            target: &paint_target(&device),
            on: false,
            size: (SIZE, SIZE),
        },
    );
    queue.submit([encoder.finish()]);
    marker.poll();
    wait(&device);
    marker.poll();
    assert!(marker.last().is_some_and(|c| c.resident > 0));

    // 🔴 The count is sticky on purpose — the ring runs a frame or two
    // behind, so a frame with nothing new keeps the last real answer.
    // That is right while the pass runs and wrong the moment it stops,
    // and forgetting it was what made turning the pass OFF log every
    // frame instead of none.
    marker.forget();
    assert_eq!(marker.last(), None);
}

/// Reads the paint target back as `[r, g, b, a]` per pixel, 0..1.
///
/// `Rgba8Unorm`, so four bytes a texel and the row pitch has to be
/// padded to wgpu's 256-byte alignment like any other copy.
fn read_paint(device: &wgpu::Device, queue: &wgpu::Queue, view: &wgpu::Texture) -> Vec<[f32; 4]> {
    let row = SIZE as u64 * 4;
    let padded = row.div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("page_marking_paint_readback"),
        size: padded * SIZE as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: view,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let (tx, rx) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    wait(device);
    rx.recv().unwrap().unwrap();

    let mapped = buffer.slice(..).get_mapped_range();
    let mut out = Vec::with_capacity((SIZE * SIZE) as usize);
    for y in 0..SIZE as usize {
        let start = y * padded as usize;
        let row = &mapped[start..start + (SIZE as usize * 4)];
        for x in 0..SIZE as usize {
            let texel = &row[x * 4..x * 4 + 4];
            out.push([
                texel[0] as f32 / 255.0,
                texel[1] as f32 / 255.0,
                texel[2] as f32 / 255.0,
                texel[3] as f32 / 255.0,
            ]);
        }
    }
    drop(mapped);
    buffer.unmap();
    out
}

/// One painted run, returning the target's contents.
fn paint(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    resources: &Resources,
    depth: f32,
) -> Vec<[f32; 4]> {
    let eye = Vec3::ZERO;
    let view = Mat4::look_at_rh(eye, Vec3::NEG_Z, Vec3::Y);
    let proj = projection();
    let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);
    let mut lights = GpuLights::new(device);
    let mut frame = kooch_lighting::LightFrame::extract(resources);
    lights.update(device, queue, resources, camera, None, &mut frame);

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("page_marking_color"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: PAINT_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&Default::default());
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
        None,
        (SIZE, SIZE),
        1,
        Paint {
            target: &target_view,
            on: true,
            size: (SIZE, SIZE),
        },
    );
    queue.submit([encoder.finish()]);
    wait(device);
    read_paint(device, queue, &target)
}

#[test]
fn the_view_paints_where_there_is_a_surface() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);

    let painted = paint(&device, &queue, &resources, 0.01);
    let lit = painted.iter().filter(|p| p[0] + p[1] + p[2] > 0.0).count();
    assert!(
        lit > painted.len() / 2,
        "{lit} of {} pixels painted",
        painted.len()
    );
    // 🔴 The failure this pins is not "the pass ran" but "anything
    // reached the screen": the palette has to survive whatever the
    // target does to it.
    let brightest = painted
        .iter()
        .fold(0.0f32, |acc, p| acc.max(p[0].max(p[1]).max(p[2])));
    assert!(brightest > 0.2, "brightest channel was {brightest}");
}

#[test]
fn the_view_leaves_the_sky_alone() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);

    // A cleared reversed-Z buffer is entirely sky, and painting over it
    // would erase the frame wherever the scene shows nothing.
    let painted = paint(&device, &queue, &resources, 0.0);
    assert!(
        painted.iter().all(|p| p[0] + p[1] + p[2] == 0.0),
        "the sky was painted over"
    );
}

#[test]
fn the_paint_format_is_the_views_own() {
    // 🔴 The bug this pins cost a frame's worth of validation errors per
    // second: the pass declared `Rgba16Float` because the radiance
    // target is HDR, and was handed `MeshletView::color_view`, which is
    // the TONEMAPPED target and `Rgba8Unorm`. wgpu compares the storage
    // class in the shader against the bind group layout, so the mismatch
    // surfaces as "Storage texture binding 8 expects format ..." on
    // every frame rather than as a wrong image — and no test caught it,
    // because the tests built their own target from the pass's own
    // constant instead of from the engine's.
    assert_eq!(PAINT_FORMAT, DEFERRED_COLOR_FORMAT);
}

#[test]
fn a_count_carries_its_resolution() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);
    // 🔴 A page count without the resolution it was taken at is not a
    // reading. The editor renders TWO views at two sizes, so the same
    // panel shows two different numbers a frame apart — and this project
    // has already had to retract a table that mixed 1080p with 720p.
    let counts = run(&device, &queue, &resources, 0.01, None);
    assert_eq!(counts.size, (SIZE, SIZE));
}
