//! The froxel grid, against a real device (#780).
//!
//! What these assert is the thing the shading loop cannot check for
//! itself: that a light which reaches a pixel is in that pixel's cell,
//! and that the two rasterizer passes agree about it. A grid that comes
//! out empty renders a scene lit by nothing but ambient — which looks
//! like a lighting bug, three layers away from here.

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

const VIEWPORT: glam::Vec2 = glam::Vec2::new(256.0, 256.0);

/// Words per cell: an offset and five type counts, then two of padding.
const CELL_WORDS: usize = 8;

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
        label: Some("cluster_grid_test_device"),
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

/// A point light at `position` that reaches `range` metres.
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

/// A camera at the origin looking down -Z, the way the grid's near end
/// is easiest to reason about.
fn camera() -> ClusterCamera {
    let eye = Vec3::ZERO;
    let view = Mat4::look_at_rh(eye, Vec3::NEG_Z, Vec3::Y);
    let proj = kooch_render_projection();
    ClusterCamera::new(eye, view, proj, VIEWPORT)
}

/// The same reversed-Z infinite projection the renderer uses (ADR 0002),
/// rebuilt here because `kooch_render` depends on this crate and not the
/// other way round.
fn kooch_render_projection() -> Mat4 {
    let fov_y: f32 = std::f32::consts::FRAC_PI_3;
    let near = 0.1_f32;
    let f = 1.0 / (fov_y * 0.5).tan();
    Mat4::from_cols(
        glam::Vec4::new(f, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, f, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, 0.0, -1.0),
        glam::Vec4::new(0.0, 0.0, near, 0.0),
    )
}

/// Builds the grid for one frame and returns (cells, indices).
fn build(
    resources: &Resources,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (Vec<u32>, Vec<u32>) {
    build_with(resources, device, queue, camera())
}

/// The same, for a scene that needs its own camera.
fn build_with(
    resources: &Resources,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    camera: ClusterCamera,
) -> (Vec<u32>, Vec<u32>) {
    let mut lights = GpuLights::new(device);
    let mut frame = kooch_lighting::LightFrame::extract(resources);
    lights.update(device, queue, resources, camera, None, &mut frame);

    let mut encoder = device.create_command_encoder(&Default::default());
    lights.record_clusters(&mut encoder);

    let cells = read_buffer(device, &mut encoder, lights.clusters().cells());
    let indices = read_buffer(device, &mut encoder, lights.clusters().indices());
    queue.submit([encoder.finish()]);
    wait(device);

    (map(device, &cells), map(device, &indices))
}

fn read_buffer(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    source: &wgpu::Buffer,
) -> wgpu::Buffer {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cluster_test_readback"),
        size: source.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(source, 0, &staging, 0, source.size());
    staging
}

/// Blocks until the GPU has caught up.
fn wait(device: &wgpu::Device) {
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
}

fn map(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u32> {
    let (tx, rx) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    wait(device);
    rx.recv().unwrap().unwrap();
    let view = buffer.slice(..).get_mapped_range();
    let words = bytemuck::cast_slice::<_, u32>(&view).to_vec();
    drop(view);
    buffer.unmap();
    words
}

/// How many cells hold at least one light.
fn occupied(cells: &[u32]) -> usize {
    cells
        .chunks(CELL_WORDS)
        .filter(|c| c[1] + c[2] + c[3] + c[4] + c[5] > 0)
        .count()
}

#[test]
fn a_light_in_front_of_the_camera_lands_in_cells() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 8.0);

    let (cells, _) = build(&resources, &device, &queue);
    assert!(
        occupied(&cells) > 0,
        "a light ten metres down the view axis reached no cell at all — \
         the grid is empty and every pixel would be lit by ambient alone",
    );
}

#[test]
fn a_light_behind_the_camera_lands_nowhere() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut resources = world();
    // Behind the eye and well short of it: nothing it lights is on
    // screen. The projection flips both axes back there, which is what
    // the near-plane clamp in `cluster_sphere_ndc` is for.
    add_point(&mut resources, Vec3::new(0.0, 0.0, 30.0), 5.0);

    let (cells, _) = build(&resources, &device, &queue);
    assert_eq!(occupied(&cells), 0);
}

