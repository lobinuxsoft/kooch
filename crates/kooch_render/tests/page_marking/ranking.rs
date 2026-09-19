//! Who keeps a slot when the pool is short: ranks, reseating, the bias, distant lights.

use super::*;

/// The CPU mirror of `entry_rank` in `page_mark.wgsl`, decode for
/// decode, so the test can name the rank of every survivor.
fn entry_rank(config: &PageConfig, clip_levels: u32, within: u32) -> u32 {
    let stride = (config.local_face_pages() * 6).div_ceil(32) * 32;
    // The tests run well under 64 lights, so the padded slot count is the first step: 64.
    let sun_base = 64 * stride;
    if within >= sun_base {
        let cell = config.side(0).pow(2);
        let level = ((within - sun_base) / cell).min(clip_levels - 1);
        return (clip_levels - 1 - level).min(31);
    }
    let face = (within % stride) % config.local_face_pages();
    let mut level = config.local_floor();
    let mut next = config.side(level).pow(2);
    while level + 1 < config.levels() && face >= next {
        level += 1;
        next += config.side(level).pow(2);
    }
    (clip_levels + (config.levels() - 1 - level)).min(31)
}

/// Under pressure, what survives is the top of the ranking — never a page the plan ranked below one
/// it turned away. The issue's own acceptance test: plant more requests than slots, read the table,
/// and check every resident against the cutoff the plan reported.
#[test]
fn the_survivors_are_the_top_ranks() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    // A lamp alongside the sun, so the demand spans both classes and
    // the local ranks are really in the contest they are meant to lose.
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);
    let small = PoolConfig {
        pages: 4,
        views: 1,
        row_cap: u32::MAX,
    };
    let (marker, counts) = run_pool(
        &device,
        &queue,
        &resources,
        0.6,
        Some(Vec3::new(0.3, -1.0, 0.2)),
        400,
        small,
    );
    assert!(counts.pool.denied > 0, "no pressure, nothing to rank");
    let cutoff = counts.pool.cutoff;
    assert!(cutoff < 32, "denials without a cutoff");

    let config = PageConfig::default();
    let clip_levels = ClipmapConfig::default().levels;
    let cells = read_words(&device, &queue, marker.pool().slots());
    let mut residents = 0;
    for entry in 0..cells.len() / PAGE_CELL as usize {
        if cells[entry * PAGE_CELL as usize] == 0 {
            continue;
        }
        residents += 1;
        let rank = entry_rank(&config, clip_levels, entry as u32);
        assert!(
            rank <= cutoff,
            "entry {entry} of rank {rank} kept its seat past the cutoff {cutoff}"
        );
    }
    assert_eq!(residents, small.slice(), "the slice seated exactly itself");
}

/// A saturated pool reseats the frame the camera moves: the new view's pages take their seats from
/// the stale ones IN THE SAME FRAME, not after `max_age` lets them go. The starvation this replaces
/// sat at `0 new` forever while 6 652 requests waited.
#[test]
fn a_saturated_pool_reseats_on_move() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let resources = world();
    let small = PoolConfig {
        pages: 4,
        views: 1,
        row_cap: u32::MAX,
    };
    let eye = Vec3::ZERO;
    let view = glam::camera::rh::view::look_at_mat4(eye, Vec3::NEG_Z, Vec3::Y);
    let proj = projection();
    let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);
    let mut lights = GpuLights::new(&device);
    let mut frame = kooch_lighting::LightFrame::extract(&resources);
    lights.update(&device, &queue, &resources, camera, None, &mut frame);
    let target = paint_target(&device);
    let mut marker = PageMarker::new(&device, PageConfig::default(), ClipmapConfig::default());
    marker.set_pool(&device, small);

    // Two frames at two depths: the surface moves, so the second frame
    // wants pages the first one never marked — against a full slice.
    let mut last = None;
    for (index, depth) in [0.6f32, 0.15].into_iter().enumerate() {
        marker.set_frame(index as u32);
        let depth_view = depth_texture(&device, &queue, depth);
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
            400,
            Paint {
                target: &target,
                on: false,
                size: (SIZE, SIZE),
            },
        );
        queue.submit([encoder.finish()]);
        marker.poll();
        wait(&device);
        marker.poll();
        last = marker.last();
    }
    let counts = last.expect("the counters came back");
    assert!(
        counts.pool.claims > 0,
        "the camera moved against a full pool and nothing reseated: {:?}",
        counts.pool
    );
    assert!(
        counts.pool.preempted > 0,
        "new pages were seated but no stale resident paid for them: {:?}",
        counts.pool
    );
}

