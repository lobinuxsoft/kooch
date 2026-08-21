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
use kooch_render::shadow::pages::pool::{PAGE_CELL, PoolConfig};
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
    run_pool(
        device,
        queue,
        resources,
        depth,
        sun,
        100,
        PoolConfig::default(),
    )
    .1
}

/// The same run, keeping the marker so the page table can be read back.
#[allow(clippy::too_many_arguments)]
fn run_pool(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    resources: &Resources,
    depth: f32,
    sun: Option<Vec3>,
    // Shadow texels per screen pixel, as a percentage. The one lever
    // that moves the page count without moving the camera.
    density: u32,
    pool: PoolConfig,
) -> (PageMarker, MarkCounts) {
    let eye = Vec3::ZERO;
    let view = Mat4::look_at_rh(eye, Vec3::NEG_Z, Vec3::Y);
    let proj = projection();
    let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);

    let mut lights = GpuLights::new(device);
    let mut frame = kooch_lighting::LightFrame::extract(resources);
    lights.update(device, queue, resources, camera, None, &mut frame);

    let depth_view = depth_texture(device, queue, depth);
    let mut marker = PageMarker::new(device, PageConfig::default(), ClipmapConfig::default());
    marker.set_pool(device, pool);

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
        /* view */ 0,
        /* rate */ 1,
        density,
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
    let counts = marker.last().expect("the counters came back");
    (marker, counts)
}

/// Copies a storage buffer back, as words.
fn read_words(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer) -> Vec<u32> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("table_readback"),
        size: buffer.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, buffer.size());
    queue.submit([encoder.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    wait(device);
    let words = bytemuck::cast_slice::<u8, u32>(&staging.slice(..).get_mapped_range()).to_vec();
    staging.unmap();
    words
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
        /* view */ 0,
        1,
        /* density */ 100,
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
        /* view */ 0,
        1,
        /* density */ 100,
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

#[test]
fn half_density_is_a_quarter_of_the_pages() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);

    let at = |density| {
        let eye = Vec3::ZERO;
        let view = Mat4::look_at_rh(eye, Vec3::NEG_Z, Vec3::Y);
        let proj = projection();
        let mut lights = GpuLights::new(&device);
        let mut frame = kooch_lighting::LightFrame::extract(&resources);
        lights.update(
            &device,
            &queue,
            &resources,
            ClusterCamera::new(eye, view, proj, VIEWPORT),
            None,
            &mut frame,
        );
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
            /* view */ 0,
            1,
            density,
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
        marker.last().expect("counters came back").resident
    };

    // 🔴 The lever, and the reason it is the ONE that moves: a coarser
    // texel is a level coarser in BOTH axes, so halving the density
    // quarters the pages. Not exact — a level is a power of two and the
    // cells round into it — so this asserts the direction and the
    // magnitude, not an identity.
    let full = at(100);
    let half = at(50);
    assert!(full > 0 && half > 0);
    let ratio = full as f32 / half as f32;
    assert!(
        (2.0..=6.0).contains(&ratio),
        "half density gave {half} against {full}, a ratio of {ratio}"
    );
}

#[test]
fn every_drawable_page_claims_a_slot() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    // 🔴 The sun ALONE, because it is the only thing the raster draws
    // and therefore the only thing that spends the pool. With a local
    // light in the scene the two counts are meant to differ — that is
    // `a_local_light_marks_but_does_not_claim`.
    let resources = world();
    let counts = run(
        &device,
        &queue,
        &resources,
        0.01,
        Some(Vec3::new(0.3, -1.0, 0.2)),
    );
    assert!(counts.resident > 0, "the frame needs pages");
    // Two mechanisms counting the same 0->1 transitions: the mark bit's
    // atomicOr and the allocator's atomicAdd. They agree or one of them
    // is broken.
    assert_eq!(
        counts.pool.claims, counts.resident,
        "one claim per distinct page"
    );
    assert_eq!(counts.pool.overflow, 0, "the pool held them");
    assert_eq!(counts.pool.probes, 0, "no insert ran out of probes");
}

