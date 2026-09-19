//! A lamp's page holds what its light sees, on every cull path and layer.

use super::*;

/// One lamp over a floor and a box, through the REAL pipeline: planted table -> per-level culls ->
/// compaction -> expansion -> draw, then the atlas texels are read back and checked against what
/// the light actually sees.
#[test]
fn a_lamp_page_holds_what_its_light_sees() {
    lamp_page_holds_its_view(false, small(), 7, coarse_level());
}

#[test]
fn the_two_level_cull_draws_the_same_page() {
    lamp_page_holds_its_view(true, small(), 7, coarse_level());
}

/// The TOP of a lamp's chain — one page for the whole cube face, which is the only page a distant
/// light gets (#1009).
#[test]
fn the_chain_top_draws() {
    let config = PageConfig::default();
    let top = config.levels() - 1;
    assert_eq!(
        config.side(top),
        1,
        "the top of the chain is not a single page"
    );
    lamp_page_holds_its_view(false, small(), 7, top);
}

/// 🔴 The acceptance for #1016: the SAME page, read back from a pool whose view spans two layers,
/// with the coarse page living in the second one.
#[test]
fn a_page_on_the_far_layer_draws_the_same() {
    // 64 pages across two views is 32 each; a cap of four pages a row
    // makes a layer hold 16, so each view needs two.
    let split = small().fit_atlas(4 * PageConfig::default().page, PageConfig::default().page);
    assert_eq!(split.slice(), 16, "a layer holds sixteen pages");
    assert_eq!(split.layers_per_view(), 2, "so a view needs two layers");
    // Slot 20 is page 4 of layer 1 — the far layer, and view 0's.
    assert_eq!(20 / split.slice(), 1, "the coarse page is on layer one");
    lamp_page_holds_its_view(true, split, 20, coarse_level());
}

/// Three levels above the chain's floor: coarse enough to pair against a coarse bucket's survivors,
/// fine enough that its cell is a window rather than the whole face.
fn coarse_level() -> u32 {
    PageConfig::default().local_floor() + 3
}

