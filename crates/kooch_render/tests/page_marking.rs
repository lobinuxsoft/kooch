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
use kooch_render::shadow::pages::pool::{DEFAULT_MAX_AGE, PAGE_CELL, PoolConfig};
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
/// Which marking path `run_pool` builds. The environment switch is an
/// `OnceLock`, so one process cannot answer both ways; this can.
thread_local! {
    static CLUSTER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// How far a receiver dilates its page request, in pages. Same
    /// reason as `CLUSTER`: `run_pool` builds the marker itself.
    static HALO: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
    /// The projected radius under which a light joins the distant tier
    /// (#1009). Zero is the tier off, which is what every other test
    /// here wants.
    static COVERAGE: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Runs `body` with the cluster/light marking on (#952).
fn with_clusters<T>(body: impl FnOnce() -> T) -> T {
    CLUSTER.with(|c| c.set(true));
    let out = body();
    CLUSTER.with(|c| c.set(false));
    out
}

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
    marker.set_cluster_marking(CLUSTER.with(|c| c.get()));
    marker.set_halo(HALO.with(|h| h.get()));
    marker.set_coverage(COVERAGE.with(|c| c.get()));
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
    // 🔴 A dispatch of its own now, recorded where the frame records it:
    // after the shading. The marking moved to the top of the frame so
    // the raster can fill the atlas before anything samples it, and the
    // paint could not go with it — it writes the view's FINAL colour,
    // which at that point still holds the last frame.
    marker.record_paint(&mut encoder, (SIZE, SIZE));
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
}

/// A local light claims its pages, now that something draws them.
///
/// # 🔴 The guard this replaces, and why it was there
///
/// Local pages used to be marked and NOT claimed. Measured on
/// `many_lights` with two viewports, claiming them held **991 and 1004
/// of each camera's 1024 slots**, leaving the sun — the raster's only
/// consumer at the time — 33 and 20 pages. The pool reported itself
/// 100 % full while producing almost no shadow.
///
/// What makes claiming safe is that the rest of the chain now exists:
/// the compaction buckets a lamp's pages by octave, the expansion tests
/// them against the lamp's own frustum, and the depth pass builds that
/// frustum from the light the page names. A claimed page is a drawn
/// page.
///
/// ⚠️ The pressure is real and did not go away. What changed is that
/// the pages bought something — `RasterCounts::local` and the pool's own
/// overflow counter are what say when the budget stops covering it.
#[test]
fn a_local_light_claims_its_pages() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 40.0);

    let dark = run(&device, &queue, &resources, 0.02, None);
    assert!(dark.resident > 0, "the point light marked nothing");
    assert_eq!(
        dark.pool.claims, dark.resident,
        "one claim per distinct page: {} claims of {} resident",
        dark.pool.claims, dark.resident
    );
    assert_eq!(
        dark.pool.unspent(dark.resident),
        0,
        "a marked local page is no longer a page nothing spends"
    );

    // And with a sun as well: both chains draw from one pool, which is
    // the budget line this whole track is measured against.
    let sunny = run(
        &device,
        &queue,
        &resources,
        0.02,
        Some(Vec3::new(0.3, -1.0, 0.2)),
    );
    assert!(sunny.pool.claims > 0, "nothing claimed with a sun in frame");
    assert!(
        sunny.resident > dark.resident,
        "adding a sun did not add pages: {} against {}",
        sunny.resident,
        dark.resident
    );
    assert_eq!(sunny.pool.overflow, 0, "a default pool overflowed");
}