/// A page nothing draws does not get a page nothing writes.
///
/// 🔴 The measured defect: on `many_lights` with two viewports, local
/// lights held **991 and 1004 of each camera's 1024 slots** and the sun
/// — the only consumer the raster has — was left 33 and 20 pages. The
/// pool reported itself 100 % full while producing almost no shadow.
///
/// Local pages are still MARKED, because what a hundred casting lights
/// would cost is the measurement this whole track is justified by. They
/// simply do not spend the pool until something rasterises them.
#[test]
fn a_local_light_marks_but_does_not_claim() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 40.0);

    let dark = run(&device, &queue, &resources, 0.02, None);
    assert!(dark.resident > 0, "the point light marked nothing");
    assert_eq!(
        dark.pool.claims, 0,
        "{} local pages took a slot the raster cannot fill",
        dark.pool.claims
    );
    assert_eq!(dark.pool.unspent(dark.resident), dark.resident);

    let sunny = run(
        &device,
        &queue,
        &resources,
        0.02,
        Some(Vec3::new(0.3, -1.0, 0.2)),
    );
    assert!(sunny.pool.claims > 0, "the sun claimed nothing");
    assert!(
        sunny.pool.claims < sunny.resident,
        "the local pages stopped being counted: {} claims of {} resident",
        sunny.pool.claims,
        sunny.resident
    );
    assert_eq!(sunny.pool.overflow, 0, "the sun overflowed a default pool");
}

#[test]
fn a_full_pool_reports_overflow() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    // The SUN fills it: it is the only thing that spends the pool. A
    // NEAR surface, because a far one is one coarse clipmap level and a
    // handful of pages — not enough to overflow even a four-page pool.
    let resources = world();
    let small = PoolConfig { pages: 4, views: 1 };
    let (_marker, counts) = run_pool(
        &device,
        &queue,
        &resources,
        0.6,
        Some(Vec3::new(0.3, -1.0, 0.2)),
        // Four times the screen's density, so the clipmap picks a level
        // fine enough for the frustum to cover more than a handful of
        // pages. Containment is a floor on the level and a far surface
        // pins it coarse whatever the density says.
        400,
        small,
    );
    assert!(
        counts.resident > small.slice(),
        "the frame asks for more than the pool holds: {} of {}",
        counts.resident,
        small.slice()
    );
    assert_eq!(counts.pool.allocated(), small.slice(), "the pool filled");
    // 🔴 `claims` counts the ones that GOT a slot now, not the ones that
    // asked: `page_touch` allocates before it inserts, so a request the
    // pool could not answer never reaches the table and never counts.
    // `overflow` is the rest, and the two together are the requests.
    assert_eq!(
        counts.pool.claims + counts.pool.reused,
        small.slice(),
        "the pool handed out every slot it had"
    );
    assert!(
        counts.pool.overflow > 0,
        "a pool of {} could not answer {} requests and said nothing",
        small.slice(),
        counts.resident
    );
}

#[test]
fn the_table_holds_every_claim() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);
    // 🔴 A sun, and the point light alongside it. Only the sun's pages
    // claim a slot, so a scene without one fills no table and this test
    // would pass by having nothing to check.
    let (marker, counts) = run_pool(
        &device,
        &queue,
        &resources,
        0.01,
        Some(Vec3::new(0.3, -1.0, 0.2)),
        100,
        PoolConfig::default(),
    );
    assert!(counts.pool.claims > 0, "the sun claimed nothing");
    let keys = read_words(&device, &queue, marker.pool().keys());
    let slots = read_words(&device, &queue, marker.pool().slots());
    let capacity = marker.pool().config().total();

    let mut seen_keys = std::collections::HashSet::new();
    let mut seen_slots = std::collections::HashSet::new();
    for (entry, &key) in keys.iter().enumerate() {
        // 0 is EMPTY and `PAGE_DEAD` is an evicted entry: neither names
        // a slot anyone owns.
        if key == 0 || key == 0xffff_fffe {
            continue;
        }
        assert!(seen_keys.insert(key), "key {key} filed twice");
        // TWO words an entry — slot, then age. See `PAGE_CELL`.
        let slot = slots[entry * PAGE_CELL as usize];
        assert!(slot < capacity, "slot {slot} past the pool");
        assert!(seen_slots.insert(slot), "slot {slot} handed out twice");
    }
    assert_eq!(
        seen_keys.len() as u32,
        counts.pool.allocated(),
        "the table holds exactly what was allocated"
    );
}

