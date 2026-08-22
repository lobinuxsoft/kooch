//! The paged depth raster (#866).
//!
//! The first test is the one that matters most: four shaders share
//! `page_table.wgsl`, and a page id encoded one way and decoded another
//! rasterises geometry into somebody else's page. A compile failure
//! belongs here rather than in a frame.

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
    let levels = ClipmapConfig::default().levels;
    // Per level, then bucket overflow, local pages, pairs, pair
    // overflow, pages owned by another camera — and then a second run
    // per level for the survivors each cull produced, which is the other
    // half of the expansion's cost.
    assert_eq!(raster.count_slots(), levels * 2 + 5);
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
    // TWO words an entry — the slot, then its age. See `PAGE_CELL`.
    let cell = PAGE_CELL as usize;
    let mut slots = vec![0u32; small().entries() as usize * cell];
    for (i, (page, slot)) in planted.iter().enumerate() {
        keys[i * 7] = page + 1;
        slots[i * 7 * cell] = *slot;
    }
    keys[97] = local_page(VIEW, 2, (1, 1), LIGHTS) + 1;
    slots[97 * cell] = 20;
    // 🔴 A tombstone, on a level this camera uses. An evicted entry is
    // not an empty one — `PAGE_DEAD - 1` decodes into a well-formed page
    // that stands for nothing — and compaction that only skips EMPTY
    // rasterises it. This planted one is the whole reason the test
    // exists in this shape.
    keys[71] = 0xffff_fffe;
    slots[71 * cell] = 42;
    // 🔴 The other camera's pages, on levels this one also uses. Before
    // the view entered the key these were indistinguishable, and each
    // camera rasterised the other's clipmap with its own matrices.
    keys[43] = sun_page(0, 0, (3, 4), LIGHTS) + 1;
    slots[43 * cell] = 30;
    keys[61] = sun_page(0, 5, (7, 8), LIGHTS) + 1;
    slots[61 * cell] = 31;
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

    // Centre at the origin: the snap is irrelevant to winding, which is
    // what this measures.
    let rect = sun_page_rect(0u, vec2<u32>(0u, 0u), 64.0, 128u, vec2<f32>(0.0));
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
        1,
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
/// narrow and exact: `INTI_NORMAL_BIAS` has to be multiplied by a
/// per-level texel size inside the walk, and the comparison has to be
/// against `receiver` alone. Both halves regressed together once.
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

    assert!(
        body.contains("texel_world * INTI_NORMAL_BIAS"),
        "the offset has to scale with the level's texel"
    );
    assert!(
        body.contains("to_light * INTI_DEPTH_BIAS"),
        "and carry the cascade's depth term too"
    );
    assert!(
        !body.contains("receiver + bias"),
        "a constant added to the compared depth is what detaches a shadow"
    );
}

