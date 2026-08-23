//! The paged depth raster (#866).
//!
//! The first test is the one that matters most: four shaders share
//! `page_table.wgsl`, and a page id encoded one way and decoded another
//! rasterises geometry into somebody else's page. A compile failure
//! belongs here rather than in a frame.

mod common;

use kooch_render::meshlet::GpuGlobalMeshPool;
use kooch_render::shadow::pages::pool::{PAGE_CELL, PagePool, PoolConfig};
use kooch_render::shadow::pages::raster::{PAGE_DEPTH_FORMAT, PAGE_FRONT_FACE, PageRasterizer};
use kooch_render::shadow::pages::{ClipmapConfig, PageConfig};

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("page_raster_test"),
        ..Default::default()
    }))
    .ok()
}

/// A pool small enough that the atlas is megabytes rather than a
/// quarter of a gigabyte: every test here is about arithmetic, not
/// about capacity.
fn small() -> PoolConfig {
    PoolConfig {
        pages: 64,
        views: VIEWS,
    }
}

/// Cameras the pool is sliced between. Two, because one is the case
/// that never showed the bug.
const VIEWS: u32 = 2;

fn rasterizer(device: &wgpu::Device) -> PageRasterizer {
    let bgl = GpuGlobalMeshPool::bind_group_layout(device);
    PageRasterizer::new(
        device,
        &bgl,
        PageConfig::default(),
        ClipmapConfig::default(),
        small(),
        // 🔴 The engine's own cap, not a round number. The test used to
        // pass 64 and the builder really emits meshlets of up to 98
        // triangles, so a helper configured for less would have hidden
        // the very thing `the_draw_covers_a_whole_meshlet` asserts.
        kooch_render::meshlet::DEFAULT_MAX_TRIANGLES as u32,
    )
}

