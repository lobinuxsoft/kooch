//! The paged depth raster (#866).

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

/// A pool small enough that the atlas is megabytes rather than a quarter of a gigabyte: every test
/// here is about arithmetic, not about capacity.
fn small() -> PoolConfig {
    PoolConfig {
        pages: 64,
        views: VIEWS,
        row_cap: u32::MAX,
    }
}

/// Cameras the pool is sliced between. Two, because one is the case that never showed the bug.
const VIEWS: u32 = 2;

fn rasterizer(device: &wgpu::Device) -> PageRasterizer {
    let bgl = GpuGlobalMeshPool::bind_group_layout(device);
    PageRasterizer::new(
        device,
        &bgl,
        PageConfig::default(),
        ClipmapConfig::default(),
        small(),
        // 🔴 The engine's own cap, not a round number. The test used to pass 64 and the builder
        // really emits meshlets of up to 98 triangles, so a helper configured for less would have
        // hidden the very thing `the_draw_covers_a_whole_meshlet` asserts.
        kooch_render::meshlet::DEFAULT_MAX_TRIANGLES as u32,
    )
}

/// Pages one LOCAL light addresses. Recomputed from the public config rather than read off the
/// marking pass, so the two derivations have to agree: six faces of a chain from the floor up, on a
/// word boundary.
fn stride(config: PageConfig, _clipmap: ClipmapConfig) -> u32 {
    (config.local_face_pages() * 6).div_ceil(32) * 32
}

/// Light slots the address space is laid out for. Mirrors `padded_lights`: the layout pads so
/// adding a light does not move every page id.
fn padded(lights: u32) -> u32 {
    lights.max(1).next_multiple_of(64)
}

/// Pages one camera addresses: the padded light slots, then the sun's clipmap at the tail.
fn span(lights: u32) -> u32 {
    let config = PageConfig::default();
    let clipmap = ClipmapConfig::default();
    (padded(lights) * stride(config, clipmap) + clipmap.levels * config.side(0).pow(2)).div_ceil(32)
        * 32
}

/// The virtual page `mark_sun` would write for this camera, level and cell.
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

/// A table entry that is resident but not in this view's `page_list`.
/// Mirrors `PAGE_UNLISTED` in `page_table.wgsl`.
const PAGE_UNLISTED: u32 = 0xffff_ffff;

/// A page belonging to light 0 on an explicit cube FACE.
fn lamp_face_page(view: u32, face: u32, level: u32, cell: (u32, u32), lights: u32) -> u32 {
    local_page(view, level, cell, lights) + face * PageConfig::default().local_face_pages()
}

#[path = "page_raster/sizing.rs"]
mod sizing;
#[path = "page_raster/compaction.rs"]
mod compaction;
#[path = "page_raster/lamp_pages.rs"]
mod lamp_pages;
#[path = "page_raster/geometry.rs"]
mod geometry;
#[path = "page_raster/frame.rs"]
mod frame;
#[path = "page_raster/page_table.rs"]
mod page_table;
#[path = "page_raster/reader.rs"]
mod reader;
