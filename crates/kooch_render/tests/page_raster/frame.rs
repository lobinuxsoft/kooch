//! The pages inside a frame: the clipmap grid, the pass order, the resolve and its counters.

use super::*;

/// The clipmap's texel grid does not slide with the camera.
#[test]
fn the_clipmap_grid_does_not_slide_with_the_camera() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    // 🔴 The ENGINE'S base, and it is not a power of two. An earlier version of this test used 64.0,
    // where every division lands exactly on a power of two and `floor(log2(...))` cannot round down
    // — so it passed while the sun's levels were falling into the bucket below.
    const BASE: f32 = 1.28;
    const SIDE: u32 = 128;
    const LEVEL: u32 = 3;
    // One page of level 3, which is what the camera has to stay inside for the grid to hold still.
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
    // 🔴 The KEY of a fixed world point, and the world rect that key stands for.
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

    // ⚠️ The shader's own constants, which are NOT the ones above: it declares `base = 64.0` where
    // the engine ships 1.28. One page of the level it reads is this wide, and the cameras have to
    // cross several of them or the test proves nothing.
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

    // 🔴 The property, now that a page is keyed by absolute world position: a fixed point's page
    // does not move AT ALL while the camera walks across several pages. Neither its key nor the
    // world rect that key stands for.
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
#[test]
fn the_marking_sits_between_raster_and_shading() {
    let source = include_str!("../../src/meshlet/render_stage/frame/render_r64.rs");

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

/// The paged shadow resolves at least as finely as the cascade it replaces.
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

    // 🔴 The gap is SIZED, not closed. A quality setting's maximum is its maximum, so the list stops
    // at 100 % — and at 100 % the pages are the coarser of the two at every distance measured.
    let distances = [5.0_f32, 10.0, 20.0, 40.0, 80.0];
    // 🔴 The REFERENCE density, taken from the choices list rather than from `Default`.
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
    // Measured. A clipmap level is a power of two, so where the chain steps decides this as much as
    // the density does — which is why the worst case is not at the far end.
    assert!(
        worst.1 <= 2.5,
        "the pages fell to {:.2}x the cascade at {} m",
        worst.1,
        worst.0,
    );

    // 🔴 The list REACHES past the default now, and the entries above it have to say what they cost.
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
#[test]
fn the_counters_carry_the_expansions_cost() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let raster = rasterizer(&device);
    // Every run is per BUCKET, so the offsets follow the buckets and not the clipmap's levels —
    // planting at the clipmap's stride lands the survivors inside the overflow flags.
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

    // And the third run: what the OTHER shape would have cost, and the choice between them.
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
#[test]
fn the_page_passes_are_profiled() {
    for (name, source, wanted) in [
        (
            "frame/pages/record.rs",
            include_str!("../../src/meshlet/render_stage/frame/pages/record.rs"),
            "shadow pages",
        ),
        (
            "pages/raster/record.rs",
            include_str!("../../src/shadow/pages/raster/record.rs"),
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
        (
            "pages/mark/record.rs",
            include_str!("../../src/shadow/pages/mark/record.rs"),
        ),
        (
            "pages/raster/record.rs",
            include_str!("../../src/shadow/pages/raster/record.rs"),
        ),
    ] {
        assert!(
            source.contains("#[profiling::function]"),
            "{name} records GPU work in a function the profiler cannot see"
        );
    }

    // 🔴 Every scope OPENED has to be closed, and nothing above checks that. A `nested()` without
    // its `close()` is not a missing timing: wgpu refuses the whole encoder — "a debug group was
    // not popped before the encoder was finished" — and the frame stops being submitted at all.
    {
        let source = include_str!("../../src/shadow/pages/raster/record.rs");
        let opened = source.matches("= nested(track,").count();
        let closed = source.matches("close(track,").count();
        assert_eq!(
            opened, closed,
            "pages/raster/record.rs opens {opened} GPU scopes and closes {closed}; an unpopped \
             debug group makes wgpu reject the encoder and the frame never reaches the queue"
        );
    }

    // 🔴 Everything above measures the CPU, and every line of it passed while this track spent 34 ms
    // per frame on the OneXFly that no capture could see.
    for (name, source, wanted) in [
        (
            "frame/pages/record.rs",
            include_str!("../../src/meshlet/render_stage/frame/pages/record.rs"),
            ["shadow pages", "page mark", "page raster"].as_slice(),
        ),
        (
            "pages/raster/record.rs",
            include_str!("../../src/shadow/pages/raster/record.rs"),
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