#[test]
fn a_full_pool_denies_by_rank() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    // The SUN fills it: it is the only thing that spends the pool. A
    // NEAR surface, because a far one is one coarse clipmap level and a
    // handful of pages — not enough to overflow even a four-page pool.
    let resources = world();
    let small = PoolConfig {
        pages: 4,
        views: 1,
        row_cap: u32::MAX,
    };
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
    // 🔴 With the seating plan (#942) a request the slice cannot fund is
    // DENIED at its rank, not dropped at the allocator: the plan funds
    // exactly `slice` seats, so the free list never runs dry and
    // `overflow` — the allocator's own miss — stays zero.
    assert_eq!(
        counts.pool.claims + counts.pool.reused,
        small.slice(),
        "the pool handed out every slot it had"
    );
    assert!(
        counts.pool.denied > 0,
        "a pool of {} could not answer {} requests and said nothing",
        small.slice(),
        counts.resident
    );
    assert!(
        counts.pool.cutoff < 32,
        "denials without a cutoff: the plan did not run"
    );
    assert_eq!(
        counts.pool.overflow, 0,
        "the allocator missed — the plan's arithmetic does not close"
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
    let slots = read_words(&device, &queue, marker.pool().slots());
    let capacity = marker.pool().config().total();

    let mut resident = 0u32;
    let mut seen_slots = std::collections::HashSet::new();
    for entry in 0..slots.len() / PAGE_CELL as usize {
        // The first word is `slot + 1`; `PAGE_ABSENT` (0) names nothing.
        let stored = slots[entry * PAGE_CELL as usize];
        if stored == 0 {
            continue;
        }
        resident += 1;
        let slot = stored - 1;
        assert!(slot < capacity, "slot {slot} past the pool");
        assert!(seen_slots.insert(slot), "slot {slot} handed out twice");
    }
    assert_eq!(
        resident,
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
            row_cap: u32::MAX,
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

    // The table is flat and a view's entries are a contiguous run, so
    // ownership is the entry's position against the span — the same
    // arithmetic `view_base` uses, rebuilt here so the two can disagree.
    let lights_count = lights.light_count().max(1);
    let padded = lights_count.max(1).next_multiple_of(64);
    let stride = (config.local_face_pages() * 6).div_ceil(32) * 32;
    let span = (padded as u64 * stride as u64
        + clipmap.levels as u64 * (config.side(0) as u64).pow(2))
    .div_ceil(32)
        * 32;

    let slots = read_words(&device, &queue, marker.pool().slots());
    let mut per_view = [0u32; 2];
    for entry in 0..slots.len() / PAGE_CELL as usize {
        if slots[entry * PAGE_CELL as usize] == 0 {
            continue;
        }
        let owner = (entry as u64 / span) as usize;
        assert!(owner < 2, "an entry belongs to camera {owner}");
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
    // Shadow texels per screen pixel, per frame. Varying it moves which
    // clipmap levels are marked, which is a standing camera's cheapest
    // way to ask for DIFFERENT pages each frame — the case where an
    // unreused hole is left behind for good.
    density: &dyn Fn(u32) -> u32,
    // Where the camera stands on each frame. A clipmap is centred on it,
    // so moving it is what makes a frame ask for DIFFERENT pages than
    // the last one — the case a standing camera cannot produce.
    eye_of: &dyn Fn(u32) -> Vec3,
    // Where the sun points on each frame.
    //
    // 🔴 The most hostile input there is, and the user's suggestion.
    // `sun_basis` is built from this direction, so rotating it moves
    // EVERY page's identity at once — a frame after a rotation shares
    // nothing with the frame before it, and every entry the last frame
    // filed becomes a hole nobody will ever probe through again.
    sun_of: &dyn Fn(u32) -> Vec3,
) -> Vec<MarkCounts> {
    let proj = projection();
    let mut lights = GpuLights::new(device);
    let depth_view = depth_texture(device, queue, 0.01);
    let target = paint_target(device);
    let mut marker = PageMarker::new(device, PageConfig::default(), ClipmapConfig::default());
    marker.set_pool(device, pool);
    marker.set_max_age(max_age);

    let mut out = Vec::new();
    for index in 0..frames {
        let eye = eye_of(index);
        let view = Mat4::look_at_rh(eye, eye + Vec3::NEG_Z, Vec3::Y);
        let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);
        let mut frame = kooch_lighting::LightFrame::extract(resources);
        lights.update(device, queue, resources, camera, None, &mut frame);

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
            Some(sun_of(index)),
            (SIZE, SIZE),
            /* view */ 0,
            /* rate */ 1,
            density(index),
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
    let frames = run_frames(
        &device,
        &queue,
        &resources,
        3,
        8,
        PoolConfig::default(),
        &|_| 100,
        &|_| Vec3::ZERO,
        &|_| Vec3::new(0.3, -1.0, 0.2),
    );

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
            &|_| 100,
            &|_| Vec3::ZERO,
            &|_| Vec3::new(0.3, -1.0, 0.2),
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
    let pool = PoolConfig {
        pages: 8,
        views: 1,
        row_cap: u32::MAX,
    };
    let frames = run_frames(
        &device,
        &queue,
        &resources,
        6,
        0,
        pool,
        &|_| 100,
        &|_| Vec3::ZERO,
        &|_| Vec3::new(0.3, -1.0, 0.2),
    );

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

    let churn = run_frames(
        &device,
        &queue,
        &resources,
        3,
        0,
        PoolConfig::default(),
        &|_| 100,
        &|_| Vec3::ZERO,
        &|_| Vec3::new(0.3, -1.0, 0.2),
    );
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

    let kept = run_frames(
        &device,
        &queue,
        &resources,
        3,
        1,
        PoolConfig::default(),
        &|_| 100,
        &|_| Vec3::ZERO,
        &|_| Vec3::new(0.3, -1.0, 0.2),
    );
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

// `holes_do_not_accumulate` lived here and is retired with the hash it
// measured: the flat table has no probe runs, so an eviction cannot
// leave a hole for a lookup to walk. See `page_table.wgsl`.

/// A page that stays resident keeps the SAME physical slot.
///
/// 🔴 The property the fused raster depends on and nobody wrote down.
/// `vbuf64.render` rasterises and shades in one pass, so the shading
/// samples an atlas a frame old while reading THIS frame's table. That
/// only works if the two agree about where a page lives — and before
/// persistence they did by accident, because the allocator was a bump
/// from zero every frame and handed the same page the same slot as long
/// as the marking order held.
///
/// A free list has no such order. If a page can be freed and re-taken
/// into a different slot, the table says slot 7 and last frame's atlas
/// has that page in slot 3, with something else in 7.
#[test]
fn a_resident_page_keeps_its_slot() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let resources = world();
    let pool = PoolConfig::default();

    // Two frames, camera and sun still, reading the table after each.
    let mut placements: Vec<std::collections::HashMap<u32, u32>> = Vec::new();
    let eye = Vec3::ZERO;
    let view = Mat4::look_at_rh(eye, Vec3::NEG_Z, Vec3::Y);
    let proj = projection();
    let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);
    let mut lights = GpuLights::new(&device);
    let mut frame = kooch_lighting::LightFrame::extract(&resources);
    lights.update(&device, &queue, &resources, camera, None, &mut frame);
    let depth_view = depth_texture(&device, &queue, 0.01);
    let target = paint_target(&device);
    let mut marker = PageMarker::new(&device, PageConfig::default(), ClipmapConfig::default());
    marker.set_pool(&device, pool);

    for index in 0..4u32 {
        marker.set_frame(index);
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
            0,
            1,
            100,
            Paint {
                target: &target,
                on: false,
                size: (SIZE, SIZE),
            },
        );
        queue.submit([encoder.finish()]);
        let cells = read_words(&device, &queue, marker.pool().slots());
        let mut placed = std::collections::HashMap::new();
        for entry in 0..cells.len() / PAGE_CELL as usize {
            let stored = cells[entry * PAGE_CELL as usize];
            if stored == 0 {
                continue;
            }
            // The entry index IS the page id; the word is `slot + 1`.
            placed.insert(entry as u32, stored - 1);
        }
        placements.push(placed);
    }

    let first = &placements[1];
    assert!(!first.is_empty(), "the frame filed no pages");
    for (index, later) in placements.iter().enumerate().skip(2) {
        for (page, slot) in first {
            if let Some(now) = later.get(page) {
                assert_eq!(
                    now, slot,
                    "page {page} moved from slot {slot} to {now} by frame {index}"
                );
            }
        }
    }
}