fn lamp_page_holds_its_view(
    two_level: bool,
    budget: PoolConfig,
    coarse_slot: u32,
    coarse_level: u32,
) {
    use glam::{Mat4, Vec3};
    use kooch_render::meshlet::{
        MeshInstance, MeshletCullPipelines, MeshletScene, SceneCullParams, build_default_meshlets,
    };

    // The cull pipeline binds 5 groups and 9 storage buffers, past the default limits — the shared
    // helper mirrors the production GpuContext, where this file's own `device()` does not.
    let Some((device, queue)) = common::try_acquire_device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let (device, queue) = (&device, &queue);
    let device = device.clone();
    let queue = queue.clone();

    // The scene: a lamp 4 m up, a 40 m floor whose top is y = 0, and a half-metre box hanging at
    // (0.45, 2, -0.45) — inside the window of face 3's cell (8, 8) but covering only part of it, so
    // the page must hold BOTH populations: box depth and floor depth.
    let mesh = kooch_render::mesh::primitives::Primitive::Cube {
        half_extents: Vec3::splat(0.5),
    }
    .build();
    let meshlet_mesh = build_default_meshlets(&mesh).expect("cube builds");
    let mut pool = kooch_render::meshlet::GlobalMeshPool::new();
    let handle = pool.register(&meshlet_mesh);
    let gpu_pool = pool.upload(&device);
    let meshlets_per_mesh = gpu_pool.max_meshlets_per_mesh.max(1);

    let instances = vec![
        // The floor: top face at y = 0.
        MeshInstance::new(
            Mat4::from_scale_rotation_translation(
                Vec3::new(40.0, 1.0, 40.0),
                glam::Quat::IDENTITY,
                Vec3::new(0.0, -0.5, 0.0),
            ),
            handle.mesh_id,
            0,
        ),
        // The occluder: spans [0.2, 0.7] x [1.75, 2.25] x [-0.7, -0.2].
        MeshInstance::new(
            Mat4::from_scale_rotation_translation(
                Vec3::splat(0.5),
                glam::Quat::IDENTITY,
                Vec3::new(0.45, 2.0, -0.45),
            ),
            handle.mesh_id,
            0,
        ),
    ];
    let scene = MeshletScene::new(&device, instances.len() as u32);
    scene.upload_instances(&queue, &instances);
    let scene_params = SceneCullParams::new(instances.len() as u32, meshlets_per_mesh);

    const LIGHTS: u32 = 1;
    let lamp = Vec3::new(0.0, 4.0, 0.0);
    let range = 20.0_f32;
    // Two lamps: the one under test, and one whose range reaches no
    // instance at all — the hierarchical cull's pre-pass must leave the
    // second one's survivor slice empty (#939's acceptance).
    let records = [
        kooch_lighting::GpuLight {
            position: lamp.to_array(),
            range,
            kind: 1,
            ..Default::default()
        },
        kooch_lighting::GpuLight {
            position: [100.0, 4.0, 100.0],
            range: 5.0,
            kind: 1,
            ..Default::default()
        },
    ];
    let lights_buffer = {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lamp_page_test_light"),
            size: std::mem::size_of_val(&records) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&records));
        buffer
    };

    // Plant the lamp's pages: face 3 (-Y, toward the floor). The fine page is the chain's floor;
    // the coarse one two levels up, whose octave lands in a coarse clipmap bucket.
    let config = PageConfig::default();
    let fine_level = config.local_floor();
    let fine_side = config.side(fine_level);
    let fine = lamp_face_page(0, 3, fine_level, (fine_side / 2, fine_side / 2), LIGHTS);
    // The cell under the lamp, whatever the level's grid is. At the top
    // of the chain that grid is one page and this is (0, 0).
    let coarse_side = config.side(coarse_level);
    let coarse_cell = (coarse_side / 2, coarse_side / 2);
    let coarse = lamp_face_page(0, 3, coarse_level, coarse_cell, LIGHTS);

    let mut page_pool = PagePool::new(&device, budget);
    let entries = VIEWS * span(LIGHTS);
    page_pool.ensure_entries(&device, entries);
    let cell = PAGE_CELL as usize;
    let mut slots = vec![0u32; entries as usize * cell];
    // 🔴 FOUR, not three, and that is the whole of what makes the far-layer case detectable: with a
    // layer of sixteen pages, slot 4 and slot 20 are the SAME rect of different layers.
    const FINE_SLOT: u32 = 4;
    let coarse_slot = coarse_slot;
    slots[fine as usize * cell] = FINE_SLOT + 1;
    slots[coarse as usize * cell] = coarse_slot + 1;
    queue.write_buffer(page_pool.slots(), 0, bytemuck::cast_slice(&slots));

    // Built the way the FRAME builds it — against the cull pipelines' meshlet layout, which is what
    // the depth pipeline's group(1) expects. The `rasterizer()` helper hands the pool's own layout,
    // which no test had ever exercised a draw through.
    let cull_pipelines = MeshletCullPipelines::new(&device);
    let mut raster = PageRasterizer::new(
        &device,
        cull_pipelines.meshlet_bind_group_layout(),
        PageConfig::default(),
        ClipmapConfig::default(),
        budget,
        kooch_render::meshlet::DEFAULT_MAX_TRIANGLES as u32,
    );
    let meshlet_bg = kooch_render::meshlet::pool_meshlet_bind_group(
        &device,
        cull_pipelines.meshlet_bind_group_layout(),
        &gpu_pool,
    );
    raster.set_two_level(two_level);
    let threads = instances.len() as u32 * meshlets_per_mesh;
    let chunks = kooch_render::meshlet::chunks_for(instances.len() as u32, meshlets_per_mesh);
    raster.ensure_capacity(&device, threads, threads, chunks);

    let mut encoder = device.create_command_encoder(&Default::default());
    raster.record(
        &device,
        &queue,
        &mut encoder,
        &cull_pipelines,
        &gpu_pool,
        &scene,
        &meshlet_bg,
        scene.instance_buffer(),
        &page_pool,
        &scene_params,
        0,
        Vec3::new(0.0, 1.0, 8.0),
        Vec3::NEG_Y,
        &records,
        &lights_buffer,
        &[],
        1.0,
        None,
        None,
    );
    queue.submit([encoder.finish()]);

    // The bucketing half: a lamp's pages — every level of its chain — land in ITS bucket, after the
    // sun's levels, where its own cull's survivors are bound.
    let counts = read_words(&device, &queue, raster.counts_buffer());
    let clipmap = ClipmapConfig::default();
    let lamp_bucket = clipmap.levels as usize;
    assert_eq!(
        counts[lamp_bucket],
        2,
        "lamp 0's bucket does not hold its two pages: {:?}",
        &counts[..lamp_bucket + 2]
    );
    let strays: u32 = counts[..lamp_bucket].iter().sum();
    assert_eq!(
        strays,
        0,
        "a lamp page strayed into the sun's buckets: {:?}",
        &counts[..lamp_bucket]
    );
    // The survivors mirror: lamp 0's cull found the floor and the box, and the out-of-range lamp's
    // slice is EMPTY — its light sphere touches no instance, so the pre-pass never let it reach the
    // meshlet domain.
    let buckets = raster.buckets() as usize;
    let survivors = |bucket: usize| counts[buckets + 5 + bucket];
    assert!(
        survivors(lamp_bucket) > 0,
        "the lamp under test culled no survivors at all"
    );
    assert_eq!(
        survivors(lamp_bucket + 1),
        0,
        "a lamp whose range reaches nothing kept survivors: {}",
        survivors(lamp_bucket + 1)
    );

    // What the light sees, by construction: the floor at 4 m stores `PAGE_NEAR / 4`; the box's lit
    // surfaces sit between 1.75 and 2.25 m. Reversed depth, so the box is the LARGER value.
    let floor_depth = 0.05 / 4.0;
    let page = config.page;
    let read_page =
        |slot: u32| -> Vec<f32> { read_atlas_page(&device, &queue, &raster, budget, slot, page) };

    for (name, slot, min_box, max_box) in [
        ("fine", FINE_SLOT, 0.005, 0.30),
        ("coarse", coarse_slot, 0.002, 0.40),
    ] {
        let texels = read_page(slot);
        let total = texels.len() as f32;
        let empty = texels.iter().filter(|d| **d == 0.0).count() as f32 / total;
        let floor = texels
            .iter()
            .filter(|d| (**d - floor_depth).abs() < 0.002)
            .count() as f32
            / total;
        let boxed = texels.iter().filter(|d| **d > 0.019).count() as f32 / total;
        let absurd = texels.iter().filter(|d| **d > 0.04).count();

        // 1. The projection covers the page: the floor spans the whole
        //    cell window, so an empty texel is geometry that missed its
        //    page — the misprojection class of defect.
        assert!(
            empty < 0.02,
            "{name}: {:.1}% of the page was never drawn",
            empty * 100.0
        );
        // 2. Both populations, in believable shares.
        assert!(
            floor > 0.5,
            "{name}: the floor covers {:.1}% of the page; the projection is off",
            floor * 100.0
        );
        assert!(
            boxed > min_box && boxed < max_box,
            "{name}: the box occludes {:.1}% of the page, outside [{:.1}%, {:.1}%]",
            boxed * 100.0,
            min_box * 100.0,
            max_box * 100.0
        );
        // 3. Nothing is closer to the lamp than the box's top.
        assert_eq!(
            absurd, 0,
            "{name}: {absurd} texels claim depth nearer than anything in the scene"
        );
    }

    // ---- The cache: a second frame with nothing changed draws NOTHING. The stamps written by the
    // first compaction match their lamp's generation, so both pages are cached, no page is listed,
    // the depth pass clears no quad — and the atlas still holds the scene.
    let mut encoder = device.create_command_encoder(&Default::default());
    raster.record(
        &device,
        &queue,
        &mut encoder,
        &cull_pipelines,
        &gpu_pool,
        &scene,
        &meshlet_bg,
        scene.instance_buffer(),
        &page_pool,
        &scene_params,
        0,
        Vec3::new(0.0, 1.0, 8.0),
        Vec3::NEG_Y,
        &records,
        &lights_buffer,
        &[],
        1.0,
        None,
        None,
    );
    queue.submit([encoder.finish()]);
    let counts = read_words(&device, &queue, raster.counts_buffer());
    assert_eq!(
        counts[lamp_bucket], 0,
        "an unchanged frame listed pages the cache should have kept"
    );
    assert_eq!(
        counts[buckets + 4],
        2,
        "the cached counter does not carry both pages: {:?}",
        &counts[buckets..buckets + 5]
    );
    let texels = read_atlas_page(&device, &queue, &raster, budget, FINE_SLOT, page);
    let floor = texels
        .iter()
        .filter(|d| (**d - floor_depth).abs() < 0.002)
        .count() as f32
        / texels.len() as f32;
    assert!(
        floor > 0.5,
        "a cached page lost its content: the floor covers {:.1}%",
        floor * 100.0
    );

    // ---- Invalidation: the occluder "moves" — its old bounds arrive as a moved sphere — and every
    // page its lamp can reach redraws. Per-light granularity: both pages of lamp 0 come back.
    raster.set_frame(1);
    let mut encoder = device.create_command_encoder(&Default::default());
    raster.record(
        &device,
        &queue,
        &mut encoder,
        &cull_pipelines,
        &gpu_pool,
        &scene,
        &meshlet_bg,
        scene.instance_buffer(),
        &page_pool,
        &scene_params,
        0,
        Vec3::new(0.0, 1.0, 8.0),
        Vec3::NEG_Y,
        &records,
        &lights_buffer,
        &[[0.45, 2.0, -0.45, 0.5]],
        1.0,
        None,
        None,
    );
    queue.submit([encoder.finish()]);
    let counts = read_words(&device, &queue, raster.counts_buffer());
    assert_eq!(
        counts[lamp_bucket],
        2,
        "a moved caster did not bring its lamp's pages back: {:?}",
        &counts[..lamp_bucket + 2]
    );
    assert_eq!(
        counts[buckets + 4],
        0,
        "pages stayed cached across an invalidation"
    );
    // And the redraw reproduces the scene.
    let texels = read_atlas_page(&device, &queue, &raster, budget, FINE_SLOT, page);
    let floor = texels
        .iter()
        .filter(|d| (**d - floor_depth).abs() < 0.002)
        .count() as f32
        / texels.len() as f32;
    assert!(
        floor > 0.5,
        "the invalidated redraw lost the floor: {:.1}%",
        floor * 100.0
    );
}

/// One page of the atlas, as f32 depths.
fn read_atlas_page(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    raster: &PageRasterizer,
    pool: PoolConfig,
    slot: u32,
    page: u32,
) -> Vec<f32> {
    let side = pool.per_row() * page;
    let origin_x = (slot % pool.per_row()) * page;
    let origin_y = (slot / pool.per_row() % pool.per_row()) * page;
    let layer = slot / pool.slice();
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("atlas_page_readback"),
        size: (side * side * 4) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: raster.atlas_texture(),
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(side * 4),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    let whole = bytemuck::cast_slice::<u8, f32>(&staging.slice(..).get_mapped_range()).to_vec();
    staging.unmap();
    let mut out = Vec::with_capacity((page * page) as usize);
    for row in 0..page {
        let at = ((origin_y + row) * side + origin_x) as usize;
        out.extend_from_slice(&whole[at..at + page as usize]);
    }
    out
}
