//! The cluster path: the census, its counts, lamps against the sun, the bias and the halo.

use super::*;

/// The per-pixel path counts into workgroup memory, never into a global counter.
#[test]
fn the_hot_path_counts_in_workgroup_memory() {
    let source = include_str!("../../shaders/page_mark.wgsl");
    let (_, body) = source
        .split_once("fn mark_pixel(")
        .expect("page_mark.wgsl has no mark_pixel");
    // To the next top-level item, which is where the per-pixel path ends.
    let body = body.split("\n@").next().unwrap_or(body);
    assert!(
        !body.contains("&counters["),
        "mark_pixel touches a global counter; every pixel of the dispatch \
         would serialise on that one address"
    );
    assert!(
        body.contains("&tally["),
        "mark_pixel counts nothing into workgroup memory — the census is \
         either gone or back on the global counters"
    );
}

/// The occupancy census counts froxels, and there are fewer of them than there are samples.
#[test]
fn the_census_counts_froxels_not_samples() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);
    let counts = run(&device, &queue, &resources, 0.01, None);

    assert!(counts.samples > 0, "the harness drew no surface");
    assert!(counts.froxels > 0, "a full screen of surface occupied none");
    assert!(
        counts.froxels < counts.samples,
        "froxels {} against {} samples — the census is counting pixels",
        counts.froxels,
        counts.samples
    );
    // The bitmap is 4096 bits wide and the grid is capped to match.
    assert!(
        counts.froxels <= 4096,
        "{} froxels, past the bitmap",
        counts.froxels
    );
}

/// Sky occupies nothing.
#[test]
fn sky_occupies_no_froxel() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);
    let counts = run(&device, &queue, &resources, 0.0, None);
    assert_eq!(counts.samples, 0, "the harness drew a surface");
    assert_eq!(counts.froxels, 0, "sky occupied a froxel");
}

/// The cluster path marks the same scene for a fraction of the pairs.
#[test]
fn the_cluster_path_marks_for_fewer_pairs() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);

    let per_pixel = run(&device, &queue, &resources, 0.01, None);
    let per_froxel = with_clusters(|| run(&device, &queue, &resources, 0.01, None));

    assert!(per_pixel.pairs > 0, "the per-pixel path walked nothing");
    assert_eq!(per_froxel.overflow, 0, "a page index past the buffer");
    // 🔴 Olsson's EXPLICIT bounds, in one number. A froxel is mostly empty and its box is a slab;
    // marking the box asks for pages across depth that holds nothing.
    assert!(
        per_froxel.resident <= per_pixel.resident * 2,
        "cluster marked {} pages against {} — the over-marking is what \
         the pool pays, and it is unbounded",
        per_froxel.resident,
        per_pixel.resident
    );
    // 🔴 The safety property, and the only direction an approximation of "which pages does this
    // scene need" may err in.
    assert!(
        per_froxel.resident >= per_pixel.resident,
        "cluster marked {} pages against the per-pixel path's {} — it is \
         marking FEWER, which is a missing shadow",
        per_froxel.resident,
        per_pixel.resident
    );
    // And the whole point: an order of magnitude fewer walks.
    assert!(
        per_froxel.pairs * 10 < per_pixel.pairs,
        "cluster pairs {} against per-pixel {} — not the win the rewrite \
         is for",
        per_froxel.pairs,
        per_pixel.pairs
    );
}

/// The counts say which path produced them.
#[test]
fn the_counts_say_which_path_walked_them() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    add_point(&mut resources, Vec3::new(0.0, 0.0, -10.0), 20.0);

    let per_pixel = run(&device, &queue, &resources, 0.01, None);
    let per_froxel = with_clusters(|| run(&device, &queue, &resources, 0.01, None));

    assert!(!per_pixel.by_froxel, "the per-pixel path claimed froxels");
    assert!(per_froxel.by_froxel, "the froxel path claimed pixels");
    // And the ratio each side implies is the same order of magnitude,
    // which is what makes the panel's comparison honest.
    let by_pixel = per_pixel.pairs as f32 / per_pixel.samples.max(1) as f32;
    let by_froxel = per_froxel.pairs as f32 / per_froxel.froxels.max(1) as f32;
    assert!(
        by_froxel > by_pixel * 0.5,
        "lights per froxel {by_froxel} against lights per pixel {by_pixel} — \
         a froxel holds at least as many lights as a pixel inside it"
    );
}

