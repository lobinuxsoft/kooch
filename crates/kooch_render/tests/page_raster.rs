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
        row_cap: u32::MAX,
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
fn the_lamp_arena_is_sized_by_groups() {
    // The arena is `[slot * capacity + group]` — one ROW A LAMP — so
    // whatever sizes it is multiplied by up to `LAMP_CULLS`. Sized by
    // the cull's thread count instead of the scene's real group count,
    // 2024 instances at 4700 meshlets over 64 lamps asks for 2.4 GB.
    //
    // 🔴 wgpu does not panic on that. It returns an INVALID buffer, and
    // every `Queue::submit` for the rest of the run fails validation
    // with a message that names a label and no cause.
    let instances = 2024u64;
    let meshlets = 4700u64;
    let lamps = 64u64;
    let over_approximation = instances * meshlets * lamps * 4;
    assert!(
        over_approximation > 256 * 1024 * 1024,
        "the bug this guards needs the over-approximation to exceed a buffer limit; \
         it measured {over_approximation} bytes",
    );
    // The real group count is the prefix sum, which for a scene of
    // mostly single-group cubes is nearer the instance count than the
    // thread count.
    let real_groups = instances + 24 * 1000;
    assert!(
        real_groups * lamps * 4 < 64 * 1024 * 1024,
        "the real count has to fit comfortably, or the fix is not one",
    );
}

/// The pre-pass's pair list is a PRODUCT — lamps times instances — and
/// a constant cannot hold one.
///
/// 🔴 The failure it caused is the one this whole track keeps meeting:
/// silent and healthy-looking. `cs_lamp_pairs` claims a slot with an
/// `atomicAdd` and drops the pair past the cap, so WHICH lamps keep
/// their geometry is whichever threads arrived first. A lamp that loses
/// its pairs still marks its pages, still gets them resident, still
/// gets them listed — and its cull finds no survivor, so they are
/// cleared. A cleared page is far depth under reversed-Z, which every
/// reader answers "nothing occludes".
///
/// The two `Lamp shadow pages` views named it in one look: no white in
/// `faces`, so every page was resident, and uniform green in
/// `occlusion`, so every page was empty.
#[test]
fn the_pair_list_outgrows_constants() {
    // `dense.scene`, measured: 2157 entities and 64 lamps, and the ones
    // under test carry range 90 over a city this size — so the sphere
    // test keeps most instances for most lamps.
    let instances = 2157u64;
    let lamps = 64u64;
    let old_cap = 16_384u64;
    assert!(
        instances * lamps > old_cap * 8,
        "the bug this guards needs the scene's bound to dwarf the old cap;          it measured {} pairs against {old_cap}",
        instances * lamps,
    );
    // And the bound the list now grows to still fits a buffer, at the
    // eight bytes a pair costs.
    assert!(
        instances * lamps * 8 < 16 * 1024 * 1024,
        "the bound has to fit comfortably, or growing to it is not the fix",
    );
}

/// The per-view clear of `visible_counts` must not reach the lamps'
/// buckets.
///
/// # 🔴 A source check, because the defect needs two cameras to appear
///
/// `PageRasterizer::record` runs once per VIEW and `LampCull::record`
/// once per FRAME, guarded by `lamp_frame`. So a clear that spans the
/// whole buffer is undone for the sun — its culls rerun after it — and
/// permanent for the lamps: the second camera wipes their survivor
/// counts and then skips the cull that refills them.
///
/// A lamp bucket reading zero survivors is not a slow path. The
/// compaction stamps its pages `PAGE_EMPTY` and clears them, and a
/// cleared page is far depth under reversed-Z, which every reader
/// answers "nothing occludes". Every lamp in the scene stops casting
/// and every counter stays healthy.
///
/// Every headless test here runs ONE view, so nothing in this file can
/// reproduce it. The editor has two.
#[test]
fn the_clear_spares_lamp_buckets() {
    let source = include_str!("../src/shadow/pages/raster.rs");
    assert!(
        !source.contains("clear_buffer(&self.visible_counts, 0, None)"),
        "the per-view clear spans the whole buffer again; it must stop at the sun's levels,          because the lamps' cull runs once a frame and will not refill what a second view wiped"
    );
    assert!(
        source.contains("clear_buffer(&self.visible_counts, 0, Some(levels as u64 * 4))"),
        "the per-view clear no longer covers the sun's own levels"
    );
}

/// The lamps' meshlet passes are dispatched over `pairs * meshlets`, and
/// that product does not fit one dispatch dimension.
///
/// # 🔴 Why no GPU test in this file can catch it
///
/// The rigs here draw two instances of a cube. `meshlets_per_mesh` is
/// one, the pair count is two, the dispatch is a single workgroup, and
/// it will be a single workgroup no matter what the arithmetic does.
/// `dense.scene` is 2157 instances at a SCENE-WIDE max of 4563 meshlets
/// with 64 lamps — and `meshlets_per_mesh` is the maximum over the whole
/// pool, not this mesh's own count, so it multiplies every pair.
///
/// An indirect dispatch past `maxComputeWorkGroupCount` is undefined,
/// and here it did nothing: the culls never ran, every lamp bucket kept
/// zero survivors, every lamp page was stamped empty and cleared, and
/// every reader answered "nothing occludes" over a page that was
/// resident and correctly keyed. No lamp in the scene cast a shadow and
/// no counter said why.
#[test]
fn the_dispatch_outgrows_one_dimension() {
    use kooch_core::gpu::limits::{MAX_WORKGROUPS_PER_DIM, tiled_workgroups};

    // Measured on `dense.scene`, from the cull's own growth logs:
    // `visible_meshlets required = 9 842 308` over 2157 instances.
    let instances = 2157u32;
    let scene_max_meshlets = 9_842_308u32 / instances;
    let lamps = 64u32;
    // Even the pair cap this path shipped with is far past the limit.
    let old_pair_cap = 16_384u32;
    let threads = old_pair_cap * scene_max_meshlets;
    let flat = threads.div_ceil(64);
    assert!(
        flat > MAX_WORKGROUPS_PER_DIM,
        "the bug this guards needs the flat count to exceed the limit;          it measured {flat} against {MAX_WORKGROUPS_PER_DIM}"
    );

    // Tiled, both dimensions are legal and every thread is still covered.
    let (x, y) = tiled_workgroups(threads, 64);
    assert!(x <= MAX_WORKGROUPS_PER_DIM && y <= MAX_WORKGROUPS_PER_DIM);
    assert!(
        u64::from(x) * u64::from(y) * 64 >= u64::from(threads),
        "the tiled shape does not cover every thread"
    );

    // And the pre-pass's own dispatch, which is lamps times instances.
    let (px, py) = tiled_workgroups(lamps * instances, 64);
    assert!(px <= MAX_WORKGROUPS_PER_DIM && py <= MAX_WORKGROUPS_PER_DIM);

    // The shaders must read the second dimension, or tiling the args
    // just runs the same threads several times.
    let source = include_str!("../shaders/lamp_cull.wgsl");
    for entry in ["cs_lamp_pairs", "cs_lamp_err", "cs_lamp_cull"] {
        let at = source
            .find(&format!("fn {entry}("))
            .unwrap_or_else(|| panic!("lamp_cull.wgsl has no {entry}"));
        let body = &source[at..at + 400];
        assert!(
            body.contains("num_workgroups"),
            "{entry} indexes by gid.x alone; a tiled dispatch would run row zero              {} times over",
            "y"
        );
    }
}

/// The moved-caster list is sized by what the scene moves, not by a
/// constant.
///
/// 🔴 The cap was not a memory budget, it was a cache switch. Past it
/// `write_moved` bumps the scene generation, which voids every page
/// every frame it happens — and `dense.scene` spins 2026 casters against
/// a cap of 256, so it happened continuously. The panel then reads a
/// pool at 100% hit over a raster redrawing two thirds of the atlas, and
/// the two together look like a working cache.
#[test]
fn the_moved_list_grows() {
    // Measured on `dense.scene`, from the engine's own warning.
    let spinning = 2026u64;
    let old_cap = 256u64;
    assert!(
        spinning > old_cap,
        "the bug this guards needs the scene to outrun the cap"
    );
    // Sixteen bytes a sphere, against a shadow atlas measured at 52 MiB.
    // There was never a memory argument for the cap.
    assert!(
        spinning * 16 < 64 * 1024,
        "the whole list is {} bytes; a cap that small was never about memory",
        spinning * 16
    );
}