/// Two cameras, one table.
///
/// 🔴 The defect this whole change exists for. `PageMarker` lives on the
/// stage, so both viewports marked into the same table — and the second
/// one to run had just emptied it with a `clear_buffer`. What the user
/// saw was shadows in one viewport and none in the other.
///
/// So: mark for camera 0, mark for camera 1, and camera 0's entries have
/// to still be there. The camera lives in the high part of the page id
/// and the reset is a pass that reads it.
#[test]
fn a_view_clears_only_its_own_pages() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 40.0);

    let eye = Vec3::ZERO;
    let view = Mat4::look_at_rh(eye, Vec3::NEG_Z, Vec3::Y);
    let proj = projection();
    let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);
    let mut lights = GpuLights::new(&device);
    let mut frame = kooch_lighting::LightFrame::extract(&resources);
    lights.update(&device, &queue, &resources, camera, None, &mut frame);

    let depth_view = depth_texture(&device, &queue, 0.02);
    let config = PageConfig::default();
    let clipmap = ClipmapConfig::default();
    let mut marker = PageMarker::new(&device, config, clipmap);
    marker.set_pool(
        &device,
        PoolConfig {
            pages: 512,
            views: 2,
        },
    );

    for slice in 0..2u32 {
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
            Some(Vec3::new(0.3, -1.0, 0.2)),
            (SIZE, SIZE),
            slice,
            1,
            100,
            Paint {
                target: &paint_target(&device),
                on: false,
                size: (SIZE, SIZE),
            },
        );
        queue.submit([encoder.finish()]);
        wait(&device);
    }

    // A page carries its camera in the high part: `page / span`, with
    // the span one stride per light plus one for the sun.
    let lights_count = lights.light_count().max(1);
    let stride = {
        let local = config.face_pages() * 6;
        let sun = clipmap.levels * config.side(0).pow(2);
        local.max(sun).div_ceil(32) * 32
    };
    let span = stride as u64 * (lights_count + 1) as u64;

    let mut per_view = [0u32; 2];
    for key in read_words(&device, &queue, marker.pool().keys()) {
        if key == 0 {
            continue;
        }
        let owner = ((key - 1) as u64 / span) as usize;
        assert!(owner < 2, "a key belongs to camera {owner}");
        per_view[owner] += 1;
    }
    assert!(
        per_view[0] > 0,
        "camera 0's pages were wiped by camera 1: {per_view:?}"
    );
    assert!(per_view[1] > 0, "camera 1 marked nothing: {per_view:?}");
}

// ---------------------------------------------------------------------
// Persistence (#866 A). The pool outlives the frame that filled it.
// ---------------------------------------------------------------------

/// Marks the same view `frames` times through one marker, returning what
/// each frame counted.
///
/// The camera does not move, so after the first frame every request is
/// for a page that is already there. That is the whole point: a scene
/// standing still should stop allocating.
fn run_frames(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    resources: &Resources,
    frames: u32,
    max_age: u32,
    pool: PoolConfig,
) -> Vec<MarkCounts> {
    let eye = Vec3::ZERO;
    let view = Mat4::look_at_rh(eye, Vec3::NEG_Z, Vec3::Y);
    let proj = projection();
    let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);

    let mut lights = GpuLights::new(device);
    let mut frame = kooch_lighting::LightFrame::extract(resources);
    lights.update(device, queue, resources, camera, None, &mut frame);

    let depth_view = depth_texture(device, queue, 0.01);
    let target = paint_target(device);
    let mut marker = PageMarker::new(device, PageConfig::default(), ClipmapConfig::default());
    marker.set_pool(device, pool);
    marker.set_max_age(max_age);

    let mut out = Vec::new();
    for index in 0..frames {
        marker.set_frame(index);
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
            Some(Vec3::new(0.3, -1.0, 0.2)),
            (SIZE, SIZE),
            /* view */ 0,
            /* rate */ 1,
            100,
            Paint {
                target: &target,
                on: false,
                size: (SIZE, SIZE),
            },
        );
        queue.submit([encoder.finish()]);
        marker.poll();
        wait(device);
        marker.poll();
        out.push(marker.last().expect("the counters came back"));
    }
    out
}

/// A page nothing stopped wanting is still there next frame, and cost
/// nothing to have.
///
/// 🔴 The reading the whole change exists for. Frame 0 allocates; every
/// frame after it reuses, allocates nothing, and therefore has nothing
/// to rasterise. Before this, every frame allocated every page again.
#[test]
fn a_page_survives_a_frame_that_wants_it() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let resources = world();
    let frames = run_frames(&device, &queue, &resources, 3, 8, PoolConfig::default());

    let first = frames[0];
    assert!(first.resident > 0, "the frame needs pages");
    assert_eq!(first.pool.reused, 0, "nothing was resident before frame 0");
    assert_eq!(
        first.pool.claims, first.resident,
        "frame 0 allocates them all"
    );

    for (index, counts) in frames.iter().enumerate().skip(1) {
        assert_eq!(
            counts.pool.claims, 0,
            "frame {index} allocated {} pages it already had",
            counts.pool.claims
        );
        assert_eq!(
            counts.pool.reused, counts.resident,
            "frame {index} reused {} of {} requests",
            counts.pool.reused, counts.resident
        );
    }
}