/// A camera that keeps moving does not run the pool dry.
///
/// 🔴 The measurement, not a guess. With a long `max_age` a page stays
/// resident for a second after the last frame that wanted it, so a
/// camera sweeping across a scene accumulates the pages of everywhere it
/// has been. If that fills the slice, new pages get no slot, render
/// unshadowed, and come back when something finally ages out — a shadow
/// that blinks in and out, which is what the user reported.
///
/// What it pins is the failure, not a policy. Eviction under pressure
/// exists now — `preempt_view`, #942 — so a walking camera's stale
/// pages are reseated the frame the plan stops funding them; what this
/// asserts is that the reseating actually keeps up.
#[test]
fn a_moving_camera_does_not_exhaust_the_pool() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let resources = world();
    let frames = run_frames(
        &device,
        &queue,
        &resources,
        90,
        DEFAULT_MAX_AGE,
        PoolConfig::default(),
        &|_| 100,
        // A metre and a half a second at 60 Hz, which is a walk.
        &|i| Vec3::new(i as f32 * 0.025, 0.0, i as f32 * -0.025),
        &|_| Vec3::new(0.3, -1.0, 0.2),
    );

    let peak = frames.iter().map(|c| c.pool.allocated()).max().unwrap_or(0);
    // A denial is starvation with a name on it — it counts the same.
    let spilled: u32 = frames.iter().map(|c| c.pool.overflow + c.pool.denied).sum();
    eprintln!(
        "peak {peak} of {} slots, {spilled} pages went unallocated",
        frames[0].pool.capacity
    );
    assert_eq!(
        spilled, 0,
        "{spilled} pages found no slot; the pool peaked at {peak} of {}",
        frames[0].pool.capacity
    );
}

/// The CPU mirror of `entry_rank` in `page_mark.wgsl`, decode for
/// decode, so the test can name the rank of every survivor.
fn entry_rank(config: &PageConfig, clip_levels: u32, within: u32) -> u32 {
    let stride = (config.local_face_pages() * 6).div_ceil(32) * 32;
    // The tests run well under 64 lights, so the padded slot count is
    // the first step: 64.
    let sun_base = 64 * stride;
    if within >= sun_base {
        let cell = config.side(0).pow(2);
        let level = ((within - sun_base) / cell).min(clip_levels - 1);
        return (clip_levels - 1 - level).min(31);
    }
    let face = (within % stride) % config.local_face_pages();
    let mut level = config.local_floor();
    let mut next = config.side(level).pow(2);
    while level + 1 < config.levels() && face >= next {
        level += 1;
        next += config.side(level).pow(2);
    }
    (clip_levels + (config.levels() - 1 - level)).min(31)
}

/// Under pressure, what survives is the top of the ranking — never a
/// page the plan ranked below one it turned away. The issue's own
/// acceptance test: plant more requests than slots, read the table,
/// and check every resident against the cutoff the plan reported.
#[test]
fn the_survivors_are_the_top_ranks() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    // A lamp alongside the sun, so the demand spans both classes and
    // the local ranks are really in the contest they are meant to lose.
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);
    let small = PoolConfig {
        pages: 4,
        views: 1,
        row_cap: u32::MAX,
    };
    let (marker, counts) = run_pool(
        &device,
        &queue,
        &resources,
        0.6,
        Some(Vec3::new(0.3, -1.0, 0.2)),
        400,
        small,
    );
    assert!(counts.pool.denied > 0, "no pressure, nothing to rank");
    let cutoff = counts.pool.cutoff;
    assert!(cutoff < 32, "denials without a cutoff");

    let config = PageConfig::default();
    let clip_levels = ClipmapConfig::default().levels;
    let cells = read_words(&device, &queue, marker.pool().slots());
    let mut residents = 0;
    for entry in 0..cells.len() / PAGE_CELL as usize {
        if cells[entry * PAGE_CELL as usize] == 0 {
            continue;
        }
        residents += 1;
        let rank = entry_rank(&config, clip_levels, entry as u32);
        assert!(
            rank <= cutoff,
            "entry {entry} of rank {rank} kept its seat past the cutoff {cutoff}"
        );
    }
    assert_eq!(residents, small.slice(), "the slice seated exactly itself");
}

