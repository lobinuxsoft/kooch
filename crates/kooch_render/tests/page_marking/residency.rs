//! Persistence (#866 A): the pool outlives the frame that filled it.

use super::*;

// ---------------------------------------------------------------------
// Persistence (#866 A). The pool outlives the frame that filled it.
// ---------------------------------------------------------------------

/// Marks the same view `frames` times through one marker, returning what each frame counted.
fn run_frames(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    resources: &Resources,
    frames: u32,
    max_age: u32,
    pool: PoolConfig,
    // Shadow texels per screen pixel, per frame. Varying it moves which clipmap levels are marked,
    // which is a standing camera's cheapest way to ask for DIFFERENT pages each frame — the case
    // where an unreused hole is left behind for good.
    density: &dyn Fn(u32) -> u32,
    // Where the camera stands on each frame. A clipmap is centred on it,
    // so moving it is what makes a frame ask for DIFFERENT pages than
    // the last one — the case a standing camera cannot produce.
    eye_of: &dyn Fn(u32) -> Vec3,
    // Where the sun points on each frame.
    sun_of: &dyn Fn(u32) -> Vec3,
) -> Vec<MarkCounts> {
    let proj = projection();
    let mut lights = GpuLights::new(device);
    let depth_view = depth_texture(device, queue, 0.01);
    let target = paint_target(device);
    let mut marker = PageMarker::new(device, PageConfig::default(), ClipmapConfig::default());
    marker.set_pool(device, pool);
    marker.set_max_age(max_age);

    let mut out = Vec::new();
    for index in 0..frames {
        let eye = eye_of(index);
        let view = glam::camera::rh::view::look_at_mat4(eye, eye + Vec3::NEG_Z, Vec3::Y);
        let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);
        let mut frame = kooch_lighting::LightFrame::extract(resources);
        lights.update(device, queue, resources, camera, None, &mut frame);

        marker.set_frame(index);
        let mut encoder = device.create_command_encoder(&Default::default());
        lights.record_clusters(&mut encoder);
        marker.record(
            device,
            queue,
            &mut encoder,
            &lights,
            &depth_view,
            (proj * view).inverse(),
            eye,
            Some(sun_of(index)),
            (SIZE, SIZE),
            /* view */ 0,
            /* rate */ 1,
            density(index),
            Paint {
                target: &target,
                on: false,
                size: (SIZE, SIZE),
            },
        );
        queue.submit([encoder.finish()]);
        marker.poll();
        wait(device);
        marker.poll();
        out.push(marker.last().expect("the counters came back"));
    }
    out
}

/// A page nothing stopped wanting is still there next frame, and cost nothing to have.
#[test]
fn a_page_survives_a_frame_that_wants_it() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let resources = world();
    let frames = run_frames(
        &device,
        &queue,
        &resources,
        3,
        8,
        PoolConfig::default(),
        &|_| 100,
        &|_| Vec3::ZERO,
        &|_| Vec3::new(0.3, -1.0, 0.2),
    );

    let first = frames[0];
    assert!(first.resident > 0, "the frame needs pages");
    assert_eq!(first.pool.reused, 0, "nothing was resident before frame 0");
    assert_eq!(
        first.pool.claims, first.resident,
        "frame 0 allocates them all"
    );

    for (index, counts) in frames.iter().enumerate().skip(1) {
        assert_eq!(
            counts.pool.claims, 0,
            "frame {index} allocated {} pages it already had",
            counts.pool.claims
        );
        assert_eq!(
            counts.pool.reused, counts.resident,
            "frame {index} reused {} of {} requests",
            counts.pool.reused, counts.resident
        );
    }
}

/// Every request is answered exactly once, whether by a reuse or by an allocation.
#[test]
fn a_request_is_answered_once() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let resources = world();
    // Age 0 evicts everything every frame, so every frame after the first walks a table made
    // entirely of tombstones. That is the hostile case on purpose.
    for max_age in [0u32, 1, 8] {
        let frames = run_frames(
            &device,
            &queue,
            &resources,
            4,
            max_age,
            PoolConfig::default(),
            &|_| 100,
            &|_| Vec3::ZERO,
            &|_| Vec3::new(0.3, -1.0, 0.2),
        );
        for (index, counts) in frames.iter().enumerate() {
            assert_eq!(
                counts.pool.claims + counts.pool.reused,
                counts.resident,
                "age {max_age} frame {index}: {} claims + {} reuses is not {} requests",
                counts.pool.claims,
                counts.pool.reused,
                counts.resident
            );
            assert_eq!(
                counts.pool.leaked, 0,
                "age {max_age} frame {index} double-freed a slot"
            );
        }
    }
}

/// A slot freed by eviction is handed out again.
#[test]
fn an_evicted_slot_comes_back() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let resources = world();
    // A pool sized so that ONE frame fits and six do not, which is what
    // makes recycling the only way through.
    let pool = PoolConfig {
        pages: 8,
        views: 1,
        row_cap: u32::MAX,
    };
    let frames = run_frames(
        &device,
        &queue,
        &resources,
        6,
        0,
        pool,
        &|_| 100,
        &|_| Vec3::ZERO,
        &|_| Vec3::new(0.3, -1.0, 0.2),
    );

    let first = frames[0];
    assert!(
        first.pool.claims * frames.len() as u32 > pool.slice(),
        "the run has to ask for more pages than the pool holds, or it proves nothing: \
         {} claims over {} frames against {} slots",
        first.pool.claims,
        frames.len(),
        pool.slice()
    );
    for (index, counts) in frames.iter().enumerate() {
        assert_eq!(
            counts.pool.overflow, 0,
            "frame {index} ran out of pool with {} evictions behind it",
            counts.pool.evicted
        );
    }
    assert!(
        frames[1..].iter().any(|c| c.pool.evicted > 0),
        "age 0 has to evict every frame"
    );
}