#[test]
fn the_shaders_compile() {
    let Some((device, _queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    // Builds all four pipelines, so a WGSL mistake in any of the three
    // shaders or in the shared table fails here.
    let _ = rasterizer(&device);
}

#[test]
fn the_atlas_holds_the_pool() {
    let Some((device, _queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let raster = rasterizer(&device);
    let texture = raster.atlas_texture();
    assert_eq!(texture.format(), PAGE_DEPTH_FORMAT);
    let page = PageConfig::default().page;
    let across = texture.size().width / page;
    // A LAYER per camera, and the budget is the layers together: the
    // whole point of slicing is that two viewports cost what one did.
    assert_eq!(
        texture.size().depth_or_array_layers,
        VIEWS,
        "one layer per camera"
    );
    assert_eq!(across * across, small().slice(), "a layer is one slice");
    assert!(
        across * across * VIEWS >= small().pages,
        "{across} across on {VIEWS} layers cannot hold {}",
        small().pages
    );
    assert_eq!(
        texture.size().width,
        texture.size().height,
        "a strip wastes the second dimension of every texture limit"
    );
}

#[test]
fn the_counters_name_every_level() {
    let Some((device, _queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let raster = rasterizer(&device);
    let sun = ClipmapConfig::default().levels;
    let buckets = raster.buckets();
    // 🔴 Per BUCKET, and a bucket is a LOD rather than a light: the
    // sun's clipmap levels first, then the chain levels every local
    // shared by the sun and every lamp that wants that fineness. Then
    // bucket overflow, local pages, pairs, pair overflow, a retired
    // slot (it counted the other camera's pages when the compaction
    // walked the whole shared table) — and then a second run per bucket for the
    // survivors each cull produced, which is the other half of the
    // expansion's cost, and a third for the cells a scatter would have
    // visited instead.
    // 🔴 The same count as the clipmap's levels, and that IS the design:
    // a bucket is an octave of world texel size, anchored so the sun's
    // level L lands on bucket L. A lamp's pages fall into buckets the
    // sun's culls already fill, which is what makes the local half cost
    // no new dispatch.
    assert_eq!(
        buckets, sun,
        "buckets stopped being octaves of the sun's own scale; a lamp's pages now need \
         a cull of their own to fill whatever the extra buckets are"
    );
    assert_eq!(raster.count_slots(), buckets * 3 + 5);
    let mut words = vec![0u32; raster.count_slots() as usize];
    words[0] = 7;
    words[1] = 5;
    words[2] = 9;
    words[buckets as usize + 1] = 42;
    words[buckets as usize + 2] = 900;
    let counts = raster.decode(&words, 1);
    assert_eq!(counts.pages, 21, "every bucket sums");
    assert_eq!(counts.local, 42, "local pages are reported, not hidden");
    assert_eq!(counts.pairs, 900);
    assert_eq!(counts.view, 1);
}

/// Pages one LOCAL light addresses. Recomputed from the public config
/// rather than read off the marking pass, so the two derivations have
/// to agree: six faces of a chain from the floor up, on a word
/// boundary.
fn stride(config: PageConfig, _clipmap: ClipmapConfig) -> u32 {
    (config.local_face_pages() * 6).div_ceil(32) * 32
}

/// Light slots the address space is laid out for. Mirrors
/// `padded_lights`: the layout pads so adding a light does not move
/// every page id.
fn padded(lights: u32) -> u32 {
    lights.max(1).next_multiple_of(64)
}

/// Pages one camera addresses: the padded light slots, then the sun's
/// clipmap at the tail.
fn span(lights: u32) -> u32 {
    let config = PageConfig::default();
    let clipmap = ClipmapConfig::default();
    (padded(lights) * stride(config, clipmap) + clipmap.levels * config.side(0).pow(2)).div_ceil(32)
        * 32
}

/// The virtual page `mark_sun` would write for this camera, level and
/// cell.
fn sun_page(view: u32, level: u32, cell: (u32, u32), lights: u32) -> u32 {
    let config = PageConfig::default();
    let clipmap = ClipmapConfig::default();
    let side = config.side(0);
    view * span(lights)
        + padded(lights) * stride(config, clipmap)
        + level * side * side
        + cell.1 * side
        + cell.0
}

/// A page belonging to light 0, which this raster does not draw.
///
/// `level` has to be at or above the floor: the address space stops at
/// `local_floor` on the fine side, which is what made the flat table
/// affordable.
fn local_page(view: u32, level: u32, cell: (u32, u32), lights: u32) -> u32 {
    let config = PageConfig::default();
    assert!(level >= config.local_floor(), "below the addressable floor");
    let side = config.side(level);
    let base: u32 = (config.local_floor()..level)
        .map(|l| config.side(l).pow(2))
        .sum();
    view * span(lights) + base + cell.1 * side + cell.0
}

/// A lights buffer the compaction can read a `range` out of.
///
/// 🔴 It reads exactly one field, and it is the field that places a
/// lamp's pages on the same density scale as the sun's — see
/// `page_octave`. A buffer of the wrong stride reads somebody else's
/// float as a range and buckets the lamp somewhere plausible and wrong,
/// so this builds real `GpuLight` records rather than a flat array.
fn lights_buffer(device: &wgpu::Device, queue: &wgpu::Queue, ranges: &[f32]) -> wgpu::Buffer {
    let records: Vec<kooch_lighting::GpuLight> = ranges
        .iter()
        .map(|&range| kooch_lighting::GpuLight {
            range,
            kind: 1,
            ..Default::default()
        })
        .collect();
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("page_raster_test_lights"),
        size: (records.len().max(1) * std::mem::size_of::<kooch_lighting::GpuLight>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    if !records.is_empty() {
        queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&records));
    }
    buffer
}

fn read_words(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer) -> Vec<u32> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("raster_readback"),
        size: buffer.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, buffer.size());
    queue.submit([encoder.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    let words = bytemuck::cast_slice::<u8, u32>(&staging.slice(..).get_mapped_range()).to_vec();
    staging.unmap();
    words
}

#[test]
fn a_page_compacts_into_the_level_it_came_from() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let raster = rasterizer(&device);
    let pool = PagePool::new(&device, small());
    let levels = ClipmapConfig::default().levels;
    const LIGHTS: u32 = 1;

    // Camera 1's table, seen from camera 1: three sun pages on two
    // levels, one local page this raster does not draw, and two pages
    // belonging to the OTHER camera. The table is flat — the entry
    // index IS the page id and the first word is `slot + 1`.
    const VIEW: u32 = 1;
    // The finest addressable local level — the floor itself.
    const LOCAL_LEVEL: u32 = 3;
    let planted = [
        (sun_page(VIEW, 0, (3, 4), LIGHTS), 11u32),
        (sun_page(VIEW, 0, (5, 6), LIGHTS), 12),
        (sun_page(VIEW, 5, (7, 8), LIGHTS), 13),
    ];
    let entries = ((VIEW + 1) * span(LIGHTS)) as usize;
    let mut pool = pool;
    pool.ensure_entries(&device, entries as u32);
    let cell = PAGE_CELL as usize;
    let mut slots = vec![0u32; entries * cell];
    for (page, slot) in planted.iter() {
        slots[*page as usize * cell] = *slot + 1;
    }
    let local = local_page(VIEW, LOCAL_LEVEL, (1, 1), LIGHTS);
    slots[local as usize * cell] = 20 + 1;
    // 🔴 The other camera's pages, on levels this one also uses. The
    // dispatch covers only THIS view's span, so they are outside it —
    // and their listings have to come through untouched.
    let foreign = [
        sun_page(0, 0, (3, 4), LIGHTS),
        sun_page(0, 5, (7, 8), LIGHTS),
    ];
    for (i, page) in foreign.iter().enumerate() {
        slots[*page as usize * cell] = 30 + i as u32 + 1;
    }
    queue.write_buffer(pool.slots(), 0, bytemuck::cast_slice(&slots));

    let mut encoder = device.create_command_encoder(&Default::default());
    raster.record_compaction(
        &device,
        &queue,
        &mut encoder,
        &pool,
        VIEW,
        glam::Vec3::ZERO,
        glam::Vec3::NEG_Y,
        LIGHTS,
        &lights_buffer(&device, &queue, &[10.0]),
    );
    queue.submit([encoder.finish()]);

    let buckets = raster.buckets() as usize;
    let counts = read_words(&device, &queue, raster.counts_buffer());
    assert_eq!(counts[0], 2, "two pages on level 0");
    assert_eq!(counts[buckets], 0, "no bucket overflowed");
    assert_eq!(
        counts[buckets + 1],
        1,
        "the local light's page is counted, not silently dropped"
    );
    assert_eq!(
        counts[buckets + 4],
        0,
        "the other camera's pages are outside the dispatch, so the retired          counter stays zero"
    );
    // 🔴 And it LANDS somewhere — in one of the SUN'S buckets, because a
    // bucket is an octave of world texel size and a lamp that wants the
    // sun's fineness wants the sun's LOD. Four pages were planted for
    // this view and all four are listed; which bucket the lamp shares
    // depends on its range, which is exactly the point and exactly why
    // this does not assert an index.
    let listed: u32 = (0..levels as usize).map(|l| counts[l]).sum();
    assert_eq!(
        listed, 4,
        "three sun pages and one lamp page were planted; {listed} reached a bucket"
    );
    // The lamp did not simply land on top of a sun page: level 5 held
    // exactly one before and the lamp is not at level 5's density.
    assert!(
        counts[5] >= 1,
        "the sun's own level-5 page stopped being listed"
    );

    // The list is bucketed: level L owns `[L * bucket, (L+1) * bucket)`.
    let list = read_words(&device, &queue, raster.page_list_buffer());
    let bucket = small().slice() as usize;
    let level0: Vec<(u32, u32)> = (0..2).map(|i| (list[i * 2], list[i * 2 + 1])).collect();
    assert!(
        level0.contains(&planted[0]) && level0.contains(&planted[1]),
        "level 0 holds {level0:?}"
    );
    let at = 5 * bucket * 2;
    assert_eq!(
        (list[at], list[at + 1]),
        planted[2],
        "level 5's bucket holds its page and its physical slot"
    );

    // 🔴 And the way BACK. `page_list` is dense and per view, so a pass
    // that computes a page KEY — rather than reading one out of the list
    // — has no route to the entry the draw indexes by. The compaction is
    // the only pass holding both, so it writes the listing into the
    // table's third word. Without it, finding pages by walking cells
    // means walking every resident page to identify each one, which is
    // the pairing this was meant to replace.
    let cells = read_words(&device, &queue, pool.slots());
    let listing = |page: u32| cells[page as usize * cell + 2];
    for (planted_page, slot) in planted.iter() {
        let at = listing(*planted_page) as usize;
        assert_ne!(
            at as u32, PAGE_UNLISTED,
            "the sun page {planted_page} kept no listing"
        );
        assert_eq!(
            (list[at * 2], list[at * 2 + 1]),
            (*planted_page, *slot),
            "page {planted_page}'s listing points at the wrong page"
        );
    }
    // The local page is listed now, so it has a listing like any other.
    let local_at = listing(local) as usize;
    assert_ne!(
        local_at as u32, PAGE_UNLISTED,
        "the local page is bucketed but carries no listing"
    );
    assert_eq!(
        list[local_at * 2],
        local,
        "the local page's listing points somewhere else"
    );
    assert!(
        local_at < levels as usize * bucket,
        "the local page landed past every bucket"
    );

    // The other camera's entries are outside the dispatch, so whatever
    // they carried — a cleared buffer says zero — comes through
    // untouched rather than being re-stamped with THIS view's indices.
    for page in foreign {
        assert_eq!(
            listing(page),
            0,
            "the other camera's page {page} was touched by this view's compaction"
        );
    }
}

/// A table entry that is resident but not in this view's `page_list`.
/// Mirrors `PAGE_UNLISTED` in `page_table.wgsl`.
const PAGE_UNLISTED: u32 = 0xffff_ffff;

/// A page belonging to light 0 on an explicit cube FACE.
fn lamp_face_page(view: u32, face: u32, level: u32, cell: (u32, u32), lights: u32) -> u32 {
    local_page(view, level, cell, lights) + face * PageConfig::default().local_face_pages()
}

/// One lamp over a floor and a box, through the REAL pipeline: planted
/// table -> per-level culls -> compaction -> expansion -> draw, then
/// the atlas texels are read back and checked against what the light
/// actually sees.
///
/// # 🔴 The rig this track never had, and the recurring bug it is for
///
/// Every lamp defect so far — the world-axis spot, the sunless gate,
/// the seam wedge — was found by a person staring at a broken frame,
/// because nothing between "the shaders compile" and "the editor looks
/// wrong" ever drew a lamp's page and read it. This does: if geometry
/// lands in the wrong page, at the wrong depth, at a blob's LOD or not
/// at all, a texel count here moves.
///
/// Two pages on purpose: the lamp's FINEST level and a coarse one. The
/// coarse page pairs against a coarse clipmap bucket's survivors —
/// the exact machinery behind "the shadow is a deformed blob", so an
/// empty or garbage coarse page fails here rather than on screen.
#[test]
fn a_lamp_page_holds_what_its_light_sees() {
    use glam::{Mat4, Vec3};
    use kooch_render::meshlet::{
        MeshInstance, MeshletCullPipelines, MeshletScene, SceneCullParams, build_default_meshlets,
    };

    // The cull pipeline binds 5 groups and 9 storage buffers, past the
    // default limits — the shared helper mirrors the production
    // GpuContext, where this file's own `device()` does not.
    let Some((device, queue)) = common::try_acquire_device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let (device, queue) = (&device, &queue);
    let device = device.clone();
    let queue = queue.clone();

    // The scene: a lamp 4 m up, a 40 m floor whose top is y = 0, and a
    // half-metre box hanging at (0.45, 2, -0.45) — inside the window of
    // face 3's cell (8, 8) but covering only part of it, so the page
    // must hold BOTH populations: box depth and floor depth.
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
    let lights_buffer = {
        let record = kooch_lighting::GpuLight {
            position: lamp.to_array(),
            range,
            kind: 1,
            ..Default::default()
        };
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lamp_page_test_light"),
            size: std::mem::size_of::<kooch_lighting::GpuLight>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytemuck::bytes_of(&record));
        buffer
    };

    // Plant the lamp's pages: face 3 (-Y, toward the floor). The fine
    // page is the chain's floor; the coarse one two levels up, whose
    // octave lands in a coarse clipmap bucket.
    let config = PageConfig::default();
    let fine_level = config.local_floor();
    let fine_side = config.side(fine_level);
    let fine = lamp_face_page(0, 3, fine_level, (fine_side / 2, fine_side / 2), LIGHTS);
    let coarse_level = fine_level + 3;
    let coarse = lamp_face_page(0, 3, coarse_level, (1, 1), LIGHTS);

    let mut page_pool = PagePool::new(&device, small());
    let entries = VIEWS * span(LIGHTS);
    page_pool.ensure_entries(&device, entries);
    let cell = PAGE_CELL as usize;
    let mut slots = vec![0u32; entries as usize * cell];
    const FINE_SLOT: u32 = 3;
    const COARSE_SLOT: u32 = 7;
    slots[fine as usize * cell] = FINE_SLOT + 1;
    slots[coarse as usize * cell] = COARSE_SLOT + 1;
    // The fine page carries a receivers' ask — centimetres, octave 9 —
    // the way the marking records one. The coarse page carries none,
    // exercising the range fallback.
    const ASKED: u32 = 9;
    slots[fine as usize * cell + 3] = ASKED + 1;
    queue.write_buffer(page_pool.slots(), 0, bytemuck::cast_slice(&slots));

    // Built the way the FRAME builds it — against the cull pipelines'
    // meshlet layout, which is what the depth pipeline's group(1)
    // expects. The `rasterizer()` helper hands the pool's own layout,
    // which no test had ever exercised a draw through.
    let cull_pipelines = MeshletCullPipelines::new(&device);
    let mut raster = PageRasterizer::new(
        &device,
        cull_pipelines.meshlet_bind_group_layout(),
        PageConfig::default(),
        ClipmapConfig::default(),
        small(),
        kooch_render::meshlet::DEFAULT_MAX_TRIANGLES as u32,
    );
    let meshlet_bg = kooch_render::meshlet::pool_meshlet_bind_group(
        &device,
        cull_pipelines.meshlet_bind_group_layout(),
        &gpu_pool,
    );
    let threads = instances.len() as u32 * meshlets_per_mesh;
    raster.ensure_capacity(&device, threads, threads);

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
        LIGHTS,
        &lights_buffer,
        1.0,
    );
    queue.submit([encoder.finish()]);

    // The LOD half: the fine page buckets where its receivers asked
    // (octave 9 -> a centimetre-class survivor list), NOT where the
    // lamp's 20-metre range would put it — the bucketing that turned
    // sphere shadows into octahedra. The coarse page, with no ask,
    // falls back to the range.
    let counts = read_words(&device, &queue, raster.counts_buffer());
    assert_eq!(
        counts[ASKED as usize],
        1,
        "the asked-for bucket does not hold the fine page: {:?}",
        &counts[..17]
    );
    let clipmap = ClipmapConfig::default();
    let finest = clipmap.base / config.texels(0) as f32;
    let fallback_texel = 2.0 * range / config.texels(coarse_level) as f32;
    let fallback = ((fallback_texel / finest).log2().floor() as usize).min(16);
    assert_eq!(
        counts[fallback],
        1,
        "the unasked page does not fall back to the range bucket {fallback}: {:?}",
        &counts[..17]
    );

    // What the light sees, by construction: the floor at 4 m stores
    // `PAGE_NEAR / 4`; the box's lit surfaces sit between 1.75 and
    // 2.25 m. Reversed depth, so the box is the LARGER value.
    let floor_depth = 0.05 / 4.0;
    let page = config.page;
    let read_page =
        |slot: u32| -> Vec<f32> { read_atlas_page(&device, &queue, &raster, slot, page) };

    for (name, slot, min_box, max_box) in [
        ("fine", FINE_SLOT, 0.005, 0.30),
        ("coarse", COARSE_SLOT, 0.002, 0.40),
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
}

/// One page of the atlas, as f32 depths.
///
/// Depth formats refuse partial copies, so the whole layer comes back
/// and the page is cut out on the CPU.
fn read_atlas_page(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    raster: &PageRasterizer,
    slot: u32,
    page: u32,
) -> Vec<f32> {
    let pool = small();
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

const SPOT: &str = r#"
@group(0) @binding(0) var<storage, read_write> out: array<f32>;

@compute @workgroup_size(1, 1, 1)
fn cs_spot() {
    // A spot pointing straight DOWN, a floor point below and ahead of
    // it — the exact shape of the scene that shipped broken.
    let dir = vec3<f32>(0.0, -1.0, 0.0);
    let below = vec3<f32>(0.4, -3.0, 0.2);

    // 1. The rotated offset lands on face 0 — the spot's one face.
    let rotated = spot_local(dir, below);
    let hit = cube_face(rotated);
    out[0] = hit.w;
    out[1] = hit.x;
    out[2] = hit.y;

    // 2. The raster projects the SAME rotated offset with a positive w
    //    through the whole-face cell, so writer and reader share one
    //    mapping by construction.
    let face = cell_face(0u, vec2<u32>(0u, 0u), 1u, rotated);
    out[3] = face.z;

    // 3. A point ON the axis is the face's centre, at its distance.
    let centred = spot_local(dir, dir * 5.0);
    out[4] = centred.x;
    out[5] = length(centred.yz);
    let centre_uv = cube_face(centred);
    out[6] = centre_uv.x;
    out[7] = centre_uv.y;

    // 4. The basis is orthonormal: rotation preserves length, which is
    //    what keeps `distance` and the level choice frame-independent.
    out[8] = length(rotated) - length(below);
}
"#;

/// A spot's page frame follows the SPOT's axis, through the shader's
/// own `spot_local`, `cube_face` and `cell_face` — not a Rust mirror.
///
/// 🔴 Written after the defect shipped: the marking and the reader
/// forced `face = 0` while keeping the WORLD-axis uv, and the depth
/// raster projected through the world's +X. Three mappings of one page;
/// on screen, occlusion the shape of nothing that exists.
#[test]
fn a_spot_page_rotates_with_its_axis() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let out = run_page_table_shader_f32(&device, &queue, SPOT, "cs_spot", 36);
    assert_eq!(out[0], 0.0, "a point in the cone lands on face 0");
    assert!(
        (out[1] - 0.5).abs() < 0.1 && (out[2] - 0.5).abs() < 0.1,
        "a near-axis point maps near the face's centre, got ({}, {})",
        out[1],
        out[2]
    );
    assert!(
        out[3] > 0.0,
        "the raster's w is positive in front of the spot"
    );
    assert!(
        (out[4] - 5.0).abs() < 1e-4 && out[5].abs() < 1e-4,
        "the axis maps to the face's axis"
    );
    assert!(
        (out[6] - 0.5).abs() < 1e-4 && (out[7] - 0.5).abs() < 1e-4,
        "the axis is the face's centre"
    );
    assert!(out[8].abs() < 1e-4, "the basis is orthonormal");
}

/// Runs a snippet concatenated after `page_table.wgsl`, with one
/// writable buffer at `@group(0) @binding(0)`, and reads it back.
fn run_page_table_shader(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    body: &str,
    entry: &str,
    bytes: usize,
) -> Vec<u32> {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(entry),
        source: wgpu::ShaderSource::Wgsl(format!("{}\n{body}", kooch_lighting::PAGE_TABLE).into()),
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: Some(&layout),
        module: &module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    });
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: bytes as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    queue.submit([encoder.finish()]);
    read_words(device, queue, &buffer)
}

/// The same harness, reading floats.
fn run_page_table_shader_f32(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    body: &str,
    entry: &str,
    bytes: usize,
) -> Vec<f32> {
    run_page_table_shader(device, queue, body, entry, bytes)
        .into_iter()
        .map(f32::from_bits)
        .collect()
}

const FLOOR: &str = r#"
@group(0) @binding(0) var<storage, read_write> out: array<u32>;

@compute @workgroup_size(1, 1, 1)
fn cs_floor() {
    // The engine's own virtual size, and two others so the derivation
    // is exercised rather than a single lucky value.
    out[0] = local_level_floor(16384u);
    out[1] = local_level_floor(2048u);
    out[2] = local_level_floor(1024u);
    out[3] = LOCAL_MAX_TEXELS;
    // Pages a lamp can address across one face, before and after.
    out[4] = 128u * 128u;
    out[5] = level_side_of(local_level_floor(16384u), 128u)
        * level_side_of(local_level_floor(16384u), 128u);
}
"#;

#[test]
fn a_lamp_cannot_ask_for_the_suns_finest_levels() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let out = run_page_table_shader(&device, &queue, FLOOR, "cs_floor", 24);

    assert_eq!(
        out[0], 3,
        "16384 virtual texels should give up three levels"
    );
    assert_eq!(out[1], 0, "a chain already at the cap gives up nothing");
    assert_eq!(out[2], 0, "and a finer cap is not raised back up");
    assert_eq!(out[3], 2048, "the cap moved without this test being read");

    // The whole point, as a ratio: what the floor takes off the table.
    assert_eq!(
        out[4] / out[5].max(1),
        64,
        "the floor should be 64x in pages"
    );

    // Every pass starts its walk there. A floor one pass ignores is a
    // pass looking in levels nobody marks.
    for (file, source) in [
        (
            "inti_pbr.wgsl",
            include_str!("../../kooch_lighting/shaders/inti_pbr.wgsl"),
        ),
        (
            "inti_debug.wgsl",
            include_str!("../../kooch_lighting/shaders/inti_debug.wgsl"),
        ),
        ("page_mark.wgsl", include_str!("../shaders/page_mark.wgsl")),
    ] {
        assert!(
            source.contains("local_level_floor("),
            "{file} does not consult the lamp chain's floor"
        );
    }
}