/// A saturated pool reseats the frame the camera moves: the new view's
/// pages take their seats from the stale ones IN THE SAME FRAME, not
/// after `max_age` lets them go. The starvation this replaces sat at
/// `0 new` forever while 6 652 requests waited.
#[test]
fn a_saturated_pool_reseats_on_move() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let resources = world();
    let small = PoolConfig {
        pages: 4,
        views: 1,
        row_cap: u32::MAX,
    };
    let eye = Vec3::ZERO;
    let view = Mat4::look_at_rh(eye, Vec3::NEG_Z, Vec3::Y);
    let proj = projection();
    let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);
    let mut lights = GpuLights::new(&device);
    let mut frame = kooch_lighting::LightFrame::extract(&resources);
    lights.update(&device, &queue, &resources, camera, None, &mut frame);
    let target = paint_target(&device);
    let mut marker = PageMarker::new(&device, PageConfig::default(), ClipmapConfig::default());
    marker.set_pool(&device, small);

    // Two frames at two depths: the surface moves, so the second frame
    // wants pages the first one never marked — against a full slice.
    let mut last = None;
    for (index, depth) in [0.6f32, 0.15].into_iter().enumerate() {
        marker.set_frame(index as u32);
        let depth_view = depth_texture(&device, &queue, depth);
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
            0,
            1,
            400,
            Paint {
                target: &target,
                on: false,
                size: (SIZE, SIZE),
            },
        );
        queue.submit([encoder.finish()]);
        marker.poll();
        wait(&device);
        marker.poll();
        last = marker.last();
    }
    let counts = last.expect("the counters came back");
    assert!(
        counts.pool.claims > 0,
        "the camera moved against a full pool and nothing reseated: {:?}",
        counts.pool
    );
    assert!(
        counts.pool.preempted > 0,
        "new pages were seated but no stale resident paid for them: {:?}",
        counts.pool
    );
}

/// The pressure bias settles the denials (#943): a pool too small for
/// the frame converges, one level per frame, to a marking that fits —
/// and then HOLDS, because the step down needs slack the settled state
/// does not have. The acceptance criteria of the issue, in order:
/// denials reach zero, the bias is the reason, and it does not
/// oscillate.
#[test]
fn the_bias_settles_the_denials() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let resources = world();
    let small = PoolConfig {
        pages: 4,
        views: 1,
        row_cap: u32::MAX,
    };
    let eye = Vec3::ZERO;
    let view = Mat4::look_at_rh(eye, Vec3::NEG_Z, Vec3::Y);
    let proj = projection();
    let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);
    let mut lights = GpuLights::new(&device);
    let mut frame = kooch_lighting::LightFrame::extract(&resources);
    lights.update(&device, &queue, &resources, camera, None, &mut frame);
    let depth_view = depth_texture(&device, &queue, 0.6);
    let target = paint_target(&device);
    let mut marker = PageMarker::new(&device, PageConfig::default(), ClipmapConfig::default());
    marker.set_pool(&device, small);

    // A near surface at four times the screen's density wants more sun
    // pages than four slots hold; the bias has up to six steps (four
    // local, two sun) plus the readback lag to settle in.
    let mut series = Vec::new();
    for index in 0..12u32 {
        marker.set_frame(index);
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
            0,
            1,
            400,
            Paint {
                target: &target,
                on: false,
                size: (SIZE, SIZE),
            },
        );
        queue.submit([encoder.finish()]);
        marker.poll();
        wait(&device);
        marker.poll();
        if let Some(counts) = marker.last() {
            series.push(counts);
        }
    }
    let last = series.last().expect("counters came back");
    assert!(
        last.pool.bias_sun > 0,
        "the demand never fit and the sun never paid: {:?}",
        last.pool
    );
    assert_eq!(
        last.pool.denied, 0,
        "the bias settled at +{} local +{} sun and pages still starve",
        last.pool.bias_local, last.pool.bias_sun
    );
    // No oscillation: once settled, a constant demand is a constant
    // bias. The last three frames have to agree.
    let tail: Vec<_> = series
        .iter()
        .rev()
        .take(3)
        .map(|c| (c.pool.bias_local, c.pool.bias_sun))
        .collect();
    assert!(
        tail.windows(2).all(|w| w[0] == w[1]),
        "the bias oscillates at the end: {tail:?}"
    );

    // And it unwinds: drop the demand to almost nothing and the bias
    // walks back to zero on its own — quality is only ever borrowed.
    // Two trial steps 16 frames of patience apart, plus the readback
    // ring's lag: 48 relaxed frames is the controller's own arithmetic.
    let mut relaxed = None;
    for index in 12..60u32 {
        marker.set_frame(index);
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
            0,
            1,
            25,
            Paint {
                target: &target,
                on: false,
                size: (SIZE, SIZE),
            },
        );
        queue.submit([encoder.finish()]);
        marker.poll();
        wait(&device);
        marker.poll();
        relaxed = marker.last();
    }
    let relaxed = relaxed.expect("counters came back");
    assert_eq!(
        (relaxed.pool.bias_local, relaxed.pool.bias_sun),
        (0, 0),
        "the demand shrank and the bias never gave the quality back: {:?}",
        relaxed.pool
    );
    assert_eq!(relaxed.pool.denied, 0, "relaxed and still denying");
}