/// Every pass that reads the page table reads it the SAME way.
///
/// 🔴 Written after breaking it. The table grew a second word per entry
/// and a third key state in one change, and the marking pass and the
/// shading pass were both updated while `page_compact.wgsl` was not. It
/// kept compiling, kept running, and rasterised `PAGE_DEAD - 1` — which
/// decodes into a perfectly well-formed view, light, level and cell,
/// none of which mean anything — into a slot read off the wrong word.
/// The frame filled with squares in the wrong places and nothing said
/// why.
///
/// A grep, because the alternative is running four passes against a
/// table hand-built into a hostile state. What it pins is exactly the
/// two things that drifted: the stride on the slot, and the dead key.
#[test]
fn every_table_reader_agrees_on_the_layout() {
    // (source, whether it is allowed to skip the dead check)
    let readers = [
        (
            "page_compact.wgsl",
            include_str!("../shaders/page_compact.wgsl"),
        ),
        ("page_mark.wgsl", include_str!("../shaders/page_mark.wgsl")),
    ];
    for (name, source) in readers {
        assert!(
            !source.contains("table_slots[entry]") && !source.contains("table_slots[probe]"),
            "{name} indexes the table's slots without PAGE_CELL"
        );
        assert!(
            source.contains("PAGE_DEAD"),
            "{name} reads the table without knowing an entry can be evicted"
        );
    }

    // The shading pass is the third reader and it lives in the other
    // crate. It walks PAST a tombstone rather than skipping it — a
    // lookup stops at EMPTY — so it needs the stride and not the
    // constant.
    let shading = kooch_lighting::inti_pbr_shader(1);
    assert!(
        shading.contains("inti_page_slots[probe * PAGE_CELL]"),
        "the shading pass indexes the table's slots without PAGE_CELL"
    );
    assert!(
        !shading.contains("if key == PAGE_DEAD"),
        "a lookup that skips a tombstone stops walking a run it has to finish"
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
    const BASE: f32 = 64.0;
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

    let centre = sun_centre(eyes[id.x].xyz, basis, base, side, level);
    let uv = clamp(
        (sun_plane(world, basis) - centre) / extent + vec2<f32>(0.5),
        vec2<f32>(0.0),
        vec2<f32>(0.99999),
    );
    let cell = floor(uv * f32(side));
    // 🔴 A FIXED cell's rect, not the rect of whichever cell the point
    // fell in. The second moves whenever the point changes cell, which
    // it is supposed to do; the first is the GRID, and the grid moving
    // by a fraction of a page is the crawl.
    let rect = sun_page_rect(level, vec2<u32>(0u, 0u), base, side, centre);
    cells[id.x] = vec4<f32>(cell, rect.xy);
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

    // Five camera positions spread across a fraction of one page.
    let eyes: Vec<[f32; 4]> = (0..9)
        .map(|i| {
            // Across several pages, in fractional steps: the fractional
            // part is what a grid that slides would carry into the rect.
            let t = i as f32 * page * 0.37;
            [t, 3.0, -t * 0.6, 0.0]
        })
        .collect();
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

    // 🔴 The property is NOT that the cell index holds — the camera
    // moves in world space and a page is measured in the sun's plane, so
    // it crosses page boundaries and the index is supposed to change.
    // What may never happen is the grid moving by a FRACTION of a page:
    // that is what re-quantises a shadow edge and makes it crawl. A jump
    // of exactly one page moves the index and leaves every texel's
    // footprint where it was.
    let first = read[0];
    let mut moved = false;
    for (i, got) in read.iter().enumerate().skip(1) {
        for axis in 0..2 {
            let slid = got[2 + axis] - first[2 + axis];
            let pages = slid / page;
            assert!(
                (pages - pages.round()).abs() < 1e-3,
                "camera {i} slid the grid {slid} metres on axis {axis}, \
                 which is {pages} pages of {page}"
            );
            if pages.round() != 0.0 {
                moved = true;
            }
        }
    }
    // And the run has to actually cross a boundary, or it proves that a
    // grid nobody moved did not move.
    assert!(
        moved,
        "the cameras never crossed a page; the test proves nothing"
    );
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
fn the_marking_is_recorded_before_the_shading() {
    let source = include_str!("../src/meshlet/render_stage/frame/render_r64.rs");

    let mark = source
        .find("self.record_page_marking(")
        .expect("the frame marks pages");
    let bind = source
        .find("self.bind_page_shadows(")
        .expect("the frame binds them");
    let shade = source.find(".render(").expect("the frame has a fused pass");

    assert!(
        mark < bind,
        "the shading is pointed at the pages before they are marked"
    );
    assert!(
        bind < shade,
        "the fused pass runs before it is pointed at this camera's pages"
    );

    // And the paint is the half that stays behind, because it writes the
    // colour buffer the fused pass is about to overwrite.
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

    // 🔴 Not "equal". A clipmap level is a power of two, so the level
    // it lands on is at best exact and at worst one step coarse — a
    // factor of two in the worst case, by construction, and no default
    // can close that. What is asserted is what a discrete chain can
    // promise: never much coarser anywhere, and finer at most distances.
    let mut finer = 0;
    let distances = [5.0_f32, 10.0, 20.0, 40.0, 80.0];
    let density = kooch_render::settings::RenderSettings::default().shadow_density;
    for distance in distances {
        let want = cascade(distance);
        let hundred = paged(distance, 100);
        let default = paged(distance, density);

        // The finding that moved the default: at 100 % the pages are the
        // coarser of the two at EVERY distance measured.
        assert!(
            hundred > want,
            "at {distance} m the 100 % setting already matches the cascade \
             ({hundred:.3} against {want:.3}); the default no longer needs raising"
        );
        // 1.3 is the measured worst case, at 10 m — the far edge of the
        // first cascade, which is where the cascade is most generous.
        assert!(
            default <= want * 1.3,
            "at {distance} m the default resolves {default:.3} m per texel \
             against the cascade's {want:.3}"
        );
        if default <= want {
            finer += 1;
        }
    }
    assert!(
        finer * 2 > distances.len(),
        "the default is finer than the cascade at only {finer} of {} distances",
        distances.len()
    );
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
    let levels = ClipmapConfig::default().levels as usize;

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
    let _ = queue;
}