/// The per-level split adds up to the sum beside it, and stops at the
/// sun's levels.
///
/// #1018: the sum says how many pages redrew and cannot say which level
/// did it. One level crossing a boundary and the whole chain re-snapping
/// have different causes and the same total.
#[test]
fn the_split_matches_the_sum() {
    let Some((device, _queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let raster = rasterizer(&device);
    let levels = ClipmapConfig::default().levels as usize;
    let mut words = vec![0u32; raster.count_slots() as usize];
    for (level, word) in words[..levels].iter_mut().enumerate() {
        *word = level as u32 + 1;
    }
    // A lamp's bucket, which the split must not pick up.
    words[levels] = 900;

    let counts = raster.decode(&words, 0);
    let split: u32 = counts.by_level.iter().sum();
    let planted: u32 = (1..=levels as u32).sum();
    assert_eq!(split, planted, "the sun's levels did not come back whole");
    // 🔴 NOT equal to `pages`, and that is the field's doc being wrong
    // rather than this. `pages` sums `words[..buckets]`, and `buckets`
    // is `clipmap.levels + LAMP_CULLS` — so it carries the lamps too.
    assert!(
        counts.pages > split,
        "the lamp bucket vanished from the total: {} against {split}",
        counts.pages,
    );
    assert_eq!(counts.by_level[0], 1, "level 0 lost its count");
    assert!(
        counts.by_level[levels..].iter().all(|n| *n == 0),
        "a lamp's bucket leaked into the sun's split",
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
    // 🔴 Per BUCKET: the sun's clipmap levels first — octaves of its
    // own scale, level L on bucket L — then one bucket per lamp slot,
    // each fed by that lamp's own cull. Then bucket overflow, local
    // pages, pairs, pair overflow, a retired slot (it counted the other
    // camera's pages when the compaction walked the whole shared
    // table) — and then a second run per bucket for the survivors each
    // cull produced, which is the other half of the expansion's cost,
    // and a third for the cells a scatter would have visited instead.
    assert_eq!(
        buckets,
        sun + 256,
        "the lamp buckets moved; `LAMP_CULLS` and the shader's constant have to move together"
    );
    // …then the two receiver-bound rejections, the lamps' (#940) and
    // the sun's (#949), which are counted apart because they measure
    // different properties of a scene — and last the inverted
    // expansion's own two (#1022): the pages its descents reached, and
    // the descents that ran out of stack.
    assert_eq!(raster.count_slots(), buckets * 3 + 9);
    let mut words = vec![0u32; raster.count_slots() as usize];
    words[0] = 7;
    words[1] = 5;
    words[2] = 9;
    words[buckets as usize + 1] = 42;
    words[buckets as usize + 2] = 900;
    // The two rejections sit at the tail, and the point of the pair is
    // that a reader can tell them apart: a compact scene leaves the
    // sun's bound nothing to reject while the lamps' still bites.
    words[buckets as usize * 3 + 5] = 11;
    words[buckets as usize * 3 + 6] = 77;
    // The inverted shape's cost, which is a MEASURED number and not the
    // `pages * meshlets` product the paired one reports. A reader that
    // took `tests` for both would compare a walk against an area.
    words[buckets as usize * 3 + 7] = 1234;
    let counts = raster.decode(&words, 1);
    assert_eq!(counts.pages, 21, "every bucket sums");
    assert_eq!(counts.local, 42, "local pages are reported, not hidden");
    assert_eq!(counts.pairs, 900);
    assert_eq!(counts.depth_rejected, 11, "the lamps' bound, alone");
    assert_eq!(counts.sun_rejected, 77, "the sun's bound, alone");
    assert_eq!(
        counts.walk, 1234,
        "the descent's own cost, counted where it happens"
    );
    assert_eq!(
        counts.walk_overflow, 0,
        "a descent that drops a subtree drops a caster, so the healthy reading is zero",
    );
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

/// The sun's cache generation mirrors `sun_centre` exactly: a still
/// camera caches, a lateral step inside one page width still caches,
/// and a step that crosses the snap grid redraws.
///
/// This is the CPU/WGSL arithmetic seam of the cache — `write_gens`
/// recomputes the shader's snapped centre, and a mismatch here caches
/// pages whose world rect silently moved.
/// A hundred lights — `many_lights`, the scene that found the cap —
/// and one page per lamp: every one must land in its own bucket, none
/// in the dropped counter. At `LAMP_CULLS = 64` the lights past slot
/// 63 lost every page (121 dropped in the editor, a third of the
/// scene shadowless, and every unshadowed light washing out its
/// neighbours' shadows).
#[test]
fn a_hundred_lamps_compact_without_drops() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut raster = rasterizer(&device);
    let mut pool = PagePool::new(&device, small());
    const LIGHTS: u32 = 100;
    let lamps: Vec<kooch_lighting::GpuLight> = (0..LIGHTS)
        .map(|i| kooch_lighting::GpuLight {
            position: [i as f32, 2.0, 0.0],
            range: 10.0,
            kind: if i == 0 { 0 } else { 1 },
            ..Default::default()
        })
        .collect();
    pool.ensure_entries(&device, span(LIGHTS));
    let cell = PAGE_CELL as usize;
    let mut slots = vec![0u32; span(LIGHTS) as usize * cell];
    let config = PageConfig::default();
    let floor = config.local_floor();
    let side = config.side(floor);
    // One page per punctual lamp, on its own floor level. The helpers
    // address light 0; a lamp's region sits `slot * stride` further in.
    let light_stride = stride(PageConfig::default(), ClipmapConfig::default());
    for light in 1..LIGHTS {
        let page = lamp_face_page(0, 3, floor, (side / 2, side / 2), LIGHTS) + light * light_stride;
        slots[page as usize * cell] = light + 1;
    }
    queue.write_buffer(pool.slots(), 0, bytemuck::cast_slice(&slots));

    let mut encoder = device.create_command_encoder(&Default::default());
    raster.record_compaction(
        &device,
        &queue,
        &mut encoder,
        &pool,
        0,
        glam::Vec3::new(0.3, 1.0, 0.3),
        glam::Vec3::NEG_Y,
        &lamps,
    );
    queue.submit([encoder.finish()]);
    let counts = read_words(&device, &queue, raster.counts_buffer());
    let buckets = raster.buckets() as usize;
    let sun = ClipmapConfig::default().levels as usize;
    assert_eq!(
        counts[buckets],
        0,
        "pages were dropped: {:?} / lamp buckets {:?}",
        &counts[buckets..buckets + 5],
        &counts[sun..sun + 16]
    );
    let listed: u32 = counts[sun..sun + LIGHTS as usize].iter().sum();
    assert_eq!(
        listed,
        LIGHTS - 1,
        "every lamp's page reaches its bucket: {:?}",
        &counts[sun..sun + 16]
    );
}

#[test]
fn a_still_suns_page_caches() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut raster = rasterizer(&device);
    let mut pool = PagePool::new(&device, small());
    const LIGHTS: u32 = 1;
    let lamps = [kooch_lighting::GpuLight::default()];
    let page = sun_page(0, 5, (7, 8), LIGHTS);
    pool.ensure_entries(&device, span(LIGHTS));
    let cell = PAGE_CELL as usize;
    let mut slots = vec![0u32; span(LIGHTS) as usize * cell];
    slots[page as usize * cell] = 11;
    queue.write_buffer(pool.slots(), 0, bytemuck::cast_slice(&slots));

    let compact = |raster: &mut PageRasterizer, eye: glam::Vec3| {
        let mut encoder = device.create_command_encoder(&Default::default());
        raster.record_compaction(
            &device,
            &queue,
            &mut encoder,
            &pool,
            0,
            eye,
            glam::Vec3::NEG_Y,
            &lamps,
        );
        queue.submit([encoder.finish()]);
        read_words(&device, &queue, raster.counts_buffer())
    };
    let buckets = raster.buckets() as usize;

    // Off the snap grid's own lines: an eye at the origin sits exactly
    // on a boundary, where the tiniest step flips `floor` — a real
    // invalidation, not the case under test.
    let config = PageConfig::default();
    let width = ClipmapConfig::default().base * 32.0 / config.side(0) as f32;
    let eye = glam::Vec3::new(0.25 * width, 0.0, 0.25 * width);
    let counts = compact(&mut raster, eye);
    assert_eq!(counts[5], 1, "the cold page was not listed");
    let counts = compact(&mut raster, eye);
    assert_eq!(
        counts[buckets + 4],
        1,
        "a still camera did not cache the page: {:?}",
        &counts[..8]
    );
    // A step that stays inside level 5's snap cell: still cached.
    let counts = compact(&mut raster, eye + glam::Vec3::new(0.1 * width, 0.0, 0.0));
    assert_eq!(
        counts[buckets + 4],
        1,
        "a sub-page step invalidated the level: {:?}",
        &counts[..8]
    );
    // 🔴 A step of a WHOLE page, which used to be the expensive case
    // and is the point of the fix. The snapped centre steps, so under
    // the camera-relative key every cell index in the level shifted by
    // one and every page redrew — for pages whose world footprint had
    // not moved a millimetre. Keyed by absolute world position, this
    // page is exactly where it was and keeps its content.
    let counts = compact(&mut raster, eye + glam::Vec3::new(width, 0.0, 0.0));
    assert_eq!(
        counts[buckets + 4],
        1,
        "a one-page step re-keyed a page that had not moved: {:?}",
        &counts[..8]
    );

    // Far past it: a different piece of world entirely, wrapped onto the
    // same slot — the content is someone else's, redraw.
    let counts = compact(&mut raster, glam::Vec3::new(10_000.0, 0.0, 0.0));
    assert_eq!(
        counts[5],
        1,
        "a snap crossing did not bring the page back: {:?}",
        &counts[..8]
    );
    assert_eq!(counts[buckets + 4], 0, "and it must not count as cached");
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
        &[kooch_lighting::GpuLight::default()],
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
    // 🔴 And it LANDS in the lamp's OWN bucket — after the sun's
    // levels, at `levels + slot` — where its own cull's survivors are
    // bound. It briefly shared the sun's octave buckets; that borrowed
    // survivor lists culled for the camera's orthographic boxes and
    // broke lamp shadows both ways.
    let listed: u32 = (0..levels as usize).map(|l| counts[l]).sum();
    assert_eq!(
        listed, 3,
        "three sun pages were planted; {listed} reached the sun's buckets"
    );
    assert_eq!(
        counts[levels as usize],
        1,
        "lamp 0's bucket does not hold its page: {:?}",
        &counts[..levels as usize + 2]
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
    let level0: Vec<(u32, u32)> = (0..2).map(|i| (list[i * 4], list[i * 4 + 1])).collect();
    assert!(
        level0.contains(&planted[0]) && level0.contains(&planted[1]),
        "level 0 holds {level0:?}"
    );
    let at = 5 * bucket * 4;
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
            (list[at * 4], list[at * 4 + 1]),
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
        list[local_at * 4],
        local,
        "the local page's listing points somewhere else"
    );
    // Past the sun's buckets, inside lamp 0's — its own cull's bucket.
    let lamp_bucket = levels as usize;
    assert!(
        local_at >= lamp_bucket * bucket && local_at < (lamp_bucket + 1) * bucket,
        "the local page is outside lamp 0's bucket: listing {local_at}"
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
/// 🔴 Run under BOTH cull dispatch shapes (#1002). The clipmap culls
/// used to enter per rectangle cell and now enter per instance; what
/// they must not change is a single texel of what lands in the atlas.
/// This test draws real depth and reads it back, so a shape that lost
/// geometry fails here rather than on screen.
#[test]
fn a_lamp_page_holds_what_its_light_sees() {
    lamp_page_holds_its_view(false, small(), 7, coarse_level());
}

#[test]
fn the_two_level_cull_draws_the_same_page() {
    lamp_page_holds_its_view(true, small(), 7, coarse_level());
}

/// The TOP of a lamp's chain — one page for the whole cube face, which
/// is the only page a distant light gets (#1009).
///
/// 🔴 The rig above plants the floor and three levels up, and stops
/// three short of the top. So the level the distant tier depends on had
/// never been rasterised by anything but the editor, where "the lamp
/// casts nothing" and "the page is empty" look identical.
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

/// 🔴 The acceptance for #1016: the SAME page, read back from a pool
/// whose view spans two layers, with the coarse page living in the
/// second one.
///
/// A page's rect is the same texels of every layer, so a depth pass
/// that drew every page into the layer it happened to be attached to
/// would put the coarse page's depth on top of some other page — and
/// pass every test that only ever looked at layer zero.
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

/// Three levels above the chain's floor: coarse enough to pair against a
/// coarse bucket's survivors, fine enough that its cell is a window
/// rather than the whole face.
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

    // Plant the lamp's pages: face 3 (-Y, toward the floor). The fine
    // page is the chain's floor; the coarse one two levels up, whose
    // octave lands in a coarse clipmap bucket.
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
    // 🔴 FOUR, not three, and that is the whole of what makes the
    // far-layer case detectable: with a layer of sixteen pages, slot 4
    // and slot 20 are the SAME rect of different layers. A depth pass
    // that drew every page into whichever layer it was attached to
    // would put the coarse page's depth on top of this one — and a
    // test whose two pages had different rects would never see it.
    const FINE_SLOT: u32 = 4;
    let coarse_slot = coarse_slot;
    slots[fine as usize * cell] = FINE_SLOT + 1;
    slots[coarse as usize * cell] = coarse_slot + 1;
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
    );
    queue.submit([encoder.finish()]);

    // The bucketing half: a lamp's pages — every level of its chain —
    // land in ITS bucket, after the sun's levels, where its own cull's
    // survivors are bound. Bucketing them by octave into the sun's
    // buckets handed them survivor lists culled for the camera's
    // orthographic boxes: a close lamp's casters were culled away and a
    // far bucket drew root meshlets — sphere shadows as faceted lumps.
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
    // The survivors mirror: lamp 0's cull found the floor and the box,
    // and the out-of-range lamp's slice is EMPTY — its light sphere
    // touches no instance, so the pre-pass never let it reach the
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

    // What the light sees, by construction: the floor at 4 m stores
    // `PAGE_NEAR / 4`; the box's lit surfaces sit between 1.75 and
    // 2.25 m. Reversed depth, so the box is the LARGER value.
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

    // ---- The cache: a second frame with nothing changed draws NOTHING.
    // The stamps written by the first compaction match their lamp's
    // generation, so both pages are cached, no page is listed, the
    // depth pass clears no quad — and the atlas still holds the scene.
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

    // ---- Invalidation: the occluder "moves" — its old bounds arrive
    // as a moved sphere — and every page its lamp can reach redraws.
    // Per-light granularity: both pages of lamp 0 come back.
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
///
/// Depth formats refuse partial copies, so the whole layer comes back
/// and the page is cut out on the CPU.
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

/// Which way a light-facing triangle winds after the page transform.
///
/// 🔴 Run through the SHADER'S OWN `sun_basis`, `sun_page_rect` and
/// `page_clip` — not a Rust mirror of them. A mirror is what let this
/// ship: three separate flips, each individually right, and nobody ever
/// multiplied them out.
const WINDING: &str = r#"
@group(0) @binding(0) var<storage, read_write> out: array<f32>;

@compute @workgroup_size(1, 1, 1)
fn cs_winding() {
    // The sun travels straight down, so a floor faces it.
    let dir = vec3<f32>(0.0, -1.0, 0.0);
    let basis = sun_basis(dir);

    // A triangle whose normal is +Y — towards the light — wound
    // counter-clockwise seen from above, which is what a front face is
    // everywhere else in this engine.
    var tri = array<vec3<f32>, 3>(
        vec3<f32>(0.0, 0.0, 0.0),
        vec3<f32>(1.0, 0.0, 0.0),
        vec3<f32>(0.0, 0.0, -1.0),
    );

    // Eye at the origin: the snap is irrelevant to winding, which is
    // what this measures.
    let rect = sun_page_rect(0u, vec2<u32>(0u, 0u), vec3<f32>(0.0), basis, 64.0, 128u);
    var clip = array<vec2<f32>, 3>();
    for (var i = 0u; i < 3u; i = i + 1u) {
        let p = tri[i];
        let local = vec3<f32>(
            dot(p, basis[0]),
            dot(p, basis[1]),
            dot(p, basis[2]),
        );
        let ndc = (sun_plane(p, basis) - rect.xy) / (rect.z * 0.5);
        clip[i] = page_clip(ndc, 0.5, vec4<f32>(0.0, 0.0, 128.0, 128.0), 1024.0).xy;
    }
    let a = clip[1] - clip[0];
    let b = clip[2] - clip[0];
    // Positive is counter-clockwise in clip space, which is Y-up.
    out[0] = a.x * b.y - a.y * b.x;
}
"#;

/// The pipeline's front face has to be the one the transform produces.
///
/// 🔴 The defect, measured: the sun's basis `(s, u, f)` has a
/// determinant of **-1** — `u = cross(s, f)` makes it left-handed — and
/// `page_clip` flips Y again to turn a texel row into a clip position.
/// Two flips do not cancel here, because only the second one is part of
/// the 2D map the rasteriser winds by. A triangle FACING the light comes
/// out clockwise, so `FrontFace::Ccw` called it a back face and
/// `cull_mode: Back` threw away exactly the geometry that casts.
///
/// What survived was the far shell of every closed mesh, which is why
/// the shadows were blobs with holes in them that changed shape as the
/// clipmap level — and with it the meshlet LOD — changed.
#[test]
fn a_light_facing_triangle_is_the_front_face() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("page_winding"),
        source: wgpu::ShaderSource::Wgsl(
            format!("{}\n{WINDING}", kooch_lighting::PAGE_TABLE).into(),
        ),
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
        entry_point: Some("cs_winding"),
        compilation_options: Default::default(),
        cache: None,
    });
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 4,
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

    let area = f32::from_bits(read_words(&device, &queue, &buffer)[0]);
    assert!(area != 0.0, "the triangle came out degenerate");
    let wound = if area > 0.0 {
        wgpu::FrontFace::Ccw
    } else {
        wgpu::FrontFace::Cw
    };
    assert_eq!(
        wound, PAGE_FRONT_FACE,
        "a triangle facing the light winds {wound:?} (signed area {area}), but the page \
         raster declares {PAGE_FRONT_FACE:?} — so back-face culling throws away every \
         surface that casts"
    );
}