/// Lamps that overrun the pool do not blur the sun.
#[test]
fn lamps_that_overrun_the_pool_spare_the_sun() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    // Enough lamps, close enough, that their pages cannot all be seated.
    for i in 0..12 {
        let x = (i as f32 - 6.0) * 0.7;
        add_point(&mut resources, Vec3::new(x, 0.0, -4.0), 12.0);
    }
    let eye = Vec3::ZERO;
    let view = glam::camera::rh::view::look_at_mat4(eye, Vec3::NEG_Z, Vec3::Y);
    let proj = projection();
    let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);
    let mut lights = GpuLights::new(&device);
    let mut frame = kooch_lighting::LightFrame::extract(&resources);
    lights.update(&device, &queue, &resources, camera, None, &mut frame);
    let depth_view = depth_texture(&device, &queue, 0.02);
    let target = paint_target(&device);
    let mut marker = PageMarker::new(&device, PageConfig::default(), ClipmapConfig::default());
    marker.set_pool(
        &device,
        PoolConfig {
            pages: 12,
            views: 1,
            row_cap: u32::MAX,
        },
    );

    let mut last = None;
    for index in 0..24u32 {
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
        marker.poll();
        wait(&device);
        marker.poll();
        if let Some(counts) = marker.last() {
            last = Some(counts);
        }
    }
    let last = last.expect("counters came back");
    // The premise: the cut has to land among the LAMPS, or this proves nothing about who pays.
    let sun_levels = ClipmapConfig::default().levels;
    assert!(
        last.pool.denied > 0,
        "nothing was denied — the pool absorbed the demand, and a scene \
         with no shortfall cannot say who should pay for one"
    );
    assert!(
        last.pool.cutoff >= sun_levels,
        "the plan cut at rank {} of {} sun ranks — the sun WAS denied, so \
         this scene cannot say who should pay",
        last.pool.cutoff,
        sun_levels
    );
    assert_eq!(
        last.pool.bias_sun, 0,
        "the sun was funded to its last rank and blurred anyway: \
         locals +{} sun +{}, cut at rank {}",
        last.pool.bias_local, last.pool.bias_sun, last.pool.cutoff
    );
}

/// The peak overlap is a peak, not the average, and both paths report it.
#[test]
fn the_census_reports_the_worst_froxel() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    // Stacked on one spot: every one of them reaches the same froxels.
    for i in 0..6 {
        let nudge = i as f32 * 0.01;
        add_point(&mut resources, Vec3::new(nudge, 0.0, -6.0), 40.0);
    }
    let counts = run(&device, &queue, &resources, 0.01, None);
    assert!(counts.froxels > 0, "no froxel was occupied");
    let average = counts.pairs as f32 / counts.samples.max(1) as f32;
    assert!(
        counts.peak_lights >= average.ceil() as u32,
        "peak {} is under the average {average} — it is not a peak",
        counts.peak_lights
    );
    assert!(
        counts.peak_lights >= 2,
        "six lights on one spot and the worst froxel saw {}",
        counts.peak_lights
    );

    // Same scene through the cluster path: overlap is a property of the
    // SCENE, so the alert has to read the same either way.
    let froxel = with_clusters(|| run(&device, &queue, &resources, 0.01, None));
    assert_eq!(
        froxel.peak_lights, counts.peak_lights,
        "the two marking paths disagree about how much the scene overlaps"
    );
}