/// The pressure bias settles the denials (#943): a pool too small for the frame converges, one
/// level per frame, to a marking that fits — and then HOLDS, because the step down needs slack the
/// settled state does not have.
#[test]
fn the_bias_settles_the_denials() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let resources = world();
    let small = PoolConfig {
        pages: 4,
        views: 1,
        row_cap: u32::MAX,
    };
    let eye = Vec3::ZERO;
    let view = glam::camera::rh::view::look_at_mat4(eye, Vec3::NEG_Z, Vec3::Y);
    let proj = projection();
    let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);
    let mut lights = GpuLights::new(&device);
    let mut frame = kooch_lighting::LightFrame::extract(&resources);
    lights.update(&device, &queue, &resources, camera, None, &mut frame);
    let depth_view = depth_texture(&device, &queue, 0.6);
    let target = paint_target(&device);
    let mut marker = PageMarker::new(&device, PageConfig::default(), ClipmapConfig::default());
    marker.set_pool(&device, small);

    // A near surface at four times the screen's density wants more sun pages than four slots hold;
    // the bias has up to six steps (four local, two sun) plus the readback lag to settle in.
    let mut series = Vec::new();
    for index in 0..12u32 {
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
            400,
            Paint {
                target: &target,
                on: false,
                size: (SIZE, SIZE),
            },
        );
        queue.submit([encoder.finish()]);
        marker.poll();
        wait(&device);
        marker.poll();
        if let Some(counts) = marker.last() {
            series.push(counts);
        }
    }
    let last = series.last().expect("counters came back");
    assert!(
        last.pool.bias_sun > 0,
        "the demand never fit and the sun never paid: {:?}",
        last.pool
    );
    assert_eq!(
        last.pool.denied, 0,
        "the bias settled at +{} local +{} sun and pages still starve",
        last.pool.bias_local, last.pool.bias_sun
    );
    // No oscillation: once settled, a constant demand is a constant
    // bias. The last three frames have to agree.
    let tail: Vec<_> = series
        .iter()
        .rev()
        .take(3)
        .map(|c| (c.pool.bias_local, c.pool.bias_sun))
        .collect();
    assert!(
        tail.windows(2).all(|w| w[0] == w[1]),
        "the bias oscillates at the end: {tail:?}"
    );

    // And it unwinds: drop the demand to almost nothing and the bias walks back to zero on its own
    // — quality is only ever borrowed. Two trial steps 16 frames of patience apart, plus the
    // readback ring's lag: 48 relaxed frames is the controller's own arithmetic.
    let mut relaxed = None;
    for index in 12..60u32 {
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
            25,
            Paint {
                target: &target,
                on: false,
                size: (SIZE, SIZE),
            },
        );
        queue.submit([encoder.finish()]);
        marker.poll();
        wait(&device);
        marker.poll();
        relaxed = marker.last();
    }
    let relaxed = relaxed.expect("counters came back");
    assert_eq!(
        (relaxed.pool.bias_local, relaxed.pool.bias_sun),
        (0, 0),
        "the demand shrank and the bias never gave the quality back: {:?}",
        relaxed.pool
    );
    assert_eq!(relaxed.pool.denied, 0, "relaxed and still denying");
}

/// A light too small on screen drops to ONE page per cube face instead of a chain (#1009), and gets
/// its chain back the moment the threshold would pass it — here by turning it off, which is the
/// same comparison a closer camera flips.
#[test]
fn a_tiny_light_is_distant() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    // Reach 2 m at 10 m: a dozen-odd pixels of projected radius on the
    // test viewport — under a 32 px gate, over a disabled one. The
    // surface sits AT the lamp's depth so its centre pixels are lit.
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 2.0);

    let run_gated = |pixels: u32| {
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
        marker.set_coverage(pixels);
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
            // No sun: every page below is the lamp's own.
            None,
            (SIZE, SIZE),
            0,
            1,
            // 🔴 The finest density the settings allow, so the chain case actually asks for a chain.
            400,
            Paint {
                target: &target,
                on: false,
                size: (SIZE, SIZE),
            },
        );
        queue.submit([encoder.finish()]);
        marker.poll();
        wait(&device);
        marker.poll();
        marker.last().expect("the counters came back")
    };

    let open = run_gated(0);
    assert!(open.resident > 0, "the undemoted lamp marked nothing");
    assert_eq!(open.distant, 0, "an off threshold demoted something");

    let demoted = run_gated(32);
    assert!(
        demoted.resident > 0,
        "a lamp under the threshold cast nothing: the cliff is back"
    );
    assert!(demoted.distant > 0, "nothing was counted as distant");
    assert!(
        demoted.resident <= 6,
        "a distant lamp claimed {} pages; a cube has six faces and each gets one",
        demoted.resident
    );
    assert!(
        demoted.resident < open.resident,
        "the tier cost as much as the chain: {} against {}",
        demoted.resident,
        open.resident
    );
    assert_eq!(
        demoted.pairs, open.pairs,
        "the threshold changed the grid walk instead of the marking"
    );
}