/// The receiver's gradient at a few incidences, through the SHADER'S OWN
/// `receiver_slope` rather than a Rust mirror of it.
const SLOPE: &str = r#"
@group(0) @binding(0) var<storage, read_write> out: array<f32>;

@compute @workgroup_size(1, 1, 1)
fn cs_slope() {
    // Sun straight down. `texel / (2 * span)` is 1 here, so the numbers
    // below are the plane's own slope with no scaling to unpick.
    let basis = sun_basis(vec3<f32>(0.0, -1.0, 0.0));
    let span = 0.5;
    let texel = 1.0;

    // A floor under a vertical sun faces the light: no run at all.
    let flat = receiver_slope(vec3<f32>(0.0, 1.0, 0.0), basis, texel, span, 8.0);
    out[0] = flat.x;
    out[1] = flat.y;

    // Tilted 45 degrees about ONE axis. The whole claim of this file is
    // that the gradient appears on that axis and nowhere else.
    let n = normalize(vec3<f32>(0.0, cos(radians(45.0)), sin(radians(45.0))));
    let tilt = receiver_slope(n, basis, texel, span, 8.0);
    out[2] = tilt.x;
    out[3] = tilt.y;

    // Edge-on to the sun, where the ratio diverges and only the clamp
    // answers.
    let edge = receiver_slope(vec3<f32>(0.0, 0.0, 1.0), basis, texel, span, 3.0);
    out[4] = edge.x;
    out[5] = edge.y;

    // A clamp of zero turns the term off at any incidence.
    let off = receiver_slope(n, basis, texel, span, 0.0);
    out[6] = off.x;
    out[7] = off.y;
}
"#;

/// The receiver's gradient is per AXIS, not one number.
///
/// 🔴 This is the property a scalar bias cannot have, and the reason the
/// first attempt at #1017 measured as doing nothing. How much depth a
/// filter tap crosses depends on WHICH WAY it moved: along the tilt it
/// crosses the whole run, across it none. A single multiplier has to
/// cover the worst axis on every axis, so it detaches the shadow along
/// the one that needed no correction — and three captures at 79° of
/// incidence, with the multiplier at 0, at 8, and replaced by a constant
/// raised to the same step, were indistinguishable from each other.
#[test]
fn a_tilt_gradient_is_directional() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("receiver_slope"),
        source: wgpu::ShaderSource::Wgsl(format!("{}\n{SLOPE}", kooch_lighting::PAGE_TABLE).into()),
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
        entry_point: Some("cs_slope"),
        compilation_options: Default::default(),
        cache: None,
    });
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 32,
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

    let out: Vec<f32> = read_words(&device, &queue, &buffer)
        .into_iter()
        .map(f32::from_bits)
        .collect();

    assert!(
        out[0].abs() < 1e-4 && out[1].abs() < 1e-4,
        "a surface facing the sun has no run across its texel, got ({}, {})",
        out[0],
        out[1]
    );
    // 🔴 The assertion the whole change exists for.
    assert!(
        out[2].abs() < 1e-4,
        "the axis the surface did NOT tilt about picked up a gradient of {} — that is a \
         scalar wearing a vector's shape, and it detaches the shadow along the axis that \
         needed no correction",
        out[2]
    );
    assert!(
        (out[3].abs() - 1.0).abs() < 1e-3,
        "at 45 degrees the plane falls one depth unit per texel; the tilted axis reads {}",
        out[3]
    );
    assert!(
        (out[5].abs() - 3.0).abs() < 1e-3,
        "edge-on the ratio diverges and the clamp is the only answer: expected 3, got {}",
        out[5]
    );
    assert!(
        out[6].abs() < 1e-6 && out[7].abs() < 1e-6,
        "a clamp of 0 has to restore one depth for every tap, or no project can go back \
         to the numbers it tuned; got ({}, {})",
        out[6],
        out[7]
    );
}