/// A light too small on screen drops to ONE page per cube face instead
/// of a chain (#1009), and gets its chain back the moment the threshold
/// would pass it — here by turning it off, which is the same comparison
/// a closer camera flips.
///
/// 🔴 This test used to be `a_tiny_light_casts_nothing` and asserted
/// `resident == 0`. That was #944's cliff, and the measurement that
/// retired it is in `light_distant`: on `dense.scene` the cliff silenced
/// 34 lights and the pool was still full, because the thirty above it
/// took every slot.
#[test]
fn a_tiny_light_drops_to_one_page() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    // Reach 2 m at 10 m: a dozen-odd pixels of projected radius on the
    // test viewport — under a 32 px gate, over a disabled one. The
    // surface sits AT the lamp's depth so its centre pixels are lit.
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 2.0);

    let run_gated = |pixels: u32| {
        let eye = Vec3::ZERO;
        let view = Mat4::look_at_rh(eye, Vec3::NEG_Z, Vec3::Y);
        let proj = projection();
        let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);
        let mut lights = GpuLights::new(&device);
        let mut frame = kooch_lighting::LightFrame::extract(&resources);
        lights.update(&device, &queue, &resources, camera, None, &mut frame);
        let depth_view = depth_texture(&device, &queue, 0.01);
        let target = paint_target(&device);
        let mut marker = PageMarker::new(&device, PageConfig::default(), ClipmapConfig::default());
        marker.set_coverage(pixels);
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
            // No sun: every page below is the lamp's own.
            None,
            (SIZE, SIZE),
            0,
            1,
            // 🔴 The finest density the settings allow, so the chain
            // case actually asks for a chain. At 100 the receivers this
            // rig writes are far enough that `page_level` lands near the
            // top by itself, and both runs came back with four pages —
            // the tier looked free and proved nothing.
            400,
            Paint {
                target: &target,
                on: false,
                size: (SIZE, SIZE),
            },
        );
        queue.submit([encoder.finish()]);
        marker.poll();
        wait(&device);
        marker.poll();
        marker.last().expect("the counters came back")
    };

    let open = run_gated(0);
    assert!(open.resident > 0, "the undemoted lamp marked nothing");
    assert_eq!(open.distant, 0, "an off threshold demoted something");

    let demoted = run_gated(32);
    assert!(
        demoted.resident > 0,
        "a lamp under the threshold cast nothing: the cliff is back"
    );
    assert!(demoted.distant > 0, "nothing was counted as distant");
    assert!(
        demoted.resident <= 6,
        "a distant lamp claimed {} pages; a cube has six faces and each gets one",
        demoted.resident
    );
    assert!(
        demoted.resident < open.resident,
        "the tier cost as much as the chain: {} against {}",
        demoted.resident,
        open.resident
    );
    assert_eq!(
        demoted.pairs, open.pairs,
        "the threshold changed the grid walk instead of the marking"
    );
}

/// The per-pixel path counts into workgroup memory, never into a global
/// counter.
///
/// 🔴 `mark_pixel` runs one thread per pixel and loops over the lights
/// of that pixel's cluster. A global `atomicAdd` in there lands every
/// thread of the dispatch on one address: at the OneXFly's resolution,
/// millions of increments serialised on two words, inside a pass
/// measured at a flat 13.975 ms (#952). The counts are load-bearing —
/// the panel and #942's plan read them — so they are reduced per
/// workgroup and flushed once, and this is what stops the cheap-looking
/// one-liner from coming back.
///
/// A source check, because the defect is invisible in behaviour: the
/// census comes out identical either way, only slower.
#[test]
fn the_hot_path_counts_in_workgroup_memory() {
    let source = include_str!("../shaders/page_mark.wgsl");
    let (_, body) = source
        .split_once("fn mark_pixel(")
        .expect("page_mark.wgsl has no mark_pixel");
    // To the next top-level item, which is where the per-pixel path ends.
    let body = body.split("\n@").next().unwrap_or(body);
    assert!(
        !body.contains("&counters["),
        "mark_pixel touches a global counter; every pixel of the dispatch \
         would serialise on that one address"
    );
    assert!(
        body.contains("&tally["),
        "mark_pixel counts nothing into workgroup memory — the census is \
         either gone or back on the global counters"
    );
}

/// The occupancy census counts froxels, and there are fewer of them than
/// there are samples.
///
/// 🔴 The ratio the move to cluster/light pairs rests on. Olsson §III
/// derives page masks from cluster bounds "several orders of magnitude
/// fewer than the samples", and this engine measured 3 369 702
/// sample/light pairs against 218 772 covered pixels (#952). A census
/// that counted samples, or the whole grid, would make that comparison
/// meaningless — so the two properties worth pinning are that it counts
/// something, and that it counts FEWER things.
#[test]
fn the_census_counts_froxels_not_samples() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);
    let counts = run(&device, &queue, &resources, 0.01, None);

    assert!(counts.samples > 0, "the harness drew no surface");
    assert!(counts.froxels > 0, "a full screen of surface occupied none");
    assert!(
        counts.froxels < counts.samples,
        "froxels {} against {} samples — the census is counting pixels",
        counts.froxels,
        counts.samples
    );
    // The bitmap is 4096 bits wide and the grid is capped to match.
    assert!(
        counts.froxels <= 4096,
        "{} froxels, past the bitmap",
        counts.froxels
    );
}

/// Sky occupies nothing.
///
/// The census reads the same early return the marking does, so a frame
/// with no surface must report an empty grid rather than the whole one —
/// which is the property that keeps a cluster pass from marking pages
/// for empty air.
#[test]
fn sky_occupies_no_froxel() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);
    let counts = run(&device, &queue, &resources, 0.0, None);
    assert_eq!(counts.samples, 0, "the harness drew a surface");
    assert_eq!(counts.froxels, 0, "sky occupied a froxel");
}

