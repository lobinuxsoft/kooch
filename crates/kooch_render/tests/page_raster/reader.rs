//! Reading the pages back: relisting, the sun's disc, both expansions, misses and neighbours.

use super::*;

/// 🔴 `a_caster_behind_every_receiver_pairs_nothing` lived here and is gone with the bound it
/// tested.

/// A cleared page outlives the generation it was cleared under, and only a lamp's does.
#[test]
fn an_empty_lamp_page_stops_relisting() {
    let table = kooch_lighting::PAGE_TABLE;
    let compact = include_str!("../../shaders/page_compact.wgsl");
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
    // And the box reader is still reachable, because nothing has measured what the march costs.
    assert!(
        shading.contains("inti_pages.layer.z != 0u"),
        "the march has to stay selectable; it replaces the reader every shipped frame goes \
         through and its cost is unmeasured",
    );
}

/// The inverted expansion emits the SAME pairs as the paired one.
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
        // Each run gets its own pool and its own rasteriser: a page listed once is stamped, and the
        // second run would cache it and list nothing.
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
        // ⚠️ The per-instance cull, not the chunked one.
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
            // Every layer casts into the sun (#1220).
            u32::MAX,
            &lamps,
            &lights_buffer,
            &[],
            1.0,
            None,
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
        // The descent's own counter says which shape actually ran. A silent fallback to pairing
        // would make the comparison below compare the paired shape against itself.
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
        // The order is whatever the atomics handed out; the SET is the claim.
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

/// A resident page with no content has to read as a MISS, and the reader has to keep climbing when
/// it does.
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
    // And the lamps must NOT re-resolve: their pages are six faces of a chain, so a step off an
    // edge crosses a face and lands nowhere this arithmetic can index.
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
    let compact = include_str!("../../shaders/page_compact.wgsl");
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