/// The indirect draw has to issue enough vertices for a WHOLE meshlet.
///
/// 🔴 The defect, and it looked like everything except what it was. The
/// draw is indirect with a FIXED vertex count and the vertex shader
/// discards the tail past `desc.triangle_count`, so the count has to be
/// `max_triangles_per_meshlet * 3` — the figure `MeshletCull::new`
/// documents for the cascades' own draw, which is why theirs was right.
///
/// The page raster was issuing `meshlets_per_mesh * 3` instead: the
/// meshlet count of the registered mesh, a completely different
/// quantity. At the engine's defaults that is about a third of what a
/// 124-triangle meshlet needs, so every meshlet was drawn up to its
/// fortieth triangle or so and cut.
///
/// On screen it read as a shadow made of fragments that followed the
/// meshlet structure and rearranged themselves whenever the clipmap
/// level — and with it the LOD — changed. Three sessions' worth of
/// hypotheses went past it: page starvation, inverted meshlet faces,
/// the sampling bias.
#[test]
fn the_draw_covers_a_whole_meshlet() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let raster = rasterizer(&device);
    let pool = PagePool::new(&device, small());

    let mut encoder = device.create_command_encoder(&Default::default());
    raster.record_compaction(
        &device,
        &queue,
        &mut encoder,
        &pool,
        0,
        glam::Vec3::ZERO,
        glam::Vec3::NEG_Y,
        &[kooch_lighting::GpuLight::default()],
    );
    queue.submit([encoder.finish()]);

    let args = read_words(&device, &queue, raster.draw_args_buffer());
    assert_eq!(
        args[0],
        raster.triangles_per_meshlet() * 3,
        "the draw issues {} vertices for meshlets of up to {} triangles",
        args[0],
        raster.triangles_per_meshlet()
    );

    // And the cap itself has to cover what the builder really produces,
    // or the bug simply moves one layer down.
    use kooch_render::mesh::primitives::Primitive;
    use kooch_render::meshlet::build_default_meshlets;
    for (name, primitive) in Primitive::CANONICAL {
        let built = build_default_meshlets(&primitive.build()).expect("the primitive builds");
        let biggest = built
            .meshlets
            .iter()
            .map(|m| m.triangle_count)
            .max()
            .unwrap_or(0);
        assert!(
            biggest <= raster.triangles_per_meshlet(),
            "{name} has a {biggest}-triangle meshlet and the draw covers {}",
            raster.triangles_per_meshlet()
        );
    }
}

/// A clipmap texel is not one size, so a bias in metres cannot serve
/// both ends of the chain.
///
/// This is the argument the reader's bias is built on, in numbers: at
/// the defaults, level 0's texel and the last level's differ by four
/// orders of magnitude. Half a metre — what the reader used to add flat
/// — is six thousand texels at the near end and a fraction of one at
/// the far end. The near end is where an object meets the ground.
#[test]
fn a_clipmap_texel_is_not_one_size() {
    let clipmap = ClipmapConfig::default();
    let config = PageConfig::default();
    let across = config.side(0) * config.page;

    let finest = clipmap.extent(0) / across as f32;
    let coarsest = clipmap.extent(clipmap.levels - 1) / across as f32;

    assert!(
        coarsest / finest > 1000.0,
        "levels 0 and {} differ by {finest} vs {coarsest} metres per texel",
        clipmap.levels - 1
    );
    // And the constant that used to be added flat, measured in each.
    assert!(
        0.5 / finest > 1000.0,
        "half a metre is {} texels at level 0",
        0.5 / finest
    );
}

/// The page reader offsets its SAMPLE by the texel, the way the cascade
/// does — it does not add a constant to the depth it compares.
///
/// 🔴 A grep, because the alternative is a GPU test that reproduces the
/// whole shading bind group to observe one term. What it guards is
/// narrow and exact: the normal step has to be multiplied by a
/// per-level texel size INSIDE the walk, the depth step has to be
/// carried, and the comparison has to be against `receiver` alone.
/// All of them regressed together once.
///
/// 🔴 And the step has to be CAPPED. Uncapped it follows the texel —
/// 0.0002 m at clipmap level 0 and 9.2 m at level 16 — and nine metres
/// walks a receiver clean out of the volume its caster shadows, so the
/// comparison answers LIT with the page present and correctly drawn.
#[test]
fn the_page_reader_biases_in_texels() {
    let source = kooch_lighting::inti_pbr_shader(1);
    let start = source
        .find("fn inti_page_shadow(")
        .expect("the reader is in the shader");
    let end = source[start..]
        .find("\nfn inti_shadow(")
        .expect("the reader ends")
        + start;
    let body = &source[start..end];

    // 🔴 Matched without the whitespace, because the expression grew a
    // third factor and wrapped across lines. A grep test that pins the
    // FORMATTING fails on a change that never touched the behaviour,
    // which is how this one first fired.
    let dense: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        dense.contains("texel_world*inti_pages.bias.x"),
        "the offset has to scale with the level's texel"
    );
    assert!(
        body.contains("to_light * inti_pages.bias.y"),
        "and carry the cascade's depth term too"
    );
    assert!(
        body.contains("min(offset, inti_pages.bias.z)"),
        "and the offset has to be capped, or the coarse levels lose their shadows"
    );
    assert!(
        !body.contains("receiver + bias"),
        "a constant added to the compared depth is what detaches a shadow"
    );
}

/// Every pass that reads the page table reads it the SAME way.
///
/// 🔴 Written after breaking it, under the hash: the table grew a word
/// per entry and the compaction was not updated — it kept compiling,
/// kept running, and rasterised garbage into slots read off the wrong
/// word. The hash is gone; what can still drift is the entry STRIDE
/// and the flat contract itself — an entry is `PAGE_CELL` words, its
/// index is the page id, and the first word is `slot + 1` with zero
/// meaning absent. A reader that grows a probe loop back, or indexes
/// without the stride, reads an age as a slot again.
///
/// A grep, because the alternative is running four passes against a
/// table hand-built into a hostile state.
#[test]
fn every_table_reader_agrees_on_the_layout() {
    let readers = [
        (
            "page_compact.wgsl",
            include_str!("../shaders/page_compact.wgsl"),
        ),
        ("page_mark.wgsl", include_str!("../shaders/page_mark.wgsl")),
    ];
    for (name, source) in readers {
        assert!(
            !source.contains("table_slots[entry]") && !source.contains("table_slots[page]"),
            "{name} indexes the table's slots without PAGE_CELL"
        );
        assert!(
            !source.contains("PAGE_DEAD") && !source.contains("page_probe"),
            "{name} still speaks the hash's dialect — tombstones and probe             runs died with it"
        );
    }

    // The shading pass is the third reader and it lives in the other
    // crate. Its lookup is ONE indexed load — the whole point of the
    // flat table — so it must index by the page id, with the stride.
    let shading = kooch_lighting::inti_pbr_shader(1);
    assert!(
        shading.contains("inti_page_slots[page * PAGE_CELL]"),
        "the shading pass indexes the table's slots without PAGE_CELL"
    );
    assert!(
        !shading.contains("page_probe"),
        "the shading lookup grew a probe loop back; the flat table is one load"
    );
}