/// The cluster path marks the same scene for a fraction of the pairs.
///
/// 🔴 Olsson §III, measured against the path it replaces, in one
/// process. `many_lights` on the OneXFly walks 2 937 330 sample/light
/// pairs where 199 occupied froxels at 17.9 lights each would walk
/// ~3 560 — 824x — and the whole point is that the cheaper answer is
/// still an answer: pages get marked, nothing overflows, and the
/// coarsest-corner rule keeps every one of them reachable by a reader
/// walking its chain from the fine end.
#[test]
fn the_cluster_path_marks_for_fewer_pairs() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);

    let per_pixel = run(&device, &queue, &resources, 0.01, None);
    let per_froxel = with_clusters(|| run(&device, &queue, &resources, 0.01, None));

    assert!(per_pixel.pairs > 0, "the per-pixel path walked nothing");
    assert_eq!(per_froxel.overflow, 0, "a page index past the buffer");
    // 🔴 Olsson's EXPLICIT bounds, in one number. A froxel is mostly
    // empty and its box is a slab; marking the box asks for pages across
    // depth that holds nothing. With the implicit bounds this scene sent
    // the resolution bias straight to its ceiling — `locals +4 · sun +2`
    // and 21 pages denied — so a superset is required and a superset
    // three times over is the feature failing.
    assert!(
        per_froxel.resident <= per_pixel.resident * 2,
        "cluster marked {} pages against {} — the over-marking is what \
         the pool pays, and it is unbounded",
        per_froxel.resident,
        per_pixel.resident
    );
    // 🔴 The safety property, and the only direction an approximation of
    // "which pages does this scene need" may err in. A froxel is a
    // frustum and its rect on a cube face is that frustum's bounding
    // box, so the cluster path marks a SUPERSET: pages nothing samples
    // cost a pool slot, pages nobody marked cost a shadow. Measured
    // here at 22 against 12.
    assert!(
        per_froxel.resident >= per_pixel.resident,
        "cluster marked {} pages against the per-pixel path's {} — it is \
         marking FEWER, which is a missing shadow",
        per_froxel.resident,
        per_pixel.resident
    );
    // And the whole point: an order of magnitude fewer walks.
    assert!(
        per_froxel.pairs * 10 < per_pixel.pairs,
        "cluster pairs {} against per-pixel {} — not the win the rewrite \
         is for",
        per_froxel.pairs,
        per_pixel.pairs
    );
}

/// The counts say which path produced them.
///
/// 🔴 `pairs` means (pixel, light) on one path and (froxel, light) on
/// the other, and the panel divided it by samples either way — printing
/// `0.0 lights each` beside a multiplier it had invented. A number whose
/// MEANING changes with a switch has to carry the switch.
#[test]
fn the_counts_say_which_path_walked_them() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);

    let per_pixel = run(&device, &queue, &resources, 0.01, None);
    let per_froxel = with_clusters(|| run(&device, &queue, &resources, 0.01, None));

    assert!(!per_pixel.by_froxel, "the per-pixel path claimed froxels");
    assert!(per_froxel.by_froxel, "the froxel path claimed pixels");
    // And the ratio each side implies is the same order of magnitude,
    // which is what makes the panel's comparison honest.
    let by_pixel = per_pixel.pairs as f32 / per_pixel.samples.max(1) as f32;
    let by_froxel = per_froxel.pairs as f32 / per_froxel.froxels.max(1) as f32;
    assert!(
        by_froxel > by_pixel * 0.5,
        "lights per froxel {by_froxel} against lights per pixel {by_pixel} — \
         a froxel holds at least as many lights as a pixel inside it"
    );
}

/// Lamps that overrun the pool do not blur the sun.
///
/// 🔴 The ranking has always had this right — a clipmap level never
/// loses to a lamp — and the pressure valve did not. `cutoff` is the
/// first rank the budget could not fund whole, the sun owns ranks
/// `0..chain.w`, so a cutoff at or past `chain.w` means every sun rank
/// was funded in full. Raising the sun's bias there buys back no page it
/// asked for; it only punishes the consumer that won the ranking for the
/// overdemand of the ones that lost.
///
/// Seen in `many_lights`: "the plan funded down to rank 17" — the sun
/// complete, the cut inside the lamps — beside `locals +4 · sun +2`.
/// Two levels of sun is four times the world per shadow texel on the one
/// shadow the scene is about, and walking somewhere with fewer lamps in
/// frame snapped it back. Resolution that depended on where you stood.
#[test]
fn lamps_that_overrun_the_pool_spare_the_sun() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    // Enough lamps, close enough, that their pages cannot all be seated.
    for i in 0..12 {
        let x = (i as f32 - 6.0) * 0.7;
        add_point(&mut resources, Vec3::new(x, 0.0, -4.0), 12.0);
    }
    let eye = Vec3::ZERO;
    let view = Mat4::look_at_rh(eye, Vec3::NEG_Z, Vec3::Y);
    let proj = projection();
    let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);
    let mut lights = GpuLights::new(&device);
    let mut frame = kooch_lighting::LightFrame::extract(&resources);
    lights.update(&device, &queue, &resources, camera, None, &mut frame);
    let depth_view = depth_texture(&device, &queue, 0.02);
    let target = paint_target(&device);
    let mut marker = PageMarker::new(&device, PageConfig::default(), ClipmapConfig::default());
    marker.set_pool(
        &device,
        PoolConfig {
            pages: 12,
            views: 1,
            row_cap: u32::MAX,
        },
    );

    let mut last = None;
    for index in 0..24u32 {
        marker.set_frame(index);
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
            0,
            1,
            100,
            Paint {
                target: &target,
                on: false,
                size: (SIZE, SIZE),
            },
        );
        queue.submit([encoder.finish()]);
        marker.poll();
        wait(&device);
        marker.poll();
        if let Some(counts) = marker.last() {
            last = Some(counts);
        }
    }
    let last = last.expect("counters came back");
    // The premise: the cut has to land among the LAMPS, or this proves
    // nothing about who pays.
    let sun_levels = ClipmapConfig::default().levels;
    assert!(
        last.pool.denied > 0,
        "nothing was denied — the pool absorbed the demand, and a scene \
         with no shortfall cannot say who should pay for one"
    );
    assert!(
        last.pool.cutoff >= sun_levels,
        "the plan cut at rank {} of {} sun ranks — the sun WAS denied, so \
         this scene cannot say who should pay",
        last.pool.cutoff,
        sun_levels
    );
    assert_eq!(
        last.pool.bias_sun, 0,
        "the sun was funded to its last rank and blurred anyway: \
         locals +{} sun +{}, cut at rank {}",
        last.pool.bias_local, last.pool.bias_sun, last.pool.cutoff
    );
}

