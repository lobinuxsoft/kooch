//! Pages claiming slots: density, local lights, a full pool, the table, views apart.

use super::*;

#[test]
fn half_density_is_a_quarter_of_the_pages() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);

    let at = |density| {
        let eye = Vec3::ZERO;
        let view = glam::camera::rh::view::look_at_mat4(eye, Vec3::NEG_Z, Vec3::Y);
        let proj = projection();
        let mut lights = GpuLights::new(&device);
        let mut frame = kooch_lighting::LightFrame::extract(&resources);
        lights.update(
            &device,
            &queue,
            &resources,
            ClusterCamera::new(eye, view, proj, VIEWPORT),
            None,
            &mut frame,
        );
        let depth_view = depth_texture(&device, &queue, 0.01);
        let mut marker = PageMarker::new(&device, PageConfig::default(), ClipmapConfig::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        lights.record_clusters(&mut encoder);
        marker.record(
            &device,
            &queue,
            &mut encoder,
            &lights,
            &depth_view,
            (proj * view).inverse(),
            eye,
            None,
            (SIZE, SIZE),
            /* view */ 0,
            1,
            density,
            Paint {
                target: &paint_target(&device),
                on: false,
                size: (SIZE, SIZE),
            },
        );
        queue.submit([encoder.finish()]);
        marker.poll();
        wait(&device);
        marker.poll();
        marker.last().expect("counters came back").resident
    };

    // 🔴 The lever, and the reason it is the ONE that moves: a coarser texel is a level coarser in
    // BOTH axes, so halving the density quarters the pages.
    let full = at(100);
    let half = at(50);
    assert!(full > 0 && half > 0);
    let ratio = full as f32 / half as f32;
    assert!(
        (2.0..=6.0).contains(&ratio),
        "half density gave {half} against {full}, a ratio of {ratio}"
    );
}

#[test]
fn every_drawable_page_claims_a_slot() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    // 🔴 The sun ALONE, because it is the only thing the raster draws and therefore the only thing
    // that spends the pool. With a local light in the scene the two counts are meant to differ —
    // that is `a_local_light_marks_but_does_not_claim`.
    let resources = world();
    let counts = run(
        &device,
        &queue,
        &resources,
        0.01,
        Some(Vec3::new(0.3, -1.0, 0.2)),
    );
    assert!(counts.resident > 0, "the frame needs pages");
    // Two mechanisms counting the same 0->1 transitions: the mark bit's atomicOr and the
    // allocator's atomicAdd. They agree or one of them is broken.
    assert_eq!(
        counts.pool.claims, counts.resident,
        "one claim per distinct page"
    );
    assert_eq!(counts.pool.overflow, 0, "the pool held them");
}

/// A local light claims its pages, now that something draws them.
#[test]
fn a_local_light_claims_its_pages() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 40.0);

    let dark = run(&device, &queue, &resources, 0.02, None);
    assert!(dark.resident > 0, "the point light marked nothing");
    assert_eq!(
        dark.pool.claims, dark.resident,
        "one claim per distinct page: {} claims of {} resident",
        dark.pool.claims, dark.resident
    );
    assert_eq!(
        dark.pool.unspent(dark.resident),
        0,
        "a marked local page is no longer a page nothing spends"
    );

    // And with a sun as well: both chains draw from one pool, which is
    // the budget line this whole track is measured against.
    let sunny = run(
        &device,
        &queue,
        &resources,
        0.02,
        Some(Vec3::new(0.3, -1.0, 0.2)),
    );
    assert!(sunny.pool.claims > 0, "nothing claimed with a sun in frame");
    assert!(
        sunny.resident > dark.resident,
        "adding a sun did not add pages: {} against {}",
        sunny.resident,
        dark.resident
    );
    assert_eq!(sunny.pool.overflow, 0, "a default pool overflowed");
}

#[test]
fn a_full_pool_denies_by_rank() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    // The SUN fills it: it is the only thing that spends the pool. A
    // NEAR surface, because a far one is one coarse clipmap level and a
    // handful of pages — not enough to overflow even a four-page pool.
    let resources = world();
    let small = PoolConfig {
        pages: 4,
        views: 1,
        row_cap: u32::MAX,
    };
    let (_marker, counts) = run_pool(
        &device,
        &queue,
        &resources,
        0.6,
        Some(Vec3::new(0.3, -1.0, 0.2)),
        // Four times the screen's density, so the clipmap picks a level fine enough for the frustum
        // to cover more than a handful of pages. Containment is a floor on the level and a far
        // surface pins it coarse whatever the density says.
        400,
        small,
    );
    assert!(
        counts.resident > small.slice(),
        "the frame asks for more than the pool holds: {} of {}",
        counts.resident,
        small.slice()
    );
    assert_eq!(counts.pool.allocated(), small.slice(), "the pool filled");
    // 🔴 With the seating plan (#942) a request the slice cannot fund is DENIED at its rank, not
    // dropped at the allocator: the plan funds exactly `slice` seats, so the free list never runs
    // dry and `overflow` — the allocator's own miss — stays zero.
    assert_eq!(
        counts.pool.claims + counts.pool.reused,
        small.slice(),
        "the pool handed out every slot it had"
    );
    assert!(
        counts.pool.denied > 0,
        "a pool of {} could not answer {} requests and said nothing",
        small.slice(),
        counts.resident
    );
    assert!(
        counts.pool.cutoff < 32,
        "denials without a cutoff: the plan did not run"
    );
    assert_eq!(
        counts.pool.overflow, 0,
        "the allocator missed — the plan's arithmetic does not close"
    );
}