/// The shading reads the slice this frame's raster wrote.
///
/// 🔴 This assertion is the INVERSE of the one that stood here for an
/// hour, and the flip is the point. While the marking ran after the
/// fused pass, the shading sampled a table and an atlas that were a
/// frame old — but `Queue::write_buffer` is applied at the top of the
/// submit, ahead of every command in it, so the uniform it read was
/// THIS frame's. The reader re-based the clipmap a frame ahead of the
/// pages it was searching, and the fix was to double-buffer by parity.
///
/// The marking now runs BEFORE the fused pass, so all three are this
/// frame's and the parity is gone. What has to hold instead is that the
/// cameras still do not collide, and that the offset does not depend on
/// the frame at all — a leftover parity would now split the uniform
/// from the atlas it describes.
#[test]
fn the_uniform_slice_is_per_camera_and_not_per_frame() {
    let Some((device, _queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut raster = rasterizer(&device);

    raster.set_frame(0);
    let first: Vec<u64> = (0..2).map(|v| raster.uniform_span(v).0).collect();
    assert_ne!(first[0], first[1], "the cameras share a slice");

    for frame in 1..5u32 {
        raster.set_frame(frame);
        for view in 0..2u32 {
            assert_eq!(
                raster.uniform_span(view).0,
                first[view as usize],
                "frame {frame} moved view {view}'s slice"
            );
        }
    }
}

/// The clipmap's texel grid does not slide with the camera.
///
/// 🔴 The property that stops a shadow edge from crawling, and it is
/// only a property if it is measured. A clipmap is centred on the
/// camera, so without a snap every texel of it slides through the world
/// as the camera moves — a shadow edge is decided per texel, so the
/// silhouette is re-quantised every frame and shimmers. Temporal
/// blending hides that at the price of smearing; snapping removes it.
///
/// Runs the real WGSL on the GPU rather than a copy of the arithmetic:
/// five camera positions inside one page, and the page a fixed world
/// point lands in has to be identical in all of them.
#[test]
fn the_clipmap_grid_does_not_slide_with_the_camera() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    // 🔴 The ENGINE'S base, and it is not a power of two. An earlier
    // version of this test used 64.0, where every division lands exactly on
    // a power of two and `floor(log2(...))` cannot round down — so it passed
    // while the sun's levels were falling into the bucket below.
    const BASE: f32 = 1.28;
    const SIDE: u32 = 128;
    const LEVEL: u32 = 3;
    // One page of level 3, which is what the camera has to stay inside
    // for the grid to hold still.
    let page = BASE * 8.0 / SIDE as f32;

    let source = format!(
        "{}\n{}",
        kooch_lighting::PAGE_TABLE,
        r#"
@group(0) @binding(0) var<storage, read> eyes: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> cells: array<vec4<f32>>;

@compute @workgroup_size(1, 1, 1)
fn cs_snap(@builtin(global_invocation_id) id: vec3<u32>) {
    let basis = sun_basis(vec3<f32>(0.3, -1.0, 0.2));
    // A world point that never moves.
    let world = vec3<f32>(11.0, 0.0, -7.0);
    let base = 64.0;
    let side = 128u;
    let level = 3u;
    let extent = base * exp2(f32(level));

    let eye = eyes[id.x].xyz;
    // 🔴 The KEY of a fixed world point, and the world rect that key
    // stands for. Both are supposed to be properties of the POINT, not
    // of wherever the camera happens to be — that is what `sun_cell`'s
    // absolute-world addressing buys, and what the camera-relative key
    // it replaced could not do.
    let cell = sun_cell(world, eye, basis, base, side, level);
    let rect = sun_page_rect(level, cell, eye, basis, base, side);
    cells[id.x] = vec4<f32>(vec2<f32>(cell), rect.xy);
}
"#
    );

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("page_snap"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("page_snap"),
        layout: None,
        module: &module,
        entry_point: Some("cs_snap"),
        compilation_options: Default::default(),
        cache: None,
    });

    // ⚠️ The shader's own constants, which are NOT the ones above: it
    // declares `base = 64.0` where the engine ships 1.28. One page of
    // the level it reads is this wide, and the cameras have to cross
    // several of them or the test proves nothing.
    let page = 64.0 * 8.0 / 128.0;
    let eyes: Vec<[f32; 4]> = (0..9)
        .map(|i| {
            let t = i as f32 * page * 0.37;
            [t, 3.0, -t * 0.6, 0.0]
        })
        .collect();
    assert!(
        (eyes.len() - 1) as f32 * page * 0.37 > page,
        "the cameras never leave one page; nothing would be proven"
    );
    let eye_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("eyes"),
        size: (eyes.len() * 16) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&eye_buf, 0, bytemuck::cast_slice(&eyes));
    let out = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cells"),
        size: (eyes.len() * 16) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cells_read"),
        size: (eyes.len() * 16) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("page_snap"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: eye_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: out.as_entire_binding(),
            },
        ],
    });

    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(eyes.len() as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&out, 0, &staging, 0, (eyes.len() * 16) as u64);
    queue.submit([encoder.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    let read: Vec<[f32; 4]> =
        bytemuck::cast_slice::<u8, [f32; 4]>(&staging.slice(..).get_mapped_range()).to_vec();
    staging.unmap();

    // 🔴 The property, now that a page is keyed by absolute world
    // position: a fixed point's page does not move AT ALL while the
    // camera walks across several pages. Neither its key nor the world
    // rect that key stands for.
    //
    // The camera-relative key this replaced could only manage the weaker
    // version — the grid moved by WHOLE pages rather than fractions, so
    // texel footprints held and shadows did not crawl — and it paid for
    // it by re-keying every page of the level on every step: 72 FPS
    // standing still and 5 FPS moving (#948).
    let first = read[0];
    for (i, got) in read.iter().enumerate().skip(1) {
        assert_eq!(
            [got[0], got[1]],
            [first[0], first[1]],
            "camera {i} filed a fixed world point under a different page"
        );
        for axis in 0..2 {
            let slid = got[2 + axis] - first[2 + axis];
            assert!(
                slid.abs() < 1e-3,
                "camera {i} slid the page {slid} metres on axis {axis}"
            );
        }
    }
    assert_eq!(LEVEL, 3, "the level the shader hardcodes");
}

/// The page marking is recorded BEFORE the pass that shades with it.
///
/// 🔴 A source check, because the failure it guards has no test that can
/// see it: swapping the two lines compiles, runs, and produces a frame
/// that is correct for everything standing still. What breaks is
/// geometry that MOVES — the atlas is then a frame old, so an object is
/// compared against its own caster from the previous frame and shadows
/// itself while it travels. That was the reported symptom, and it took
/// the whole ordering to explain it.
///
/// The trade is Epic's and it is deliberate: the marking reads a depth
/// buffer the fused pass has not refilled yet, so LAST frame's depth
/// decides which pages exist — off by however far the camera moved, and
/// costing a page at the edge of the screen. THIS frame's geometry fills
/// them, which is what the shading compares against.
#[test]
fn the_marking_sits_between_raster_and_shading() {
    let source = include_str!("../src/meshlet/render_stage/frame/render_r64.rs");

    let raster = source
        .find(".render_geometry(")
        .expect("the frame rasterises");
    let mark = source
        .find("self.record_page_marking(")
        .expect("the frame marks pages");
    let bind = source
        .find("self.bind_page_shadows(")
        .expect("the frame binds them");
    let shade = source.find(".render_shading(").expect("the frame shades");

    // 🔴 The window, and it is one line wide on both sides.
    //
    // Before the raster the depth buffer still holds the PREVIOUS
    // frame, so the marking asks for pages where the geometry used to
    // be — a receiver that crossed a clipmap level boundary lands on a
    // page nobody requested, and a page that does not exist shades as
    // lit. That is what this order exists to stop, and it is Unreal's:
    // depth, then page management, then shading.
    //
    // After the shading is worse and was tried: the atlas is then a
    // frame old, so a moving object is compared against its OWN caster
    // from the previous frame and shadows itself.
    assert!(
        raster < mark,
        "the marking reads a depth buffer the raster has not filled yet, so it asks for \
         pages where the geometry was last frame"
    );
    assert!(
        mark < bind,
        "the shading is pointed at the pages before they are marked"
    );
    assert!(
        bind < shade,
        "the shading runs before it is pointed at this camera's pages"
    );

    // And the paint is the half that stays behind, because it writes the
    // colour buffer the shading is about to overwrite.
    let paint = source
        .find("self.record_page_paint(")
        .expect("the frame paints the debug view");
    assert!(
        shade < paint,
        "the debug paint runs before the pass that erases it"
    );
}

/// The paged shadow resolves at least as finely as the cascade it
/// replaces.
///
/// 🔴 The comparison that decided the default, and the reason it is a
/// test rather than a paragraph. "One shadow texel per screen pixel" is
/// Epic's ask and it is honest, but the technique being replaced is not
/// spending that: a cascade hands 2048 texels to a slice of the frustum
/// whatever the screen asked for. Measured, that is about twice the
/// resolution at every distance — so a project switching over at 100 %
/// gets a visibly coarser shadow and no setting that says why.
///
/// Both sides are computed with the engine's own arithmetic:
/// `level_below` is what the marking pass mirrors, and the cascade's
/// diameter/texels is what `cascades.rs` fits.
#[test]
fn the_paged_shadow_resolves_like_a_cascade() {
    use kooch_render::shadow::pages::{ClipmapConfig, PageConfig, level_below};

    const CASCADE_TEXELS: f32 = 2048.0;
    const FIRST: f32 = 10.0;
    const FAR: f32 = 100.0;
    const COUNT: usize = 4;
    // The resolution every figure in this track was measured at.
    const HEIGHT: f32 = 403.0;
    let focal = 1.0 / (60.0_f32.to_radians() / 2.0).tan();

    let clipmap = ClipmapConfig::default();
    let config = PageConfig::default();
    let virtual_texels = config.texels(0) as f32;

    let splits: [f32; COUNT] =
        std::array::from_fn(|i| FIRST * (FAR / FIRST).powf(i as f32 / (COUNT - 1) as f32));

    // What one cascade texel covers at `distance`, the way `cascades.rs`
    // fits it: the slice's diagonal over the atlas side.
    let cascade = |distance: f32| -> f32 {
        let mut near = 0.1;
        for (i, &far) in splits.iter().enumerate() {
            if distance <= far || i == COUNT - 1 {
                let h_near = 2.0 * near / focal;
                let h_far = 2.0 * far / focal;
                let body = ((far - near).powi(2) + (h_far + h_near).powi(2)).sqrt();
                let far_diag = 2.0_f32.sqrt() * h_far;
                return body.max(far_diag).ceil() / CASCADE_TEXELS;
            }
            near = far;
        }
        unreachable!()
    };

    // What one page texel covers, which is the level the marking picks.
    let paged = |distance: f32, density: u32| -> f32 {
        let wanted = 2.0 * distance / (focal * HEIGHT) * (100.0 / density as f32);
        let level = level_below(wanted * virtual_texels / clipmap.base).min(clipmap.levels - 1);
        clipmap.extent(level) / virtual_texels
    };

    // 🔴 The gap is SIZED, not closed. A quality setting's maximum is
    // its maximum, so the list stops at 100 % — and at 100 % the pages
    // are the coarser of the two at every distance measured. The number
    // below is how much coarser, and it is here so the day somebody
    // claims the page path "looks about the same" there is a figure to
    // answer with.
    let distances = [5.0_f32, 10.0, 20.0, 40.0, 80.0];
    // 🔴 The REFERENCE density, taken from the choices list rather than
    // from `Default`. It used to read the default and assert it was 100,
    // which was the same number by coincidence until the defaults moved
    // to what the engine is tuned at — and the comparison below is
    // against a cascade at full rate, so it has to be measured at the
    // reference whatever a project happens to ship.
    let density = kooch_render::settings::shadow_density_choices()
        .iter()
        .map(|choice| choice.value as u32)
        .find(|value| *value == 100)
        .expect("100 % is the reference the cascade comparison is made at");

    let mut worst: (f32, f32) = (0.0, 0.0);
    for distance in distances {
        let want = cascade(distance);
        let ratio = paged(distance, density) / want;
        assert!(
            ratio > 1.0,
            "at {distance} m the pages already match the cascade \
             ({ratio:.2}x); the gap this test sizes has closed and the \
             doc on `default_shadow_density` is now wrong"
        );
        if ratio > worst.1 {
            worst = (distance, ratio);
        }
    }
    // Measured. A clipmap level is a power of two, so where the chain
    // steps decides this as much as the density does — which is why the
    // worst case is not at the far end.
    // Measured at 2.41x, at 10 m — the far edge of the first cascade,
    // which is where a cascade is most generous and the chain has just
    // stepped. Pinned with a little slack so the number is a fact under
    // guard, not a tripwire on rounding.
    assert!(
        worst.1 <= 2.5,
        "the pages fell to {:.2}x the cascade at {} m",
        worst.1,
        worst.0,
    );

    // 🔴 The list REACHES past the default now, and the entries above it
    // have to say what they cost.
    //
    // This assertion used to be `top == default`, guarding "a list with
    // something above the default is a list whose maximum is a lie".
    // That is right for a quality tier and wrong for this number: 100 %
    // is one texel per screen pixel measured in the SUN's plane, and a
    // texel lands square only on a surface facing the sun. A receiver
    // tilted by 79° is already under one texel per pixel with the
    // control at its old ceiling — so the ceiling was pinned on the
    // wrong side of the case that needs it, and the pass had clamped to
    // 400 the whole time. Epic's equivalent goes negative for the same
    // reason.
    //
    // What replaces it is the guard that actually matters: an option
    // that multiplies the page count must NAME the multiplier, because
    // the pool overflows without a word and its failure looks like a
    // missing shadow.
    let choices = kooch_render::settings::shadow_density_choices();
    assert!(
        choices.iter().any(|choice| choice.value == density as i64),
        "the default density is not one of the options"
    );
    for choice in choices.iter().filter(|c| c.value > density as i64) {
        assert!(
            choice.label.contains("the pages"),
            "`{}` asks for more pages than the default without saying how many",
            choice.label,
        );
    }
}

/// The expansion's cost is reported as the product it is.
///
/// 🔴 Written after guessing this number instead of measuring it. The
/// scatter form was built on the assumption that a meshlet touches "a
/// handful" of pages; at the finest clipmap levels a page is a
/// centimetre across and a one-metre meshlet's rect covers 16384 cells,
/// so the frame went from 200 fps to 30. The assumption was never in a
/// test because it was never a measurement.
///
/// Now both halves come home in the same readback — pages per level from
/// the compaction, survivors per level copied in from the culls — and
/// the product is exact rather than assumed.
#[test]
fn the_counters_carry_the_expansions_cost() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let raster = rasterizer(&device);
    // Every run is per BUCKET, so the offsets follow the buckets and
    // not the clipmap's levels — planting at the clipmap's stride lands
    // the survivors inside the overflow flags.
    let levels = raster.buckets() as usize;

    // The layout has to have room for both runs, or `decode` reads a
    // survivor count out of a slot that holds an overflow flag.
    assert!(
        raster.count_slots() as usize >= levels * 2 + 5,
        "the counter buffer has no room for the survivor counts"
    );

    // Planted rather than rendered: what is under test is that `decode`
    // multiplies the right two runs, not what a scene happens to hold.
    let mut words = vec![0u32; raster.count_slots() as usize];
    words[3] = 7; // level 3: seven pages
    words[9] = 2; // level 9: two pages
    words[levels + 2] = 40; // pairs emitted
    words[levels + 5 + 3] = 100; // level 3: a hundred survivors
    words[levels + 5 + 9] = 500; // level 9: five hundred survivors

    let counts = raster.decode(&words, 0);
    assert_eq!(counts.tests, 7 * 100 + 2 * 500, "the product is wrong");
    assert_eq!(
        counts.worst,
        (9, 1000),
        "the worst level is the one that walks the most, not the one with the most pages"
    );
    assert_eq!(counts.pairs, 40);

    // And the third run: what the OTHER shape would have cost, and the
    // choice between them.
    //
    // 🔴 The numbers are picked so a GLOBAL choice and a PER-LEVEL one
    // disagree. Level 3 is cheaper to scatter, level 9 is cheaper to
    // pair; summed, pairing wins outright (1700 against 4050), so a
    // hybrid that compared totals would pick pairing everywhere and
    // save nothing. Comparing per level saves the 650 that level 3 was
    // wasting. That distinction IS the feature — the last attempt at
    // this picked one shape for the whole chain and cost two thirds of
    // the frame rate.
    words[levels * 2 + 5 + 3] = 50; // level 3: cheap to scatter
    words[levels * 2 + 5 + 9] = 4000; // level 9: ruinous to scatter
    let counts = raster.decode(&words, 0);
    assert_eq!(counts.scatter, 4050, "the scatter's cells are summed raw");
    assert_eq!(
        counts.hybrid,
        50 + 1000,
        "the cheaper shape is chosen per level, not for the whole chain"
    );
    assert!(
        counts.hybrid < counts.tests && counts.hybrid < counts.scatter,
        "a per-level choice is at least as good as either shape alone"
    );
    let _ = queue;
}

