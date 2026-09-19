//! What the page transform does to a triangle: winding, slope, coverage, texel size, the table's layout.

use super::*;

/// Which way a light-facing triangle winds after the page transform.
const WINDING: &str = r#"
@group(0) @binding(0) var<storage, read_write> out: array<f32>;

@compute @workgroup_size(1, 1, 1)
fn cs_winding() {
    // The sun travels straight down, so a floor faces it.
    let dir = vec3<f32>(0.0, -1.0, 0.0);
    let basis = sun_basis(dir);

    // A triangle whose normal is +Y — towards the light — wound counter-clockwise seen from above,
    // which is what a front face is everywhere else in this engine.
    var tri = array<vec3<f32>, 3>(
        vec3<f32>(0.0, 0.0, 0.0),
        vec3<f32>(1.0, 0.0, 0.0),
        vec3<f32>(0.0, 0.0, -1.0),
    );

    // Eye at the origin: the snap is irrelevant to winding, which is what this measures.
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

    // Edge-on to the sun, where the ratio diverges and only the clamp answers.
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

/// A clipmap texel is not one size, so a bias in metres cannot serve both ends of the chain.
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

/// The page reader offsets its SAMPLE by the texel, the way the cascade does — it does not add a
/// constant to the depth it compares.
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

    // 🔴 Matched without the whitespace, because the expression grew a third factor and wrapped
    // across lines. A grep test that pins the FORMATTING fails on a change that never touched the
    // behaviour, which is how this one first fired.
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
#[test]
fn every_table_reader_agrees_on_the_layout() {
    let readers = [
        (
            "page_compact.wgsl",
            include_str!("../../shaders/page_compact.wgsl"),
        ),
        (
            "page_mark.wgsl",
            concat!(
                include_str!("../../shaders/page_mark.wgsl"),
                include_str!("../../shaders/page_mark_views.wgsl"),
                include_str!("../../shaders/page_mark_froxels.wgsl"),
            ),
        ),
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