/// Ageing is measured in FRAMES, and `max_age` decides how many.
#[test]
fn max_age_decides_whether_a_page_is_kept() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let resources = world();

    let churn = run_frames(
        &device,
        &queue,
        &resources,
        3,
        0,
        PoolConfig::default(),
        &|_| 100,
        &|_| Vec3::ZERO,
        &|_| Vec3::new(0.3, -1.0, 0.2),
    );
    for (index, counts) in churn.iter().enumerate().skip(1) {
        assert!(
            counts.pool.evicted > 0,
            "age 0, frame {index}: nothing was evicted"
        );
        assert_eq!(
            counts.pool.alive, 0,
            "age 0, frame {index} kept pages alive"
        );
        assert_eq!(
            counts.pool.claims, counts.resident,
            "age 0, frame {index} should re-allocate everything"
        );
    }

    let kept = run_frames(
        &device,
        &queue,
        &resources,
        3,
        1,
        PoolConfig::default(),
        &|_| 100,
        &|_| Vec3::ZERO,
        &|_| Vec3::new(0.3, -1.0, 0.2),
    );
    for (index, counts) in kept.iter().enumerate().skip(1) {
        assert_eq!(
            counts.pool.evicted, 0,
            "age 1, frame {index} evicted a page the frame was still asking for"
        );
        assert!(
            counts.pool.alive > 0,
            "age 1, frame {index} kept nothing alive"
        );
    }
}

// `holes_do_not_accumulate` lived here and is retired with the hash it measured: the flat table has
// no probe runs, so an eviction cannot leave a hole for a lookup to walk. See `page_table.wgsl`.

/// A page that stays resident keeps the SAME physical slot.
#[test]
fn a_resident_page_keeps_its_slot() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let resources = world();
    let pool = PoolConfig::default();

    // Two frames, camera and sun still, reading the table after each.
    let mut placements: Vec<std::collections::HashMap<u32, u32>> = Vec::new();
    let eye = Vec3::ZERO;
    let view = glam::camera::rh::view::look_at_mat4(eye, Vec3::NEG_Z, Vec3::Y);
    let proj = projection();
    let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);
    let mut lights = GpuLights::new(&device);
    let mut frame = kooch_lighting::LightFrame::extract(&resources);
    lights.update(&device, &queue, &resources, camera, None, &mut frame);
    let depth_view = depth_texture(&device, &queue, 0.01);
    let target = paint_target(&device);
    let mut marker = PageMarker::new(&device, PageConfig::default(), ClipmapConfig::default());
    marker.set_pool(&device, pool);

    for index in 0..4u32 {
        marker.set_frame(index);
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
            0,
            1,
            100,
            Paint {
                target: &target,
                on: false,
                size: (SIZE, SIZE),
            },
        );
        queue.submit([encoder.finish()]);
        let cells = read_words(&device, &queue, marker.pool().slots());
        let mut placed = std::collections::HashMap::new();
        for entry in 0..cells.len() / PAGE_CELL as usize {
            let stored = cells[entry * PAGE_CELL as usize];
            if stored == 0 {
                continue;
            }
            // The entry index IS the page id; the word is `slot + 1`.
            placed.insert(entry as u32, stored - 1);
        }
        placements.push(placed);
    }

    let first = &placements[1];
    assert!(!first.is_empty(), "the frame filed no pages");
    for (index, later) in placements.iter().enumerate().skip(2) {
        for (page, slot) in first {
            if let Some(now) = later.get(page) {
                assert_eq!(
                    now, slot,
                    "page {page} moved from slot {slot} to {now} by frame {index}"
                );
            }
        }
    }
}

/// A camera that keeps moving does not run the pool dry.
#[test]
fn a_moving_camera_does_not_exhaust_the_pool() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let resources = world();
    let frames = run_frames(
        &device,
        &queue,
        &resources,
        90,
        DEFAULT_MAX_AGE,
        PoolConfig::default(),
        &|_| 100,
        // A metre and a half a second at 60 Hz, which is a walk.
        &|i| Vec3::new(i as f32 * 0.025, 0.0, i as f32 * -0.025),
        &|_| Vec3::new(0.3, -1.0, 0.2),
    );

    let peak = frames.iter().map(|c| c.pool.allocated()).max().unwrap_or(0);
    // A denial is starvation with a name on it — it counts the same.
    let spilled: u32 = frames.iter().map(|c| c.pool.overflow + c.pool.denied).sum();
    eprintln!(
        "peak {peak} of {} slots, {spilled} pages went unallocated",
        frames[0].pool.capacity
    );
    assert_eq!(
        spilled, 0,
        "{spilled} pages found no slot; the pool peaked at {peak} of {}",
        frames[0].pool.capacity
    );
}