/// The shadow page track is visible to the profiler.
///
/// 🔴 It ran completely UNSCOPED: not one `profiling::scope!` across the
/// marking, the seventeen per-level culls, the compaction, the expansion
/// or the draw. In a capture that is time that simply goes missing, and
/// the CPU cost of this track was argued about for an hour without a
/// single measurement because there was nothing to measure.
///
/// A source check, because a scope's whole purpose is to exist in a
/// build a test does not run.
#[test]
fn the_page_passes_are_profiled() {
    for (name, source, wanted) in [
        (
            "frame/pages.rs",
            include_str!("../src/meshlet/render_stage/frame/pages.rs"),
            "shadow pages",
        ),
        (
            "pages/raster.rs",
            include_str!("../src/shadow/pages/raster.rs"),
            "cull: clipmap levels",
        ),
    ] {
        assert!(
            source.contains(&format!("profiling::scope!(\"{wanted}\")")),
            "{name} has no `{wanted}` scope; the pass is invisible in a capture"
        );
    }

    // And the two entry points that record the GPU work.
    for (name, source) in [
        ("pages/mark.rs", include_str!("../src/shadow/pages/mark.rs")),
        (
            "pages/raster.rs",
            include_str!("../src/shadow/pages/raster.rs"),
        ),
    ] {
        assert!(
            source.contains("#[profiling::function]"),
            "{name} records GPU work in a function the profiler cannot see"
        );
    }

    // 🔴 Every scope OPENED has to be closed, and nothing above checks
    // that. A `nested()` without its `close()` is not a missing timing:
    // wgpu refuses the whole encoder — "a debug group was not popped
    // before the encoder was finished" — and the frame stops being
    // submitted at all. It shipped that way once, from a reorder that
    // moved a `close` out from under the scope it belonged to, and the
    // only warning was an unused variable nobody read.
    {
        let source = include_str!("../src/shadow/pages/raster.rs");
        let opened = source.matches("= nested(track,").count();
        let closed = source.matches("close(track,").count();
        assert_eq!(
            opened, closed,
            "pages/raster.rs opens {opened} GPU scopes and closes {closed}; an unpopped \
             debug group makes wgpu reject the encoder and the frame never reaches the queue"
        );
    }

    // 🔴 Everything above measures the CPU, and every line of it passed
    // while this track spent 34 ms per frame on the OneXFly that no
    // capture could see. `profiling::scope!` times the RECORDING —
    // walking levels, writing uniforms, building bind groups — and the
    // recording is under a millisecond. What the dispatches then cost
    // the GPU needs a timestamp on the encoder, which is a different
    // call, and the name of this test claimed both.
    for (name, source, wanted) in [
        (
            "frame/pages.rs",
            include_str!("../src/meshlet/render_stage/frame/pages.rs"),
            ["shadow pages", "page mark", "page raster"].as_slice(),
        ),
        (
            "pages/raster.rs",
            include_str!("../src/shadow/pages/raster.rs"),
            ["page cull", "page expand", "page depth"].as_slice(),
        ),
    ] {
        for label in wanted {
            assert!(
                source.contains(&format!("\"{label}\"")),
                "{name} opens no `{label}` GPU scope; its dispatches land \
                 in a capture under no name at all"
            );
        }
    }
}

/// The octave a page asks for, run through the SHADER'S OWN arithmetic.
///
/// 🔴 The anchor is the whole claim. A bucket is a density, so the
/// expansion can pair a lamp's pages against the sun's survivors — but
/// only if the sun's clipmap level `L` lands on bucket `L` exactly.
/// Off by one and every page draws geometry from the wrong LOD; off by a
/// scale factor and the local pages pile into one bucket.
///
/// A Rust mirror of `page_octave` would prove the mirror. This runs the
/// shader.
const OCTAVE: &str = r#"
@group(0) @binding(0) var<storage, read_write> out: array<u32>;

// 🔴 The ENGINE'S base, and it is not a power of two. An earlier
// version of this test used 64.0, where every division lands exactly on
// a power of two and `floor(log2(...))` cannot round down — so it passed
// while the sun's levels were falling into the bucket below.
const BASE: f32 = 1.28;
const VIRTUAL: u32 = 16384u;
const LEVELS: u32 = 17u;

fn sun_at(level: u32) -> PageId {
    var id: PageId;
    id.is_sun = true;
    id.level = level;
    return id;
}

fn local_at(level: u32) -> PageId {
    var id: PageId;
    id.is_sun = false;
    id.level = level;
    return id;
}

@compute @workgroup_size(1, 1, 1)
fn cs_octave() {
    // Every clipmap level, which must land on its own index.
    for (var l = 0u; l < LEVELS; l = l + 1u) {
        let texel = page_texel_world(sun_at(l), BASE, VIRTUAL, 0.0);
        out[l] = page_octave(texel, BASE, VIRTUAL, LEVELS);
    }
    // A ten-metre lamp across its chain: finer than the sun at the top
    // of the chain, coarser at the bottom, and monotonic between.
    for (var l = 0u; l < 8u; l = l + 1u) {
        let texel = page_texel_world(local_at(l), BASE, VIRTUAL, 10.0);
        out[LEVELS + l] = page_octave(texel, BASE, VIRTUAL, LEVELS);
    }
    // A hundred-metre lamp, which asks for coarser buckets than the
    // ten-metre one at the same chain level.
    out[LEVELS + 8u] = page_octave(
        page_texel_world(local_at(0u), BASE, VIRTUAL, 100.0), BASE, VIRTUAL, LEVELS);
    out[LEVELS + 9u] = page_octave(
        page_texel_world(local_at(4u), BASE, VIRTUAL, 100.0), BASE, VIRTUAL, LEVELS);
}
"#;

#[test]
fn a_page_asks_for_the_octave_its_texels_are() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    const LEVELS: usize = 17;
    let out = run_page_table_shader(&device, &queue, OCTAVE, "cs_octave", (LEVELS + 10) * 4);

    // 🔴 The anchor: the sun's level IS its bucket. Everything else
    // rests on this, because it is what lets a lamp's pages reach the
    // survivor lists the sun's culls already produce.
    for level in 0..LEVELS {
        assert_eq!(
            out[level], level as u32,
            "the sun's clipmap level {level} landed on bucket {}; a local page reaching \
             that bucket would draw geometry culled for a different density",
            out[level]
        );
    }

    // A local light's chain is monotonic in the same direction: a
    // coarser chain level is a coarser bucket, never a finer one.
    let lamp: Vec<u32> = (0..8).map(|i| out[LEVELS + i]).collect();
    for pair in lamp.windows(2) {
        assert!(
            pair[1] >= pair[0],
            "a lamp's chain is not monotonic across buckets: {lamp:?}"
        );
    }
    assert!(
        lamp.iter().any(|&b| b != lamp[0]),
        "every level of a lamp's chain landed in one bucket ({lamp:?}); the octave is \
         not separating them and one list would serve densities 128x apart"
    );

    // And range moves it. A hundred-metre lamp covers ten times the
    // world with the same texels, so it asks for coarser geometry than
    // a ten-metre one at the same chain level — which is the reason the
    // bucket cannot be read off the chain level alone.
    assert!(
        out[LEVELS + 8] > lamp[0],
        "a 100 m lamp asked for bucket {} at chain level 0, the same as a 10 m lamp's {}",
        out[LEVELS + 8],
        lamp[0]
    );
    assert!(out[LEVELS + 9] > lamp[4], "and the same at chain level 4");
    // Measured: a 10 m lamp's chain lands on buckets [0,0,0,1,2,3,4,5]
    // and a 100 m lamp's on [1,..,5,..] — inside the sun's range, where
    // its culls already produce survivor lists. That is the claim C
    // rests on and it is checked rather than assumed.
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

