use super::*;
use crate::shadow::pages::pool::PAGE_CELL;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: None,
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults(),
        ..Default::default()
    }))
    .ok()
}

/// The `page_list` index the fixture's one listed page carries. An
/// arbitrary number on purpose: the pyramid has to hand back what the
/// compaction wrote, not a bit that happens to be set.
const LISTING: u32 = 37;

/// A table with exactly one LISTED page, at `(level, x, y)`.
fn table_with(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    side: u32,
    levels: u32,
    at: (u32, u32, u32),
) -> wgpu::Buffer {
    let entries = (side * side * levels) as usize;
    // Every entry starts UNLISTED, which is what the compaction leaves
    // behind for the pages it did not list. Zero would read as
    // "listing 0" and put every cached page in the pyramid.
    let mut words = vec![0u32; entries * PAGE_CELL as usize];
    for entry in 0..entries {
        words[entry * PAGE_CELL as usize + 2] = u32::MAX;
    }
    let page = at.0 * side * side + at.2 * side + at.1;
    // Entries store `slot + 1`, so any non-zero word is resident.
    words[page as usize * PAGE_CELL as usize] = 1;
    words[page as usize * PAGE_CELL as usize + 2] = LISTING;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (words.len() * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&words));
    buffer
}

/// One mip of one layer, read back with the row padding wgpu requires.
fn read_mip(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    mip: u32,
    layer: u32,
    side: u32,
) -> Vec<u32> {
    let row = (side * 4).div_ceil(256) * 256;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * side) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: mip,
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
                bytes_per_row: Some(row),
                rows_per_image: Some(side),
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
    let raw = staging.slice(..).get_mapped_range().to_vec();
    let mut out = Vec::with_capacity((side * side) as usize);
    for y in 0..side {
        let start = (y * row) as usize;
        out.extend_from_slice(bytemuck::cast_slice::<u8, u32>(
            &raw[start..start + (side * 4) as usize],
        ));
    }
    out
}

/// One resident page has to light every texel above it, and only those.
///
/// 🔴 Both halves matter and they fail differently. A missing ancestor
/// is a caster the overlap test will reject — geometry that silently
/// stops being drawn into a page that asked for it, which is the exact
/// shape of the artefact this whole line of work is chasing. A spurious
/// one is only wasted raster. The first is why the structure has to be
/// proven before anything reads it.
#[test]
fn a_listed_page_lights_its_ancestors() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    // Small enough to read back whole, still a real chain of five mips.
    let config = PageConfig {
        page: 128,
        virtual_size: 128 * 16,
        ..PageConfig::default()
    };
    let clipmap = ClipmapConfig {
        base: 1.28,
        levels: 3,
    };
    let side = config.side(0);
    assert_eq!(side, 16, "the fixture wanted a 16x16 grid");

    let pyramid = PagePyramid::new(&device, config, clipmap);
    let at = (1u32, 5u32, 9u32);
    let table = table_with(&device, &queue, side, clipmap.levels, at);

    let mut encoder = device.create_command_encoder(&Default::default());
    pyramid.build(&device, &queue, &mut encoder, &table, 0);
    queue.submit([encoder.finish()]);

    for mip in 0..PagePyramid::mip_count(side) {
        let mip_side = (side >> mip).max(1);
        let texels = read_mip(&device, &queue, pyramid.texture(), mip, at.0, mip_side);
        let want = ((at.2 >> mip) * mip_side + (at.1 >> mip)) as usize;
        assert_eq!(
            texels[want],
            LISTING + 1,
            "mip {mip} lost the page at ({}, {}) — an ancestor that reads 0 is a caster the \
             overlap test rejects, and the geometry stops being drawn with nothing failing",
            at.1,
            at.2
        );
        let lit = texels.iter().filter(|&&t| t != 0).count();
        assert_eq!(
            lit, 1,
            "mip {mip} lit {lit} texels for one listed page — wasted raster, and it means \
             the reduction read outside its own block",
        );
    }

    // A level nobody marked stays dark, or the pyramid is answering for
    // the wrong clipmap level entirely.
    let other = read_mip(&device, &queue, pyramid.texture(), 0, 0, side);
    assert!(
        other.iter().all(|&t| t == 0),
        "level 0 lit up for a page marked on level 1",
    );
}