#[test]
fn the_table_holds_every_claim() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);
    // 🔴 A sun, and the point light alongside it. Only the sun's pages claim a slot, so a scene
    // without one fills no table and this test would pass by having nothing to check.
    let (marker, counts) = run_pool(
        &device,
        &queue,
        &resources,
        0.01,
        Some(Vec3::new(0.3, -1.0, 0.2)),
        100,
        PoolConfig::default(),
    );
    assert!(counts.pool.claims > 0, "the sun claimed nothing");
    let slots = read_words(&device, &queue, marker.pool().slots());
    let capacity = marker.pool().config().total();

    let mut resident = 0u32;
    let mut seen_slots = std::collections::HashSet::new();
    for entry in 0..slots.len() / PAGE_CELL as usize {
        // The first word is `slot + 1`; `PAGE_ABSENT` (0) names nothing.
        let stored = slots[entry * PAGE_CELL as usize];
        if stored == 0 {
            continue;
        }
        resident += 1;
        let slot = stored - 1;
        assert!(slot < capacity, "slot {slot} past the pool");
        assert!(seen_slots.insert(slot), "slot {slot} handed out twice");
    }
    assert_eq!(
        resident,
        counts.pool.allocated(),
        "the table holds exactly what was allocated"
    );
}

/// Two cameras, one table.
#[test]
fn a_view_clears_only_its_own_pages() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 40.0);

    let eye = Vec3::ZERO;
    let view = glam::camera::rh::view::look_at_mat4(eye, Vec3::NEG_Z, Vec3::Y);
    let proj = projection();
    let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);
    let mut lights = GpuLights::new(&device);
    let mut frame = kooch_lighting::LightFrame::extract(&resources);
    lights.update(&device, &queue, &resources, camera, None, &mut frame);

    let depth_view = depth_texture(&device, &queue, 0.02);
    let config = PageConfig::default();
    let clipmap = ClipmapConfig::default();
    let mut marker = PageMarker::new(&device, config, clipmap);
    marker.set_pool(
        &device,
        PoolConfig {
            pages: 512,
            views: 2,
            row_cap: u32::MAX,
        },
    );

    for slice in 0..2u32 {
        let mut encoder = device.create_command_encoder(&Default::default());
        lights.record_clusters(&mut encoder);
        marker.record(
            &device,
            &queue,
            &mut encoder,
            &lights,
            &depth_view,
            (proj * view).inverse(),
            eye,
            Some(Vec3::new(0.3, -1.0, 0.2)),
            (SIZE, SIZE),
            slice,
            1,
            100,
            Paint {
                target: &paint_target(&device),
                on: false,
                size: (SIZE, SIZE),
            },
        );
        queue.submit([encoder.finish()]);
        wait(&device);
    }

    // The table is flat and a view's entries are a contiguous run, so
    // ownership is the entry's position against the span — the same
    // arithmetic `view_base` uses, rebuilt here so the two can disagree.
    let lights_count = lights.light_count().max(1);
    let padded = lights_count.max(1).next_multiple_of(64);
    let stride = (config.local_face_pages() * 6).div_ceil(32) * 32;
    let span = (padded as u64 * stride as u64
        + clipmap.levels as u64 * (config.side(0) as u64).pow(2))
    .div_ceil(32)
        * 32;

    let slots = read_words(&device, &queue, marker.pool().slots());
    let mut per_view = [0u32; 2];
    for entry in 0..slots.len() / PAGE_CELL as usize {
        if slots[entry * PAGE_CELL as usize] == 0 {
            continue;
        }
        let owner = (entry as u64 / span) as usize;
        assert!(owner < 2, "an entry belongs to camera {owner}");
        per_view[owner] += 1;
    }
    assert!(
        per_view[0] > 0,
        "camera 0's pages were wiped by camera 1: {per_view:?}"
    );
    assert!(per_view[1] > 0, "camera 1 marked nothing: {per_view:?}");
}