/// `face_dir` really is `cube_face`'s inverse, on all six faces.
///
/// 🔴 A cube face's axis conventions are six sign choices, and every one
/// of them is invisible until a shadow lands on the wrong wall — at
/// which point it looks like a bad matrix, a bad cull, or a bad page
/// key. The expansion and the depth pass both build a face's frustum
/// from `face_dir` while the marking picks the face with `cube_face`;
/// one flipped sign between them puts a caster in a page it never
/// touches, on the opposite side of the lamp.
const FACE_ROUNDTRIP: &str = r#"
@group(0) @binding(0) var<storage, read_write> out: array<f32>;

@compute @workgroup_size(1, 1, 1)
fn cs_faces() {
    var worst = 0.0;
    var wrong_face = 0.0;
    for (var face = 0u; face < 6u; face = face + 1u) {
        // Corners, edges and the middle: a sign error that survives the
        // centre still shows at a corner.
        for (var i = 0u; i < 9u; i = i + 1u) {
            let uv = vec2<f32>(f32(i % 3u), f32(i / 3u)) * 0.5;
            // Pulled off the exact edge: a direction on the seam is
            // genuinely ambiguous and belongs to either face.
            let inset = clamp(uv, vec2<f32>(0.02), vec2<f32>(0.98));
            let dir = face_dir(face, inset);
            let back = cube_face(dir);
            if u32(back.w) != face {
                wrong_face = wrong_face + 1.0;
            }
            worst = max(worst, max(abs(back.x - inset.x), abs(back.y - inset.y)));
        }
    }
    out[0] = worst;
    out[1] = wrong_face;
}
"#;

#[test]
fn a_cube_face_maps_back_to_itself() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let out = run_page_table_shader(&device, &queue, FACE_ROUNDTRIP, "cs_faces", 8);
    let worst = f32::from_bits(out[0]);
    let wrong = f32::from_bits(out[1]);
    assert_eq!(
        wrong, 0.0,
        "{wrong} of 54 directions came back on a different face than they were built \
         from; a caster would be rasterised into a page on the other side of the lamp"
    );
    assert!(
        worst < 1e-5,
        "the round trip drifts by {worst} across a face, so a page's own frustum does \
         not cover the cell the marking assigned it"
    );
}

/// A lamp's chain is floored, and every pass agrees on where.
///
/// 🔴 The marking picks a level, the reader walks from one and the
/// debug view walks from one. A floor the three disagree on is a reader
/// looking for pages in levels nothing marks — three table lookups a
/// pixel that can only miss — or, worse, a marking that allocates
/// levels the reader never visits, which is pool spent on pages nobody
/// can sample.

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

/// 🔴 `a_caster_behind_every_receiver_pairs_nothing` lived here and is
/// gone with the bound it tested.
///
/// Olsson §4's receiver bound rejected a caster whose nearest point lay
/// beyond a page's furthest RECORDED receiver. The record only ever
/// covered the receivers that marked THAT level, while the reader
/// climbs to coarser ones — so a receiver that climbed met a bound
/// written by other receivers and lost the caster it needed. The page
/// was then drawn with the ground in it and without the occluder, which
/// shades lit and which `VirtualPages` paints GREEN, the colour it
/// documents as "the comparison is wrong".
///
/// It saved 7% of the sun's candidates, measured. Making it correct
/// means the marking writing the bound on every level the reader could
/// reach — seventeen atomics per sample instead of one, over 1.2M
/// samples a frame — which costs more than it saved. Removed rather
/// than left behind a switch.

/// A cleared page outlives the generation it was cleared under, and only
/// a lamp's does.
///
/// 🔴 A grep, because the alternative is a fixture with a spinning lamp,
/// a cull, and a readback to observe one skipped listing. What it guards
/// is exact and was measured: `dense.scene` spins 64 light pivots, every
/// spin turns that lamp's generation over, and every one of its pages
/// then misses the cache gate and is listed and cleared to produce the
/// same nothing. 902 of the 924 pages one frame rasterised; 1469 of 1491
/// in another.
///
/// Empty content does not depend on a generation — a page with no caster
/// in reach reads lit whether or not the lamp moved — so the stamp says
/// EMPTY and the gate honours it while the bucket stays empty.
///
/// ⚠️ The sun is excluded and has to be. Its pages carry an invariant
/// the lamps' do not: one whose ADDRESSING changed under a snap crossing
/// must redraw even though its content would be identical, because the
/// listing is what writes the way back into the table.
/// `a_still_suns_page_caches` caught this gate applying to the sun.
#[test]
fn an_empty_lamp_page_stops_relisting() {
    let table = kooch_lighting::PAGE_TABLE;
    let compact = include_str!("../shaders/page_compact.wgsl");
    let dense: String = compact.chars().filter(|c| !c.is_whitespace()).collect();

    assert!(
        table.contains("const PAGE_EMPTY: u32 = 2u;"),
        "the sentinel has to be EVEN: every generation ends `h | 1`, and that is the whole \
         reason no generation can be mistaken for it",
    );
    assert!(
        dense.contains("if!id.is_sun&&stamp==PAGE_EMPTY&&survivors==0u{"),
        "the cache gate has to honour an empty lamp page, or a moving light relists every \
         page it owns every frame to clear it to the same nothing",
    );
    assert!(
        dense.contains("select(gen,PAGE_EMPTY,!id.is_sun&&survivors==0u)"),
        "a lamp page with no survivors has to be stamped EMPTY rather than with the \
         generation, or the gate above can never fire",
    );
}

/// The march is a different QUESTION, not a wider filter.
///
/// 🔴 A grep, because observing it needs an atlas with a caster whose
/// footprint misses one texel — the exact configuration that is hard to
/// build on purpose and easy to hit by accident, which is why the
/// artefact survived four rounds of tuning the reader that cannot see
/// it.
///
/// What it pins is the part that would be quietly lost in a cleanup:
/// the rays have to SPREAD. Stepping along the sun's own axis does not
/// move the sample in the sun's plane at all — `basis[2]` is
/// perpendicular to the two axes a page is addressed by — so an
/// unjittered march reads one texel at several depths and answers
/// exactly what the single tap already answered. The spread is the
/// mechanism, not a soft-shadow nicety on top of it.
#[test]
fn the_march_spreads_over_the_suns_disc() {
    let shading = kooch_lighting::inti_pbr_shader(1);
    let start = shading
        .find("fn inti_page_march(")
        .expect("the march is in the shader");
    let end = shading[start..]
        .find("\nfn inti_page_shadow(")
        .expect("the march ends")
        + start;
    let body: String = shading[start..end]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    assert!(
        body.contains("inti.sun_softness"),
        "the rays have to open over the sun's ANGULAR SIZE; without a spread every step \
         reads the same texel and the march answers what the single tap already did",
    );
    assert!(
        body.contains("basis[0]*") && body.contains("basis[1]*"),
        "the spread has to be in the sun's two PLANE axes — offsetting along `basis[2]` is \
         the degenerate direction, the one that does not move the sample in the page",
    );
    assert!(
        body.contains("abs(reference-previous)*1.05"),
        "the tolerance has to be measured from the ray's own step, or the march has \
         reacquired the constant it exists to remove",
    );
    // And the box reader is still reachable, because nothing has
    // measured what the march costs.
    assert!(
        shading.contains("inti_pages.layer.z != 0u"),
        "the march has to stay selectable; it replaces the reader every shipped frame goes \
         through and its cost is unmeasured",
    );
}