/// A page that is RESIDENT but not listed has to stay dark.
///
/// 🔴 The failure this guards is not an artefact, it is the cache. Most
/// of the atlas is resident at any moment and almost none of it is
/// listed — listing is the compaction's decision that a page redraws
/// this frame. A pyramid seeded on residency answers `true` for every
/// cached page, the inverted expansion pairs against all of them, and
/// the frame rasterises the whole atlas every frame while every counter
/// reports health. That is #477 undone in one texture.
#[test]
fn a_cached_page_stays_dark() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let config = PageConfig {
        page: 128,
        virtual_size: 128 * 16,
        ..PageConfig::default()
    };
    let clipmap = ClipmapConfig {
        base: 1.28,
        levels: 1,
    };
    let side = config.side(0);
    let entries = (side * side * clipmap.levels) as usize;
    let mut words = vec![0u32; entries * PAGE_CELL as usize];
    // Every page resident, every one of them cached: a slot in the
    // pool, no listing.
    for entry in 0..entries {
        words[entry * PAGE_CELL as usize] = entry as u32 + 1;
        words[entry * PAGE_CELL as usize + 2] = u32::MAX;
    }
    let table = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (words.len() * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&table, 0, bytemuck::cast_slice(&words));

    let pyramid = PagePyramid::new(&device, config, clipmap);
    let mut encoder = device.create_command_encoder(&Default::default());
    pyramid.build(&device, &queue, &mut encoder, &table, 0);
    queue.submit([encoder.finish()]);

    let top = PagePyramid::mip_count(side) - 1;
    let texels = read_mip(&device, &queue, pyramid.texture(), top, 0, 1);
    assert_eq!(
        texels[0], 0,
        "a full, entirely cached atlas lit the pyramid — every page would be rasterised again",
    );
}

/// Rectangles to ask about, and the answer the pyramid gives for each.
const PROBE: &str = r#"
@group(0) @binding(0) var pyramid: texture_2d_array<u32>;
@group(0) @binding(1) var<storage, read> rects: array<vec4<u32>>;
@group(0) @binding(2) var<storage, read_write> answers: array<u32>;
@group(0) @binding(3) var<uniform> shape: vec4<u32>;

@compute @workgroup_size(64, 1, 1)
fn cs_probe(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= arrayLength(&rects) {
        return;
    }
    let hit = overlaps_any_page(pyramid, rects[gid.x], shape.x, shape.y);
    // The mip rides along so the test can pin the CHOICE, not only the
    // answer: too high a mip is still safe and still wrong.
    answers[gid.x] = select(0u, 1u, hit) | (overlap_mip(rects[gid.x], shape.y) << 1u);
}
"#;