/// The bias lands on its value in one step, not one step a frame.
#[test]
fn the_bias_reaches_its_value_in_one_step() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut resources = world();
    for i in 0..24 {
        let x = (i as f32 - 12.0) * 0.35;
        add_point(&mut resources, Vec3::new(x, 0.2, -2.0), 20.0);
    }
    let eye = Vec3::ZERO;
    let view = glam::camera::rh::view::look_at_mat4(eye, Vec3::NEG_Z, Vec3::Y);
    let proj = projection();
    let camera = ClusterCamera::new(eye, view, proj, VIEWPORT);
    let mut lights = GpuLights::new(&device);
    let mut frame = kooch_lighting::LightFrame::extract(&resources);
    lights.update(&device, &queue, &resources, camera, None, &mut frame);
    let depth_view = depth_texture(&device, &queue, 0.02);
    let target = paint_target(&device);

    let mut series_for = |cluster: bool| {
        let mut marker = PageMarker::new(&device, PageConfig::default(), ClipmapConfig::default());
        marker.set_cluster_marking(cluster);
        marker.set_pool(
            &device,
            PoolConfig {
                pages: 12,
                views: 1,
                row_cap: u32::MAX,
            },
        );

        let mut series = Vec::new();
        for index in 0..8u32 {
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
            marker.poll();
            wait(&device);
            marker.poll();
            if let Some(counts) = marker.last() {
                series.push(counts.pool.bias_local);
            }
        }
        series
    };

    // `corrections` is how many times the bias moved up AFTER its first move. Stepping one per
    // frame would need `settled - 1` of them; the point of computing the fit is that this number
    // stays tiny however deep the scene's answer is.
    let check = |series: Vec<u32>, corrections: usize, path: &str| {
        let settled = *series.last().expect("counters came back");
        assert!(
            settled > 1,
            "{path}: this scene does not need a multi-step bias, so it \
             cannot show one being reached in a step: settled at +{settled}"
        );
        // 🔴 The property, and it is deliberately not exactness. The raise uses the OPTIMISTIC
        // estimate — four pages become one per level — because raising too little costs a frame of
        // denials while raising too much costs blur the player sees.
        let first_move = series
            .iter()
            .copied()
            .find(|&b| b > 0)
            .expect("the bias never rose");
        assert!(
            first_move + corrections as u32 >= settled,
            "{path}: the first move was +{first_move} against a settled \
             +{settled}: {series:?} — that is stepping, not computing"
        );
        let rises = series.windows(2).filter(|w| w[1] > w[0]).count();
        assert!(
            rises <= corrections,
            "{path}: the bias rose {rises} times after its first move: \
             {series:?} — a correction is the estimate erring low, a climb \
             is stepping"
        );
        // Whatever it took to get there, it must be strictly better than
        // the one-step-a-frame rule this replaced.
        assert!(
            rises < settled as usize,
            "{path}: {rises} rises to reach +{settled} is the per-frame \
             step it was supposed to replace: {series:?}"
        );
        settled
    };

    let pixels = check(series_for(false), 1, "per pixel");
    // One further out, and the cap on a face's rect is why — see the
    // header. Same answer, one more frame to reach it.
    let clusters = check(series_for(true), 2, "per cluster");
    assert_eq!(
        pixels, clusters,
        "the two marking paths settled on different resolutions for one \
         scene, which is a difference in what they MARK, not in how fast \
         the bias converges"
    );
}

/// A dilated request asks for the neighbouring pages too.
#[test]
fn a_halo_asks_for_the_neighbours() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let resources = world();
    let sun = Some(Vec3::new(0.2, -1.0, 0.3));

    HALO.with(|h| h.set(0.0));
    let plain = run(&device, &queue, &resources, 0.5, sun);
    HALO.with(|h| h.set(0.5));
    let dilated = run(&device, &queue, &resources, 0.5, sun);
    HALO.with(|h| h.set(0.0));

    assert!(plain.resident > 0, "the rig marked nothing to dilate");
    assert!(
        dilated.resident > plain.resident,
        "the halo asked for nothing new: {} against {}",
        dilated.resident,
        plain.resident,
    );
    assert!(
        dilated.resident < plain.resident * 3,
        "the halo cost the full 3x — {} against {} — so no two neighbours ever \
         collapsed onto one page and the offset is far larger than a page",
        dilated.resident,
        plain.resident,
    );
}

/// The dilation direction has to vary per thread.
#[test]
fn the_dilation_picks_a_diagonal_per_thread() {
    let source = include_str!("../../shaders/page_mark.wgsl");
    let dense: String = source.chars().filter(|c| !c.is_whitespace()).collect();

    assert!(
        dense.contains("select(-1.0,1.0,(id.x&1u)!=0u)")
            && dense.contains("select(-1.0,1.0,(id.y&1u)!=0u)"),
        "the dilation offset is not dithered off the thread index, so every pixel dilates \
         the same way and three of the four diagonals are never requested",
    );
    assert!(
        !dense.contains("(basis[0]+basis[1])*(halo*width)"),
        "the fixed diagonal is back",
    );
}