/// 🔴 The assertion #780 turns on: what the count pass counted is what
/// the populate pass wrote.
///
/// The two are the same source compiled twice, and nothing else in the
/// pipeline can notice when they disagree — a cell would either overflow
/// its run into its neighbour's or leave a hole in it, and both render
/// as lighting that is subtly wrong.
#[test]
fn the_counts_and_the_indices_agree() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(-3.0, 0.0, -10.0), 6.0);
    add_point(&mut resources, Vec3::new(3.0, 1.0, -14.0), 6.0);
    add_point(&mut resources, Vec3::new(0.0, -2.0, -25.0), 10.0);

    let (cells, indices) = build(&resources, &device, &queue);
    assert!(
        occupied(&cells) > 0,
        "no cell holds any of the three lights"
    );

    for (cell, record) in cells.chunks(CELL_WORDS).enumerate() {
        let offset = record[0] as usize;
        let total: u32 = record[1..6].iter().sum();
        for slot in offset..offset + total as usize {
            let light = indices[slot];
            assert!(
                light < 3,
                "cell {cell} names light {light}, and the scene has three — \
                 the run either overflowed or was never written",
            );
        }
    }
}

/// A light whose range does not reach the camera's view still occupies
/// the cells it does reach, and no more.
#[test]
fn a_distant_light_does_not_fill_the_grid() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut near = world();
    add_point(&mut near, Vec3::new(0.0, 0.0, -10.0), 3.0);
    let (small, _) = build(&near, &device, &queue);

    let mut wide = world();
    add_point(&mut wide, Vec3::new(0.0, 0.0, -10.0), 30.0);
    let (large, _) = build(&wide, &device, &queue);

    assert!(
        occupied(&large) > occupied(&small),
        "a light with ten times the range touched no more cells ({} vs {}) — \
         the grid is not culling by volume, it is culling by nothing",
        occupied(&large),
        occupied(&small),
    );
}

/// A spot light gets a second, tighter test than a point: its sphere is
/// its range, and the cone inside that sphere is most of what the grid
/// saves. The test is that the cone does not cull away the cells it
/// actually lights.
#[test]
fn a_spot_reaches_the_cells_in_its_cone() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut resources = world();
    add_spot(&mut resources, Vec3::new(0.0, 6.0, -10.0), 40.0);

    let (cells, _) = build(&resources, &device, &queue);
    assert!(
        occupied(&cells) > 0,
        "a spot pointing down at the middle of the view reached no cell — \
         the cone test is culling everything, and the scene renders with \
         ambient light only",
    );
}

/// A spot at `position` pointing straight down.
fn add_spot(resources: &mut Resources, position: Vec3, range: f32) {
    let mut commands = resources.remove::<Commands>().unwrap();
    let entity = commands.spawn(resources).id();
    commands.apply(resources);
    commands
        .entity(entity)
        // Local -Z is where a light points, so pointing down is a
        // quarter turn about X.
        .insert(GlobalTransform {
            matrix: Mat4::from_rotation_translation(
                glam::Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
                position,
            ),
        })
        .insert(SpotLight {
            active: true,
            color: Vec3::ONE,
            intensity: 1_000_000.0,
            range,
            inner_angle: 25.0,
            outer_angle: 40.0,
            ..Default::default()
        });
    commands.apply(resources);
    resources.insert(commands);
}