/// The constant-time answer may never miss a resident page.
///
/// 🔴 The two directions are not equally bad and the assertions say so.
/// A FALSE NEGATIVE drops a caster: the expansion never pairs it with
/// the page it belongs in, the page is drawn without it, and the shadow
/// is missing with every counter reporting health. A false positive is
/// one pair tested and discarded. So the first is checked exhaustively
/// against the slow answer, and the second only has to stay away from
/// the degenerate case — a function that returned `true` always would
/// satisfy the safety property and be worthless.
#[test]
fn the_pyramid_never_misses_a_page() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let config = PageConfig {
        page: 128,
        virtual_size: 128 * 16,
        ..PageConfig::default()
    };
    let clipmap = ClipmapConfig {
        base: 1.28,
        levels: 2,
    };
    let side = config.side(0);
    let mips = PagePyramid::mip_count(side);
    let level = 0u32;
    // Scattered on purpose: one in a corner, one that shares a mip-2
    // block with nothing, one adjacent to another so a block holds two.
    let resident = [(0u32, 0u32), (5u32, 9u32), (6u32, 9u32), (15u32, 2u32)];

    let entries = (side * side * clipmap.levels) as usize;
    let mut words = vec![0u32; entries * PAGE_CELL as usize];
    for entry in 0..entries {
        words[entry * PAGE_CELL as usize + 2] = u32::MAX;
    }
    for (listing, &(x, y)) in resident.iter().enumerate() {
        let page = level * side * side + y * side + x;
        words[page as usize * PAGE_CELL as usize] = 1;
        words[page as usize * PAGE_CELL as usize + 2] = listing as u32;
    }
    let table = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (words.len() * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&table, 0, bytemuck::cast_slice(&words));

    let pyramid = PagePyramid::new(&device, config, clipmap);
    let mut encoder = device.create_command_encoder(&Default::default());
    pyramid.build(&device, &queue, &mut encoder, &table, 0);
    queue.submit([encoder.finish()]);

    // Every rectangle up to 4 pages a side, everywhere on the grid.
    let mut rects: Vec<[u32; 4]> = Vec::new();
    for y in 0..side {
        for x in 0..side {
            for size in 1..=4u32 {
                rects.push([
                    x,
                    y,
                    (x + size - 1).min(side - 1),
                    (y + size - 1).min(side - 1),
                ]);
            }
        }
    }
    let truth: Vec<bool> = rects
        .iter()
        .map(|r| {
            resident
                .iter()
                .any(|&(x, y)| x >= r[0] && x <= r[2] && y >= r[1] && y <= r[3])
        })
        .collect();

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("page_overlap_probe"),
        source: wgpu::ShaderSource::Wgsl(format!("{OVERLAP}\n{PROBE}").into()),
    });
    let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            storage(1, true),
            storage(2, false),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
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
        entry_point: Some("cs_probe"),
        compilation_options: Default::default(),
        cache: None,
    });

    let flat: Vec<u32> = rects.iter().flatten().copied().collect();
    let rect_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (flat.len() * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&rect_buf, 0, bytemuck::cast_slice(&flat));
    let out = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (rects.len() * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let shape = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&shape, 0, bytemuck::cast_slice(&[level, mips, 0u32, 0u32]));
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(pyramid.view()),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: rect_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: out.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: shape.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups((rects.len() as u32).div_ceil(64), 1, 1);
    }
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (rects.len() * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(&out, 0, &staging, 0, (rects.len() * 4) as u64);
    queue.submit([encoder.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    let answers: Vec<u32> =
        bytemuck::cast_slice::<u8, u32>(&staging.slice(..).get_mapped_range()).to_vec();

    // The smallest mip at which both axes collapse to two texels — the
    // definition, spelled out here so the shader's bit trick is checked
    // against the thing it is a trick FOR.
    let wanted_mip = |r: &[u32; 4]| {
        (0..mips)
            .find(|m| (r[2] >> m) - (r[0] >> m) <= 1 && (r[3] >> m) - (r[1] >> m) <= 1)
            .unwrap_or(mips - 1)
    };

    let mut over = 0usize;
    for (i, rect) in rects.iter().enumerate() {
        let said = answers[i] & 1 != 0;
        assert_eq!(
            answers[i] >> 1,
            wanted_mip(rect),
            "rect {rect:?} was answered at the wrong mip — too high is still safe and still              reads a block bigger than it needs, which is pairs tested for nothing",
        );
        assert!(
            !truth[i] || said,
            "rect {rect:?} contains a resident page and the pyramid denied it — that is a \
             caster the expansion never pairs, a page drawn without it, and a shadow \
             missing with every counter reporting health",
        );
        if said && !truth[i] {
            over += 1;
        }
    }
    // A function returning `true` always would pass the loop above.
    let loose = over as f32 / rects.len() as f32;
    assert!(
        loose < 0.5,
        "{:.0}% of rectangles were over-reported; the block granularity is meant to cost a \
         few discarded pairs, not to answer yes to everything",
        loose * 100.0
    );
}
