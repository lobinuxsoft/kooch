//! GPU acceptance: Hi-Z 2-pass cull (#445).

mod common;

use common::{build_cube_mesh, try_acquire_device};
use glam::{Mat4, Vec3};
use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::registry::ComponentRegistry;
use kooch_ecs::hierarchy::global_transform::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_ecs::query::AccessTracker;
use kooch_render::material::{Material, MaterialPipeline};
use kooch_render::meshlet::{MeshletRenderStage, MeshletRenderStageConfig, build_default_meshlets};

fn install_material_pipeline(
    resources: &mut Resources,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    material: Material,
) {
    let mut pipeline = MaterialPipeline::with_capacity(device, queue, 4);
    pipeline.register(queue, Guid::new_v4(), &material);
    resources.insert(pipeline);
}

fn ecs_test_resources() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r
}

fn read_u32(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer) -> u32 {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("hi_z_two_pass_u32_staging"),
        size: 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("hi_z_two_pass_u32_readback"),
    });
    enc.copy_buffer_to_buffer(buffer, 0, &staging, 0, 4);
    queue.submit(std::iter::once(enc.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    rx.recv().unwrap().unwrap();
    let bytes = slice.get_mapped_range().to_vec();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

#[test]
fn single_frame_first_render_does_not_crash_with_empty_prev_pyramid() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };

    let cube = build_cube_mesh();
    let cube_meshlets = build_default_meshlets(&cube).expect("build meshlets");
    let cube_guid = Guid::new_v4();

    let mut stage = MeshletRenderStage::new(
        &device,
        MeshletRenderStageConfig {
            size: (256, 256),
            instance_capacity: 8,
            meshlet_capacity: 4096,
            ..Default::default()
        },
    );
    stage.ensure_gpu_mesh(&device, cube_guid, &cube_meshlets);

    let mut resources = ecs_test_resources();
    install_material_pipeline(
        &mut resources,
        &device,
        &queue,
        Material::new([1.0, 1.0, 1.0, 1.0], 0.0, 0.5, 0.0),
    );
    let mut commands = Commands::new();
    commands
        .spawn(&mut resources)
        .insert(MeshRenderer {
            mesh: Some(cube_guid),
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(Vec3::new(0.0, 0.0, 0.0)),
        });
    commands.apply(&mut resources);

    let camera = kooch_render::ViewCamera::looking_at(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO);
    let stats = stage.render_with_assets_primary(&device, &queue, &resources, &camera, 1.0);
    assert_eq!(
        stats.instances_uploaded, 1,
        "single visible cube must upload"
    );
    assert_eq!(
        stats.draw_calls, 6,
        "Hi-Z 2-pass orchestrator (post-#488 with AABB cull + reversed-Z) \
         should report 6 logical passes per frame (cull A + raster A + SPD \
         pyramid build + cull B + raster B + deferred shade)."
    );

    // First frame: hiz_prev is fresh / uninitialised.
    let visible = read_u32(&device, &queue, stage.cull().visible_count_buffer());
    let culled = read_u32(&device, &queue, stage.cull().culled_count_buffer());
    assert!(
        visible > 0,
        "first frame must produce visible meshlets, got visible={visible}"
    );
    assert_eq!(
        culled, 0,
        "first-frame Hi-Z has no signal yet; pass A should reject nothing, got culled={culled}"
    );
}

#[test]
fn two_pass_visible_set_stays_stable_across_frames_in_static_scene() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };

    let cube = build_cube_mesh();
    let cube_meshlets = build_default_meshlets(&cube).expect("build meshlets");
    let cube_guid = Guid::new_v4();

    let mut stage = MeshletRenderStage::new(
        &device,
        MeshletRenderStageConfig {
            size: (256, 256),
            instance_capacity: 64,
            // Each instance is a single-meshlet cube, but we still
            // reserve the worst-case stride headroom across the pool.
            meshlet_capacity: 4096,
            ..Default::default()
        },
    );
    stage.ensure_gpu_mesh(&device, cube_guid, &cube_meshlets);

    // "Wall" instance: a large flat cube near the camera that covers the screen, plus several
    // smaller cubes parked behind it.
    let mut resources = ecs_test_resources();
    install_material_pipeline(
        &mut resources,
        &device,
        &queue,
        Material::new([0.6, 0.6, 0.6, 1.0], 0.0, 0.5, 0.0),
    );
    let mut commands = Commands::new();

    // Wall: scaled cube at z = -2 (in front of the back row).
    // Scale then translate (matrix order: rightmost applied first).
    let wall = Mat4::from_translation(Vec3::new(0.0, 0.0, -2.0))
        * Mat4::from_scale(Vec3::new(8.0, 8.0, 0.5));
    commands
        .spawn(&mut resources)
        .insert(MeshRenderer {
            mesh: Some(cube_guid),
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform { matrix: wall });

    // Back row: cubes deeper into -Z (occluded by the wall).
    for i in 0..6 {
        let x = (i as f32 - 2.5) * 1.5;
        let m = Mat4::from_translation(Vec3::new(x, 0.0, -10.0));
        commands
            .spawn(&mut resources)
            .insert(MeshRenderer {
                mesh: Some(cube_guid),
                visible: true,
                ..Default::default()
            })
            .insert(GlobalTransform { matrix: m });
    }
    commands.apply(&mut resources);

    let camera =
        kooch_render::ViewCamera::looking_at(Vec3::new(0.0, 0.0, 4.0), Vec3::new(0.0, 0.0, -10.0));

    // Render N frames of a static scene.
    let mut counts = Vec::with_capacity(4);
    for _ in 0..4 {
        let _ = stage.render_with_assets_primary(&device, &queue, &resources, &camera, 1.0);
        let visible = read_u32(&device, &queue, stage.cull().visible_count_buffer());
        counts.push(visible);
    }
    // Frame 0 is the init transient: hiz_prev was just cleared to 0.0 (= far in reversed-Z), so
    // pass A's conservative test `aabb.max.z <= tile_min` rejects nothing and visible_count covers
    // the full instance set.
    assert!(
        counts[0] > 0,
        "first frame must produce visible meshlets, got 0 — orchestration broken"
    );
    let steady = counts[1];
    assert!(
        steady > 0,
        "steady-state frame 1 must still draw at least one meshlet, got 0 \
         (counts so far: {:?})",
        counts
    );
    for (i, &c) in counts.iter().enumerate().skip(2) {
        assert_eq!(
            c, steady,
            "frame {} visible_count {} diverged from steady-state frame 1 ({}); \
             Hi-Z 2-pass swap or pass-B retest is leaking instability into the \
             rendered set. Full counts: {:?}",
            i, c, steady, counts
        );
    }
}