/// The peak overlap is a peak, not the average, and both paths report it.
///
/// 🔴 `pairs / froxels` hides the case that hurts. Lights are authored
/// one at a time and the froxel they share is drawn nowhere, so the
/// number that matters is the worst cell — it decides the shading
/// loop's worst pixel and how much of the pool one cell can claim.
#[test]
fn the_census_reports_the_worst_froxel() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    // Stacked on one spot: every one of them reaches the same froxels.
    for i in 0..6 {
        let nudge = i as f32 * 0.01;
        add_point(&mut resources, Vec3::new(nudge, 0.0, -6.0), 40.0);
    }
    let counts = run(&device, &queue, &resources, 0.01, None);
    assert!(counts.froxels > 0, "no froxel was occupied");
    let average = counts.pairs as f32 / counts.samples.max(1) as f32;
    assert!(
        counts.peak_lights >= average.ceil() as u32,
        "peak {} is under the average {average} — it is not a peak",
        counts.peak_lights
    );
    assert!(
        counts.peak_lights >= 2,
        "six lights on one spot and the worst froxel saw {}",
        counts.peak_lights
    );

    // Same scene through the cluster path: overlap is a property of the
    // SCENE, so the alert has to read the same either way.
    let froxel = with_clusters(|| run(&device, &queue, &resources, 0.01, None));
    assert_eq!(
        froxel.peak_lights, counts.peak_lights,
        "the two marking paths disagree about how much the scene overlaps"
    );
}

/// The bias lands on its value in one step, not one step a frame.
///
/// 🔴 The raise used to move by one and wait for the next frame to see
/// whether that was enough: a scene needing four steps denied pages for
/// four frames, and gave them back over as many as ninety-six. The
/// player saw it — shadows blurring on the way into a lit area and
/// sharpening again on the way out, resolution as a function of where
/// they had been.
///
/// WickedEngine has no lag at all: it sizes lights from a formula, packs,
/// halves on failure and repacks inside the frame. Ours are measured by
/// a per-pixel pass, so one frame is the floor — and this pins that it
/// reaches the floor.
///
/// ⚠️ **Both paths, and they are allowed different bounds.** The raise
/// estimates a level as four pages becoming one, which is exact for the
/// sun's clipmap and only an upper bound for a lamp: `mark_face_rect`
/// caps a face's rect at `FROXEL_RECT_MAX`, so a rect already at the cap
/// shrinks by less than four when the level goes up. The cluster path
/// therefore lands one correction further out. It is measured here
/// rather than hidden because a bound nobody states is a bound nobody
/// notices growing.
#[test]
fn the_bias_reaches_its_value_in_one_step() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    for i in 0..24 {
        let x = (i as f32 - 12.0) * 0.35;
        add_point(&mut resources, Vec3::new(x, 0.2, -2.0), 20.0);
    }
    let eye = Vec3::ZERO;
    let view = Mat4::look_at_rh(eye, Vec3::NEG_Z, Vec3::Y);
    let proj = projection();
    let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);
    let mut lights = GpuLights::new(&device);
    let mut frame = kooch_lighting::LightFrame::extract(&resources);
    lights.update(&device, &queue, &resources, camera, None, &mut frame);
    let depth_view = depth_texture(&device, &queue, 0.02);
    let target = paint_target(&device);

    let mut series_for = |cluster: bool| {
        let mut marker = PageMarker::new(&device, PageConfig::default(), ClipmapConfig::default());
        marker.set_cluster_marking(cluster);
        marker.set_pool(
            &device,
            PoolConfig {
                pages: 12,
                views: 1,
                row_cap: u32::MAX,
            },
        );

        let mut series = Vec::new();
        for index in 0..8u32 {
            marker.set_frame(index);
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
                0,
                1,
                100,
                Paint {
                    target: &target,
                    on: false,
                    size: (SIZE, SIZE),
                },
            );
            queue.submit([encoder.finish()]);
            marker.poll();
            wait(&device);
            marker.poll();
            if let Some(counts) = marker.last() {
                series.push(counts.pool.bias_local);
            }
        }
        series
    };

    // `corrections` is how many times the bias moved up AFTER its first
    // move. Stepping one per frame would need `settled - 1` of them; the
    // point of computing the fit is that this number stays tiny however
    // deep the scene's answer is.
    let check = |series: Vec<u32>, corrections: usize, path: &str| {
        let settled = *series.last().expect("counters came back");
        assert!(
            settled > 1,
            "{path}: this scene does not need a multi-step bias, so it \
             cannot show one being reached in a step: settled at +{settled}"
        );
        // 🔴 The property, and it is deliberately not exactness. The
        // raise uses the OPTIMISTIC estimate — four pages become one per
        // level — because raising too little costs a frame of denials
        // while raising too much costs blur the player sees. So it lands
        // at or under the answer and corrects, never climbing through it.
        let first_move = series
            .iter()
            .copied()
            .find(|&b| b > 0)
            .expect("the bias never rose");
        assert!(
            first_move + corrections as u32 >= settled,
            "{path}: the first move was +{first_move} against a settled \
             +{settled}: {series:?} — that is stepping, not computing"
        );
        let rises = series.windows(2).filter(|w| w[1] > w[0]).count();
        assert!(
            rises <= corrections,
            "{path}: the bias rose {rises} times after its first move: \
             {series:?} — a correction is the estimate erring low, a climb \
             is stepping"
        );
        // Whatever it took to get there, it must be strictly better than
        // the one-step-a-frame rule this replaced.
        assert!(
            rises < settled as usize,
            "{path}: {rises} rises to reach +{settled} is the per-frame \
             step it was supposed to replace: {series:?}"
        );
        settled
    };

    let pixels = check(series_for(false), 1, "per pixel");
    // One further out, and the cap on a face's rect is why — see the
    // header. Same answer, one more frame to reach it.
    let clusters = check(series_for(true), 2, "per cluster");
    assert_eq!(
        pixels, clusters,
        "the two marking paths settled on different resolutions for one \
         scene, which is a difference in what they MARK, not in how fast \
         the bias converges"
    );
}

