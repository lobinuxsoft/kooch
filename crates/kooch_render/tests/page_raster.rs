//! The paged depth raster (#866).
//!
//! The first test is the one that matters most: four shaders share
//! `page_table.wgsl`, and a page id encoded one way and decoded another
//! rasterises geometry into somebody else's page. A compile failure
//! belongs here rather than in a frame.

use kooch_render::meshlet::GpuGlobalMeshPool;
use kooch_render::shadow::pages::pool::{PagePool, PoolConfig};
use kooch_render::shadow::pages::raster::{PAGE_DEPTH_FORMAT, PageRasterizer};
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
        64,
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
    let levels = ClipmapConfig::default().levels;
    // Per level, then bucket overflow, local pages, pairs, pair
    // overflow, pages owned by another camera.
    assert_eq!(raster.count_slots(), levels + 5);
    let mut words = vec![0u32; raster.count_slots() as usize];
    words[0] = 7;
    words[1] = 5;
    words[levels as usize + 1] = 42;
    words[levels as usize + 2] = 900;
    words[levels as usize + 4] = 31;
    let counts = raster.decode(&words, 1);
    assert_eq!(counts.pages, 12, "levels sum");
    assert_eq!(counts.local, 42, "local pages are reported, not hidden");
    assert_eq!(counts.pairs, 900);
    assert_eq!(counts.others, 31, "the other camera's pages are named");
    assert_eq!(counts.view, 1);
}

/// Pages one light addresses. Recomputed from the public config rather
/// than read off the marking pass, so the two derivations have to agree.
fn stride(config: PageConfig, clipmap: ClipmapConfig) -> u32 {
    let local = config.face_pages() * 6;
    let sun = clipmap.levels * config.side(0).pow(2);
    // A multiple of 32, so a camera's bits start on a word boundary and
    // its region of the mark bitmap can be cleared on its own.
    local.max(sun).div_ceil(32) * 32
}

/// Pages one camera addresses: every light plus the sun.
fn span(lights: u32) -> u32 {
    (lights + 1) * stride(PageConfig::default(), ClipmapConfig::default())
}

/// The virtual page `mark_sun` would write for this camera, level and
/// cell.
fn sun_page(view: u32, level: u32, cell: (u32, u32), lights: u32) -> u32 {
    let config = PageConfig::default();
    let clipmap = ClipmapConfig::default();
    let side = config.side(0);
    view * span(lights)
        + lights * stride(config, clipmap)
        + level * side * side
        + cell.1 * side
        + cell.0
}

/// A page belonging to light 0, which this raster does not draw.
fn local_page(view: u32, level: u32, cell: (u32, u32), lights: u32) -> u32 {
    let config = PageConfig::default();
    let side = config.side(level);
    let base: u32 = (0..level).map(|l| config.side(l).pow(2)).sum();
    view * span(lights) + base + cell.1 * side + cell.0
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
    // belonging to the OTHER camera. Keys are `page + 1`; where they
    // sit in the table is the hash's business and compaction reads all
    // of it.
    const VIEW: u32 = 1;
    let planted = [
        (sun_page(VIEW, 0, (3, 4), LIGHTS), 11u32),
        (sun_page(VIEW, 0, (5, 6), LIGHTS), 12),
        (sun_page(VIEW, 5, (7, 8), LIGHTS), 13),
    ];
    let mut keys = vec![0u32; small().entries() as usize];
    let mut slots = vec![0u32; small().entries() as usize];
    for (i, (page, slot)) in planted.iter().enumerate() {
        keys[i * 7] = page + 1;
        slots[i * 7] = *slot;
    }
    keys[97] = local_page(VIEW, 2, (1, 1), LIGHTS) + 1;
    slots[97] = 20;
    // 🔴 The other camera's pages, on levels this one also uses. Before
    // the view entered the key these were indistinguishable, and each
    // camera rasterised the other's clipmap with its own matrices.
    keys[43] = sun_page(0, 0, (3, 4), LIGHTS) + 1;
    slots[43] = 30;
    keys[61] = sun_page(0, 5, (7, 8), LIGHTS) + 1;
    slots[61] = 31;
    queue.write_buffer(pool.keys(), 0, bytemuck::cast_slice(&keys));
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
        64,
    );
    queue.submit([encoder.finish()]);

    let counts = read_words(&device, &queue, raster.counts_buffer());
    assert_eq!(counts[0], 2, "two pages on level 0");
    assert_eq!(counts[5], 1, "one page on level 5");
    assert_eq!(counts[levels as usize], 0, "no bucket overflowed");
    assert_eq!(
        counts[levels as usize + 1],
        1,
        "the local light's page is counted, not silently dropped"
    );
    assert_eq!(
        counts[levels as usize + 4],
        2,
        "the other camera's pages are counted and left alone"
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
}