/// Every request is answered exactly once, whether by a reuse or by an
/// allocation.
///
/// ⚠️ This is the tombstone test. A lookup that stopped at a freed entry
/// would declare a resident page missing and allocate a SECOND slot for
/// it, which shows up here as the two halves summing past `resident` —
/// and nowhere else, because both pages then rasterise correctly and
/// only the pool runs out early.
#[test]
fn a_request_is_answered_once() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let resources = world();
    // Age 0 evicts everything every frame, so every frame after the
    // first walks a table made entirely of tombstones. That is the
    // hostile case on purpose.
    for max_age in [0u32, 1, 8] {
        let frames = run_frames(
            &device,
            &queue,
            &resources,
            4,
            max_age,
            PoolConfig::default(),
        );
        for (index, counts) in frames.iter().enumerate() {
            assert_eq!(
                counts.pool.claims + counts.pool.reused,
                counts.resident,
                "age {max_age} frame {index}: {} claims + {} reuses is not {} requests",
                counts.pool.claims,
                counts.pool.reused,
                counts.resident
            );
            assert_eq!(
                counts.pool.leaked, 0,
                "age {max_age} frame {index} double-freed a slot"
            );
        }
    }
}

/// A slot freed by eviction is handed out again.
///
/// 🔴 Without recycling the pool is a bump allocator over the session
/// rather than over the frame: it runs out after `slice` distinct pages
/// have EVER been asked for. Four frames at age 0 request roughly four
/// times what one frame does, so a pool sized for one frame proves it.
#[test]
fn an_evicted_slot_comes_back() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let resources = world();
    // A pool sized so that ONE frame fits and six do not, which is what
    // makes recycling the only way through.
    let pool = PoolConfig { pages: 8, views: 1 };
    let frames = run_frames(&device, &queue, &resources, 6, 0, pool);

    let first = frames[0];
    assert!(
        first.pool.claims * frames.len() as u32 > pool.slice(),
        "the run has to ask for more pages than the pool holds, or it proves nothing: \
         {} claims over {} frames against {} slots",
        first.pool.claims,
        frames.len(),
        pool.slice()
    );
    for (index, counts) in frames.iter().enumerate() {
        assert_eq!(
            counts.pool.overflow, 0,
            "frame {index} ran out of pool with {} evictions behind it",
            counts.pool.evicted
        );
    }
    assert!(
        frames[1..].iter().any(|c| c.pool.evicted > 0),
        "age 0 has to evict every frame"
    );
}

/// Ageing is measured in FRAMES, and `max_age` decides how many.
///
/// 🔴 `age_view` runs BEFORE the marking, so a page requested last frame
/// is already one frame old when it is judged. At `max_age` 0 that is
/// too old and it is evicted and immediately re-requested — which is
/// exactly the behaviour that came before persistence, produced by the
/// machine that replaces it. At 1 or more it survives. Both directions
/// are asserted, because a threshold nothing tests off-by-one is a
/// threshold nobody knows the meaning of.
#[test]
fn max_age_decides_whether_a_page_is_kept() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let resources = world();

    let churn = run_frames(&device, &queue, &resources, 3, 0, PoolConfig::default());
    for (index, counts) in churn.iter().enumerate().skip(1) {
        assert!(
            counts.pool.evicted > 0,
            "age 0, frame {index}: nothing was evicted"
        );
        assert_eq!(
            counts.pool.alive, 0,
            "age 0, frame {index} kept pages alive"
        );
        assert_eq!(
            counts.pool.claims, counts.resident,
            "age 0, frame {index} should re-allocate everything"
        );
    }

    let kept = run_frames(&device, &queue, &resources, 3, 1, PoolConfig::default());
    for (index, counts) in kept.iter().enumerate().skip(1) {
        assert_eq!(
            counts.pool.evicted, 0,
            "age 1, frame {index} evicted a page the frame was still asking for"
        );
        assert!(
            counts.pool.alive > 0,
            "age 1, frame {index} kept nothing alive"
        );
    }
}