/// 🔴 The assertion that matters to a pixel.
///
/// "Some cell holds the light" is not the property shading depends on:
/// what it needs is that the cell **this fragment lands in** holds the
/// lights that reach it. A grid can be full and still miss the one cell
/// being looked at — which renders as a surface lit by ambient alone,
/// with a light sitting right on top of it.
#[test]
fn the_cell_a_lit_point_falls_in_holds_the_light() {
    let Some((device, queue)) = device() else {
        return;
    };
    // A light three metres above a point on the floor, reaching well
    // past it.
    let lit_point = Vec3::new(0.0, 0.0, -12.0);
    let mut resources = world();
    add_point(&mut resources, lit_point + Vec3::Y * 3.0, 20.0);

    let (cells, indices) = build(&resources, &device, &queue);
    let cell = cell_of(lit_point);
    let record = &cells[cell * CELL_WORDS..(cell + 1) * CELL_WORDS];
    let total: u32 = record[1..6].iter().sum();
    assert!(
        total > 0,
        "the cell at {cell} — the one the shaded point falls in — holds no \
         lights, while a light three metres above it reaches twenty. That \
         point renders lit by ambient only.",
    );
    let run = record[0] as usize..record[0] as usize + total as usize;
    assert!(indices[run].contains(&0), "the cell names some other light");
}

/// The cell a world position falls in, computed the way the shading
/// model does it: project to pixels, scale to the grid, slice by view
/// depth.
fn cell_of(world: Vec3) -> usize {
    cell_of_with(world, camera())
}

fn cell_of_with(world: Vec3, cam: ClusterCamera) -> usize {
    let (view, proj) = cam.matrices.unwrap();
    let grid = kooch_lighting::ClusterGrid::new(&Default::default(), VIEWPORT);

    let clip = proj * view * world.extend(1.0);
    let ndc = clip.truncate() / clip.w;
    let frag = glam::Vec2::new(
        (ndc.x * 0.5 + 0.5) * VIEWPORT.x,
        (0.5 - ndc.y * 0.5) * VIEWPORT.y,
    );
    let xy = (frag * grid.tile_factors).floor();
    let z = grid.z_slice((view * world.extend(1.0)).z);
    let dims = grid.dimensions;
    ((xy.y as u32 * dims.x + xy.x as u32) * dims.z + z) as usize
}

/// The `spot_shadows` scene, at the level of the grid.
///
/// That suite renders a cube over a floor with one spot and measures the
/// floor. It went dark the day clustering landed, which said the grid
/// was missing the very cell being measured — and none of the tests
/// above could see it, because they all used a camera looking straight
/// down its own axis.
#[test]
fn the_spot_shadows_scene_lights_its_floor() {
    let Some((device, queue)) = device() else {
        return;
    };
    let eye = Vec3::new(0.0, 4.0, 9.0);
    let view = Mat4::look_at_rh(eye, Vec3::new(0.0, 0.5, 0.0), Vec3::Y);
    let cam = ClusterCamera::new(eye, view, kooch_render_projection(), VIEWPORT);

    let mut resources = world();
    // The spot of that suite: above and to the side, pointed at the
    // origin, reaching forty metres.
    let position = Vec3::new(3.0, 6.0, 0.0);
    let aim = (Vec3::new(0.0, 1.5, 0.0) - position).normalize();
    let mut commands = resources.remove::<Commands>().unwrap();
    let entity = commands.spawn(&mut resources).id();
    commands.apply(&mut resources);
    commands
        .entity(entity)
        .insert(GlobalTransform {
            matrix: Mat4::from_rotation_translation(
                glam::Quat::from_rotation_arc(Vec3::NEG_Z, aim),
                position,
            ),
        })
        .insert(SpotLight {
            active: true,
            color: Vec3::ONE,
            intensity: 4_000_000.0,
            range: 40.0,
            inner_angle: 25.0,
            outer_angle: 40.0,
            ..Default::default()
        });
    commands.apply(&mut resources);
    resources.insert(commands);

    let (cells, _) = build_with(&resources, &device, &queue, cam);
    // Where that suite measures: the cube's shadow on the floor.
    let measured = Vec3::new(-1.0, 0.0, 0.0);
    let cell = cell_of_with(measured, cam);
    let record = &cells[cell * CELL_WORDS..(cell + 1) * CELL_WORDS];
    let total: u32 = record[1..6].iter().sum();
    assert!(
        total > 0,
        "the floor point the spot lights falls in cell {cell}, and that cell \
         holds no lights — which is exactly what `spot_shadows` sees as a \
         floor lit by ambient alone",
    );
}