/// `face_local` and `cube_face` agree, and a point behind a face comes
/// back with a negative `w` rather than being rejected.
///
/// 🔴 The bar. A triangle straddling a cube seam has vertices on two
/// faces, and rejecting one of them per vertex does not remove the
/// triangle — it pushes one corner outside the clip volume and lets the
/// clipper interpolate the rest, drawing a wedge of geometry into a page
/// it never touched. The fix is to project unconditionally and let `w`
/// carry the answer, so this pins BOTH halves: the projection agrees
/// with the face selection where they overlap, and disagrees by SIGN
/// where the point is behind.
const FACE_LOCAL: &str = r#"
@group(0) @binding(0) var<storage, read_write> out: array<f32>;

@compute @workgroup_size(1, 1, 1)
fn cs_face_local() {
    var worst_uv = 0.0;
    var wrong_sign = 0.0;
    var behind_positive = 0.0;
    for (var face = 0u; face < 6u; face = face + 1u) {
        for (var i = 0u; i < 9u; i = i + 1u) {
            let uv = clamp(
                vec2<f32>(f32(i % 3u), f32(i / 3u)) * 0.5,
                vec2<f32>(0.05), vec2<f32>(0.95));
            let dir = face_dir(face, uv);
            let local = face_local(face, dir);
            // In front of its own face, and the uv it reconstructs is
            // the uv it was built from.
            if local.z <= 0.0 {
                wrong_sign = wrong_sign + 1.0;
            }
            let back = local.xy / max(local.z, 1e-6) * 0.5 + vec2<f32>(0.5);
            worst_uv = max(worst_uv, max(abs(back.x - uv.x), abs(back.y - uv.y)));

            // And the OPPOSITE face has to report it behind. Rejecting
            // that per vertex is the defect; reporting it as negative w
            // is the fix.
            let opposite = select(face - 1u, face + 1u, face % 2u == 0u);
            if face_local(opposite, dir).z > 0.0 {
                behind_positive = behind_positive + 1.0;
            }
        }
    }
    out[0] = worst_uv;
    out[1] = wrong_sign;
    out[2] = behind_positive;
}
"#;

#[test]
fn a_point_behind_a_face_gets_a_negative_w() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let out = run_page_table_shader(&device, &queue, FACE_LOCAL, "cs_face_local", 12);
    let worst = f32::from_bits(out[0]);
    let wrong_sign = f32::from_bits(out[1]);
    let behind = f32::from_bits(out[2]);

    assert_eq!(
        wrong_sign, 0.0,
        "{wrong_sign} directions came back BEHIND the face they were built on; the \
         clipper would drop geometry that belongs in the page"
    );
    assert!(
        worst < 1e-5,
        "the projection reconstructs a uv off by {worst}; it disagrees with the face \
         selection the marking pass used, so pages are drawn where nothing looks"
    );
    assert_eq!(
        behind, 0.0,
        "{behind} directions read as IN FRONT of the opposite face; a point behind a \
         face has to come back with a negative w or it rasterises into the wrong one"
    );
}
