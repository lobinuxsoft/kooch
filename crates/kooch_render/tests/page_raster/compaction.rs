//! The page table compacting into lists: lamps without drops, cached sun pages, levels kept apart.

use super::*;

/// The sun's cache generation mirrors `sun_centre` exactly: a still camera caches, a lateral step
/// inside one page width still caches, and a step that crosses the snap grid redraws.
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

    // Off the snap grid's own lines: an eye at the origin sits exactly on a boundary, where the
    // tiniest step flips `floor` — a real invalidation, not the case under test.
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
    // 🔴 A step of a WHOLE page, which used to be the expensive case and is the point of the fix.
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

    // Camera 1's table, seen from camera 1: three sun pages on two levels, one local page this
    // raster does not draw, and two pages belonging to the OTHER camera. The table is flat — the
    // entry index IS the page id and the first word is `slot + 1`.
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
    // 🔴 The other camera's pages, on levels this one also uses. The dispatch covers only THIS
    // view's span, so they are outside it — and their listings have to come through untouched.
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
    // 🔴 And it LANDS in the lamp's OWN bucket — after the sun's levels, at `levels + slot` — where
    // its own cull's survivors are bound. It briefly shared the sun's octave buckets; that borrowed
    // survivor lists culled for the camera's orthographic boxes and broke lamp shadows both ways.
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

    // 🔴 And the way BACK. `page_list` is dense and per view, so a pass that computes a page KEY —
    // rather than reading one out of the list — has no route to the entry the draw indexes by.
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