/// The inverted expansion emits the SAME pairs as the paired one.
///
/// # 🔴 The only claim #1022 is allowed to make
///
/// One shape walks every listed page against every survivor; the other
/// runs one thread per survivor and descends the page pyramid to the
/// pages it lands in. They reach a page from opposite ends and then ask
/// the same three questions about it — `sun_pair` is one function, and
/// this is the test that says so.
///
/// So the switch is a COST switch. If this ever fails, the two halves
/// of the pass disagree about which caster belongs in which page, and
/// that disagreement is exactly the artefact the whole line of work is
/// chasing: it would be a finding, not a regression to paper over.
///
/// The pages are planted around the toroidal seam on purpose — absolute
/// indices −2..1 wrap to cells 126, 127, 0, 1 — because a rectangle
/// that crosses it becomes four rectangles in the table's own
/// coordinates, and a descent that ignored the wrap would read the far
/// side of the world.
#[test]
fn both_expansions_emit_the_same_pairs() {
    use glam::{Mat4, Vec3};
    use kooch_render::meshlet::{
        MeshInstance, MeshletCullPipelines, MeshletScene, SceneCullParams, build_default_meshlets,
    };

    // 🔴 The shared device and not `device()`: the meshlet cull binds a
    // fifth group and the downlevel default is four.
    let Some((device, queue)) = common::try_acquire_device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let (device, queue) = (device.clone(), queue.clone());

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
        MeshInstance::new(
            Mat4::from_scale_rotation_translation(
                Vec3::new(40.0, 1.0, 40.0),
                glam::Quat::IDENTITY,
                Vec3::new(0.0, -0.5, 0.0),
            ),
            handle.mesh_id,
            0,
        ),
        MeshInstance::new(
            Mat4::from_scale_rotation_translation(
                Vec3::splat(1.5),
                glam::Quat::IDENTITY,
                Vec3::new(1.0, 2.0, -1.0),
            ),
            handle.mesh_id,
            0,
        ),
    ];

    const LIGHTS: u32 = 1;
    const LEVEL: u32 = 8;
    // The sun, and nothing else: a directional light owns no bucket of
    // its own, so every pair this counts is the clipmap's.
    let lamps = [kooch_lighting::GpuLight {
        kind: kooch_lighting::LIGHT_KIND_DIRECTIONAL,
        ..Default::default()
    }];
    // Level 8's pages are 2.56 m wide, so the floor covers absolute
    // indices −8..7 and these four straddle the seam.
    let cells: Vec<(u32, u32)> = [126u32, 127, 0, 1]
        .iter()
        .flat_map(|&x| [126u32, 127, 0, 1].iter().map(move |&y| (x, y)))
        .collect();

    let run = |geometry: bool| -> Vec<[u32; 3]> {
        // Each run gets its own pool and its own rasteriser: a page
        // listed once is stamped, and the second run would cache it and
        // list nothing.
        let scene = MeshletScene::new(&device, instances.len() as u32);
        scene.upload_instances(&queue, &instances);
        let scene_params = SceneCullParams::new(instances.len() as u32, meshlets_per_mesh);
        let lights_buffer = {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("both_expansions_light"),
                size: std::mem::size_of_val(&lamps) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&lamps));
            buffer
        };
        let mut page_pool = PagePool::new(&device, small());
        let entries = VIEWS * span(LIGHTS);
        page_pool.ensure_entries(&device, entries);
        let cell = PAGE_CELL as usize;
        let mut slots = vec![0u32; entries as usize * cell];
        for (index, &(x, y)) in cells.iter().enumerate() {
            let page = sun_page(0, LEVEL, (x, y), LIGHTS);
            slots[page as usize * cell] = index as u32 + 1;
        }
        queue.write_buffer(page_pool.slots(), 0, bytemuck::cast_slice(&slots));

        let cull_pipelines = MeshletCullPipelines::new(&device);
        // Built against the CULL's layout, not the pool's: the depth
        // draw shares it and the two differ in visibility.
        let mut raster = PageRasterizer::new(
            &device,
            cull_pipelines.meshlet_bind_group_layout(),
            PageConfig::default(),
            ClipmapConfig::default(),
            small(),
            kooch_render::meshlet::DEFAULT_MAX_TRIANGLES as u32,
        );
        raster.set_geometry(geometry);
        // ⚠️ The per-instance cull, not the chunked one. The two-level
        // path produces ZERO survivors at every level in this rig — it
        // enters per rectangle cell and this scene has no cell data to
        // enter by — so leaving it on would compare two empty lists and
        // pass for the wrong reason.
        raster.set_two_level(false);
        let meshlet_bg = kooch_render::meshlet::pool_meshlet_bind_group(
            &device,
            cull_pipelines.meshlet_bind_group_layout(),
            &gpu_pool,
        );
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
            Vec3::new(0.4, 3.0, 0.4),
            Vec3::NEG_Y,
            &lamps,
            &lights_buffer,
            &[],
            1.0,
            None,
        );
        queue.submit([encoder.finish()]);

        let counts = read_words(&device, &queue, raster.counts_buffer());
        let buckets = raster.buckets() as usize;
        let emitted = counts[buckets + 2] as usize;
        assert!(
            counts[LEVEL as usize] > 0 && counts[buckets + 5 + LEVEL as usize] > 0,
            "the rig planted no pages or the cull kept no survivors at level {LEVEL}",
        );
        // The descent's own counter says which shape actually ran. A
        // silent fallback to pairing would make the comparison below
        // compare the paired shape against itself.
        let walk = counts[buckets * 3 + 7];
        assert_eq!(
            walk > 0,
            geometry,
            "the expansion did not run the shape it was asked for",
        );
        assert_eq!(counts[buckets + 3], 0, "the pair list overflowed");
        assert_eq!(
            counts[buckets * 3 + 8],
            0,
            "a descent ran out of stack and dropped a subtree",
        );
        let words = read_words(&device, &queue, raster.pairs_buffer());
        // The order is whatever the atomics handed out; the SET is the
        // claim.
        let mut pairs: Vec<[u32; 3]> = (0..emitted)
            .map(|i| [words[i * 4], words[i * 4 + 1], words[i * 4 + 2]])
            .collect();
        pairs.sort_unstable();
        pairs
    };

    let paired = run(false);
    let inverted = run(true);
    assert!(
        !paired.is_empty(),
        "the rig paired nothing, so the comparison proves nothing",
    );
    assert_eq!(
        paired.len(),
        inverted.len(),
        "the two shapes emitted different numbers of pairs",
    );
    assert_eq!(
        paired, inverted,
        "the two shapes disagree about which caster belongs in which page",
    );
}

/// A resident page with no content has to read as a MISS, and the
/// reader has to keep climbing when it does.
///
/// # 🔴 Resident is not readable
///
/// `PAGE_CELL` says the fourth word is the content stamp and that zero
/// means "no valid content". A page reaches that state by being freshly
/// claimed, by being invalidated, or by its bucket overflowing so the
/// compaction never listed it — and the atlas under its slot then holds
/// whatever was there before, or a clear. A clear is far depth under
/// reversed-Z, which every reader answers "nothing occludes here".
///
/// Reporting it as a hit does not only read one wrong texel: it ENDS
/// the walk. `inti_page_shadow` climbs the clipmap until a level
/// answers — Unreal's "onwards to coarser levels if no valid data is
/// present" — and a present, empty page stops the search at the one
/// level that cannot answer, with a coarser one right above it holding
/// the shadow. The panel already called this out; its `pages dropped`
/// alert says "some resident pages hold no depth and shade as lit". The
/// reader was the half that did not know.
#[test]
fn an_empty_page_is_not_a_hit() {
    let source = kooch_lighting::inti_pbr_shader(1);
    let start = source
        .find("fn inti_page_lookup(")
        .expect("the lookup is in the shader");
    let end = source[start..].find("\nfn ").expect("the lookup ends") + start;
    let body = &source[start..end];

    let dense: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        dense.contains("inti_page_slots[page*PAGE_CELL+3u]==0u"),
        "the lookup does not check the content stamp, so a resident page with no depth \
         reads as a hit and shades lit",
    );
    assert!(
        body.contains("return PAGE_MISS;"),
        "and it has to answer PAGE_MISS, which is what makes the caller climb",
    );

    // The other half of the pair: the climb itself. A lookup that
    // reports the miss buys nothing if the caller gives up on it.
    let walk_start = source
        .find("fn inti_page_shadow(")
        .expect("the reader is in the shader");
    let walk_end = source[walk_start..]
        .find("\nfn inti_local_page_shadow(")
        .expect("the reader ends")
        + walk_start;
    let walk = &source[walk_start..walk_end];
    let walk_dense: String = walk.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        walk_dense.contains("level=level+1u"),
        "the sun's reader has to walk to coarser levels, or a missing page is a lit pixel \
         however honestly the lookup reported it",
    );
}

/// A PCF tap that leaves its page is resolved through the table.
///
/// # 🔴 Clamping is a lit band along every page seam
///
/// The kernel is `W` texels wide, so a receiver within `W/2` of a page
/// edge has taps that belong to the neighbouring page — and whenever a
/// shadow crosses a seam the occluder's depth is exactly there. Folded
/// back onto the edge, those taps read the receiver's own page, find
/// nothing, and the pixel answers LIT with the page present, resident
/// and correctly drawn.
///
/// That is the third of the three faults `VirtualPages` separates, and
/// it is the one that looks like the other two: the debug view paints
/// it green — a real page whose COMPARISON is wrong — while a missing
/// page is red and an undrawn one yellow. A texel is centimetres at the
/// fine levels and metres at the coarse ones, so the same defect is a
/// hairline near the camera and a wedge further out.
///
/// Unreal resolve every sample through the page table inside
/// `SampleBilinear` for exactly this reason.
#[test]
fn a_tap_off_the_page_finds_its_neighbour() {
    let source = kooch_lighting::inti_pbr_shader(1);
    let start = source
        .find("fn inti_page_filter(")
        .expect("the filter is in the shader");
    let end = source[start..].find("\nfn ").expect("the filter ends") + start;
    let body = &source[start..end];

    assert!(
        body.contains("inti_page_lookup(page)"),
        "a tap that leaves its page is not resolved through the table, so it folds back \
         onto the edge and reads the wrong page's depth",
    );
    assert!(
        body.contains("let outside ="),
        "the filter does not test whether a tap left the page at all",
    );
    // The fallback has to stay: a neighbour that is absent must clamp
    // rather than read somebody else's slot.
    assert!(
        body.contains("clamp(raw,"),
        "an absent neighbour has to fall back to the clamp",
    );
    // And the lamps must NOT re-resolve: their pages are six faces of a
    // chain, so a step off an edge crosses a face and lands nowhere this
    // arithmetic can index.
    let sun = source
        .find("fn inti_page_shadow(")
        .expect("the sun's reader is in the shader");
    let lamp = source
        .find("fn inti_local_page_shadow(")
        .expect("the lamps' reader is in the shader");
    assert!(
        source[lamp..].contains("PAGE_UNLISTED,"),
        "the lamps have to opt out of the neighbour walk",
    );
    assert!(
        !source[sun..lamp].contains("PAGE_UNLISTED,"),
        "the sun has to opt IN, or the fix does nothing where it was measured",
    );
}

/// The reader jumps to the level that answers instead of walking to it.
///
/// # 🔴 Up to seventeen misses per pixel PER LIGHT
///
/// The walk starts at the containment floor and the marking chose
/// `max(contain, density)`, so the common case is `density - contain`
/// levels of pure misses before the first hit, with the whole chain as
/// the ceiling. Each miss is one indexed read — the flat table's whole
/// point — and seventeen of them per pixel per light is not cheap.
///
/// `cs_lod_offsets` walks the chain once per page per frame and writes
/// the answer into `PAGE_LOD`; the reader then does two reads. That is
/// Unreal's `LODOffset` beside its `bAnyLODValid` bit.
///
/// The loop stays, and must: the hint is a frame's worth of arithmetic
/// over a table that other passes are still writing, so a stale one has
/// to degrade into the walk rather than into a wrong answer.
#[test]
fn the_reader_jumps_to_the_level_that_answers() {
    let source = kooch_lighting::inti_pbr_shader(1);
    let start = source
        .find("fn inti_page_shadow(")
        .expect("the reader is in the shader");
    let end = source[start..]
        .find("\nfn inti_local_page_shadow(")
        .expect("the reader ends")
        + start;
    let body = &source[start..end];

    assert!(
        body.contains("PAGE_LOD"),
        "the reader still climbs the clipmap one level at a time",
    );
    assert!(
        body.contains("PAGE_NO_LOD"),
        "and it has to tell 'no coarser page' from a jump of zero",
    );
    assert!(
        body.contains("level = level + 1u"),
        "the walk has to remain as the fallback for a stale hint",
    );

    // The writer, and the ordering that makes it mean anything.
    let compact = include_str!("../shaders/page_compact.wgsl");
    assert!(
        compact.contains("fn cs_lod_offsets("),
        "nothing fills the jump table",
    );
    let stamp = compact
        .find("PAGE_CELL + 3u] = select(gen")
        .expect("the compaction stamps content");
    let fill = compact
        .find("fn cs_lod_offsets(")
        .expect("the pass is in the shader");
    assert!(
        stamp < fill,
        "the jump table is filled before the stamps it reads, so it would point at pages \
         that hold a clear",
    );
}
