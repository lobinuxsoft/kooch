//! The page table's shader functions, run on the GPU: octaves, faces, spots, floors.

use super::*;

/// The octave a page asks for, run through the SHADER'S OWN arithmetic.
const OCTAVE: &str = r#"
@group(0) @binding(0) var<storage, read_write> out: array<u32>;

// 🔴 The ENGINE'S base, and it is not a power of two. An earlier version of this test used 64.0,
// where every division lands exactly on a power of two and `floor(log2(...))` cannot round down —
// so it passed while the sun's levels were falling into the bucket below.
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

    // 🔴 The anchor: the sun's level IS its bucket. Everything else rests on this, because it is
    // what lets a lamp's pages reach the survivor lists the sun's culls already produce.
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

    // And range moves it. A hundred-metre lamp covers ten times the world with the same texels, so
    // it asks for coarser geometry than a ten-metre one at the same chain level — which is the
    // reason the bucket cannot be read off the chain level alone.
    assert!(
        out[LEVELS + 8] > lamp[0],
        "a 100 m lamp asked for bucket {} at chain level 0, the same as a 10 m lamp's {}",
        out[LEVELS + 8],
        lamp[0]
    );
    assert!(out[LEVELS + 9] > lamp[4], "and the same at chain level 4");
    // Measured: a 10 m lamp's chain lands on buckets [0,0,0,1,2,3,4,5] and a 100 m lamp's on
    // [1,..,5,..] — inside the sun's range, where its culls already produce survivor lists. That is
    // the claim C rests on and it is checked rather than assumed.
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

/// A spot's page frame follows the SPOT's axis, through the shader's own `spot_local`, `cube_face`
/// and `cell_face` — not a Rust mirror.
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
            include_str!("../../../kooch_lighting/shaders/inti_pbr.wgsl"),
        ),
        (
            "inti_debug.wgsl",
            include_str!("../../../kooch_lighting/shaders/inti_debug.wgsl"),
        ),
        ("page_mark.wgsl", include_str!("../../shaders/page_mark.wgsl")),
    ] {
        assert!(
            source.contains("local_level_floor("),
            "{file} does not consult the lamp chain's floor"
        );
    }
}

/// `face_local` and `cube_face` agree, and a point behind a face comes back with a negative `w`
/// rather than being rejected.
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
            // In front of its own face, and the uv it reconstructs is the uv it was built from.
            if local.z <= 0.0 {
                wrong_sign = wrong_sign + 1.0;
            }
            let back = local.xy / max(local.z, 1e-6) * 0.5 + vec2<f32>(0.5);
            worst_uv = max(worst_uv, max(abs(back.x - uv.x), abs(back.y - uv.y)));

            // And the OPPOSITE face has to report it behind. Rejecting that per vertex is the
            // defect; reporting it as negative w is the fix.
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