/// A dilated request asks for the neighbouring pages too.
///
/// # 🔴 What it buys is a FRAME
///
/// `vbuf64.render` rasterises and shades in one pass, so the atlas the
/// shading samples is a frame old. A page allocated this frame is read
/// with whatever its slot held last frame — cleared, which is far depth
/// under reversed-Z, which every reader answers "nothing occludes
/// here". One lit frame every time a page turns over, and standing on a
/// clipmap level boundary turns them over continuously: that is where
/// it was reported from, a player sitting exactly on the line where the
/// resolution changes.
///
/// The halo asks for the page before the camera reaches it, so the
/// content is there by the time anything samples it. Epic's
/// `PageDilationOffset` in `PageMarking.ush` does the same thing, and
/// theirs is a mitigation on top of a pipeline that has no such
/// latency at all.
///
/// Both bounds matter. It must grow — a halo that asked for nothing new
/// would buy no frame — and it must stay well under three times, or
/// every receiver really is paying for three pages and the pool pays
/// for all of them.
#[test]
fn a_halo_asks_for_the_neighbours() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let resources = world();
    let sun = Some(Vec3::new(0.2, -1.0, 0.3));

    HALO.with(|h| h.set(0.0));
    let plain = run(&device, &queue, &resources, 0.5, sun);
    HALO.with(|h| h.set(0.5));
    let dilated = run(&device, &queue, &resources, 0.5, sun);
    HALO.with(|h| h.set(0.0));

    assert!(plain.resident > 0, "the rig marked nothing to dilate");
    assert!(
        dilated.resident > plain.resident,
        "the halo asked for nothing new: {} against {}",
        dilated.resident,
        plain.resident,
    );
    assert!(
        dilated.resident < plain.resident * 3,
        "the halo cost the full 3x — {} against {} — so no two neighbours ever \
         collapsed onto one page and the offset is far larger than a page",
        dilated.resident,
        plain.resident,
    );
}

/// The dilation direction has to vary per thread.
///
/// # 🔴 A fixed diagonal is three quarters of a halo missing
///
/// The offset is applied as `±step`, so one diagonal covers two of the
/// eight neighbours. Epic vary which diagonal on a bit of the thread's
/// own index — `PageDilationDither` — and their comment says why that
/// is enough: *"as long as there's at least a single pixel near the
/// edge the adjacent one will get mapped"*. Over a 2x2 block all four
/// diagonals are asked for, at two marks per pixel instead of eight.
///
/// A constant direction still marks two pages per pixel, still raises
/// `resident`, and still passes a test that only counts. It just never
/// asks for the pages on the other three sides — so the halo does
/// nothing at all on the side the camera happens to be moving towards,
/// which is the only side that matters.
///
/// ⚠️ And it has to come off the THREAD index, not the pixel: `pixel`
/// is `id.xy * rate`, so at any even rate every thread shares a parity
/// and the pattern collapses back to the constant it replaced.
#[test]
fn the_dilation_picks_a_diagonal_per_thread() {
    let source = include_str!("../shaders/page_mark.wgsl");
    let dense: String = source.chars().filter(|c| !c.is_whitespace()).collect();

    assert!(
        dense.contains("select(-1.0,1.0,(id.x&1u)!=0u)")
            && dense.contains("select(-1.0,1.0,(id.y&1u)!=0u)"),
        "the dilation offset is not dithered off the thread index, so every pixel dilates \
         the same way and three of the four diagonals are never requested",
    );
    assert!(
        !dense.contains("(basis[0]+basis[1])*(halo*width)"),
        "the fixed diagonal is back",
    );
}
