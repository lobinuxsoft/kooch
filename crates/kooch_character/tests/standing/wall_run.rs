//! Running along a wall: arriving, running out, jumping off it, keeping it, banking.

use super::*;

/// Thrown along the wall at `speed`, and left to run.
fn running(resources: &mut Resources, run: WallRun, speed: f32) -> Entity {
    thrown(resources, run, speed, Vec3::new(0.15, 0.0, 1.0))
}

/// The same, steering where you say — including nowhere, for a test about the speed a character
/// *arrives* with rather than the speed air steering works it up to on the way in.
fn thrown(resources: &mut Resources, run: WallRun, speed: f32, steer: Vec3) -> Entity {
    let hero = against_a_wall(resources);
    insert(resources, hero, run);
    insert(resources, hero, Facing { direction: steer });
    simulate(resources, 1);
    let body = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<kooch_physics::plugin::SolverBody>())
        .and_then(|s| s.get(hero))
        .copied()
        .expect("no body");
    if let Some(world) = resources.get_mut::<PhysicsWorld>() {
        world.set_linear_velocity(body, Vec3::new(2.0, 0.0, speed));
    }
    // The side probes see the wall from the spawn point, so give the body speed first — a player
    // arrives already moving.
    if let Some(runs) = resources.get_mut::<kooch_character::plugin::run::Runs>() {
        runs.landed(hero);
    }
    hero
}

/// Acceptance: it carries speed along the wall instead of down it.
#[test]
fn it_runs_along_a_wall() {
    let mut resources = world();
    let hero = running(&mut resources, WallRun::default(), 9.0);
    let start = position(&resources, hero);
    simulate(&mut resources, 60);
    let after = position(&resources, hero);

    assert!(
        after.z - start.z > 5.0,
        "should have carried along the wall: {} to {}",
        start.z,
        after.z,
    );
    // Against free fall rather than a number: one second unheld is 4.9 m.
    assert!(
        start.y - after.y < 2.5,
        "and held most of the drop off: fell {} of a possible 4.9",
        start.y - after.y,
    );
}

/// Whether a character thrown at the wall at `speed` ends up running.
fn ran(speed: f32) -> bool {
    let mut resources = world();
    // Unsteered, so the speed it arrives with is the speed it was given.
    let hero = thrown(&mut resources, WallRun::default(), speed, Vec3::ZERO);
    simulate(&mut resources, 30);
    resources
        .get::<kooch_character::plugin::run::Runs>()
        .and_then(|runs| runs.of(hero))
        .is_some()
}

/// Arriving slowly is not a run. Asks the run's own clock: residual wall friction slows a fall
/// about as much as a run does.
#[test]
fn a_slow_arrival_does_not_run() {
    assert!(ran(9.0), "should have started a run at speed");
    assert!(!ran(0.5), "and not at a crawl");
}

/// The clock is what ends it, and the sag before that is what tells the player it is going to.
#[test]
fn the_run_runs_out() {
    let mut resources = world();
    let run = WallRun {
        duration: 0.5,
        ..Default::default()
    };
    let hero = running(&mut resources, run, 9.0);
    let held = position(&resources, hero).y;
    simulate(&mut resources, 30);
    let during = held - position(&resources, hero).y;
    simulate(&mut resources, 60);
    let after = held - position(&resources, hero).y;

    assert!(during < 1.0, "should still be up during the run: {during}");
    assert!(after > 4.0, "and falling once it is over: {after}");
}

/// Acceptance: a wall jump off a run keeps the run's speed. Throwing it
/// away is what makes a wall jump feel like a wall *stop*.
#[test]
fn a_wall_jump_keeps_the_run() {
    let mut resources = world();
    let run = WallRun::default();
    let hero = running(&mut resources, run, 9.0);
    insert(&mut resources, hero, WallJump::default());
    insert(
        &mut resources,
        hero,
        Jump {
            air_jumps: 0,
            ..Default::default()
        },
    );
    simulate(&mut resources, 20);

    let before = position(&resources, hero);
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<Jump>()
        && let Some(jump) = storage.get_mut(hero)
    {
        jump.wanted = true;
    }
    simulate(&mut resources, 30);
    let after = position(&resources, hero);

    assert!(
        after.x < before.x - 0.8,
        "should have been pushed off: {} to {}",
        before.x,
        after.x,
    );
    assert!(
        after.z - before.z > 2.0,
        "and kept going along it: {} to {}",
        before.z,
        after.z,
    );
}

#[test]
#[ignore = "measurement, not an assertion"]
fn run_trace() {
    let mut resources = world();
    let hero = running(&mut resources, WallRun::default(), 9.0);
    for step in 0..8 {
        simulate(&mut resources, 10);
        let found = resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<Touching>())
            .and_then(|s| s.get(hero))
            .copied()
            .unwrap_or_default();
        println!(
            "{:>3}  pos {:>7.3?}  wall {} {:.2}",
            step * 10,
            position(&resources, hero),
            found.wall,
            found.distance,
        );
    }
}

#[test]
#[ignore = "measurement, not an assertion"]
fn entry_trace() {
    for speed in [0.5f32, 9.0] {
        let mut resources = world();
        let hero = thrown(&mut resources, WallRun::default(), speed, Vec3::ZERO);
        for step in 0..6 {
            simulate(&mut resources, 5);
            let found = resources
                .get::<ComponentRegistry>()
                .and_then(|r| r.get_cpu::<Touching>())
                .and_then(|s| s.get(hero))
                .copied()
                .unwrap_or_default();
            println!(
                "entry {speed:>4}  {:>3}  pos {:>7.3?}  wall {}  run {:?}",
                step * 5,
                position(&resources, hero),
                found.wall,
                resources
                    .get::<kooch_character::plugin::run::Runs>()
                    .and_then(|runs| runs.state(hero)),
            );
        }
    }
}

/// Acceptance: it keeps hold of the wall it is running along, which a forward-only probe lost every
/// few frames.
#[test]
fn a_run_keeps_the_wall() {
    let mut resources = world();
    // Steered purely along the wall, which is what a player holds during a wall run — and the case
    // where a probe aimed where the character is going stops pointing at the wall at all.
    let hero = thrown(&mut resources, WallRun::default(), 9.0, Vec3::Z);
    simulate(&mut resources, 10);

    let mut lost = 0;
    for _ in 0..60 {
        simulate(&mut resources, 1);
        let found = resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<Touching>())
            .and_then(|s| s.get(hero))
            .copied()
            .unwrap_or_default();
        if !found.wall {
            lost += 1;
        }
    }
    assert!(lost < 5, "lost the wall on {lost} frames out of 60");
}

/// Acceptance: it banks towards the wall while running one. Upright, a
/// character running along a wall reads as one hovering beside it.
#[test]
fn a_run_banks_the_body() {
    let mut resources = world();
    let hero = running(&mut resources, WallRun::default(), 9.0);
    simulate(&mut resources, 30);

    let standing = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Transform>())
        .and_then(|s| s.get(hero))
        .map(|t| t.rotation * Vec3::Y)
        .expect("no transform");
    assert!(
        standing.x < -0.2,
        "should have tipped towards the wall at +X: {standing}",
    );
}

/// Measurement: a jump's time to apex, apex, fall and total — floaty is about time, not height. Run
/// with `--ignored --nocapture`.
#[test]
#[ignore = "measurement"]
fn jump_profile() {
    let mut resources = world();
    let hero = on_the_floor(&mut resources);
    let resting = position(&resources, hero).y;
    // The harness never advances `Time`, so the step is the physics fallback the systems actually
    // used — `FALLBACK_DT` in `kooch_physics::plugin::systems`.
    let dt = 1.0 / 60.0f32;

    let body = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<kooch_physics::plugin::SolverBody>())
        .and_then(|s| s.get(hero))
        .copied()
        .expect("no body");
    let speed = 5.0f32;
    if let Some(world) = resources.get_mut::<PhysicsWorld>() {
        let mass = world.mass(body).unwrap_or(1.0);
        world.apply_impulse(body, Vec3::Y * speed * mass);
    }

    let mut highest = resting;
    let mut apex_at = 0u32;
    let mut landed_at = None;
    for frame in 1..=240u32 {
        simulate(&mut resources, 1);
        let y = position(&resources, hero).y;
        if y > highest {
            highest = y;
            apex_at = frame;
        }
        // Back within a centimetre of where it started, on the way down.
        if landed_at.is_none() && frame > apex_at && (y - resting).abs() < 0.01 {
            landed_at = Some(frame);
        }
    }

    let rise = apex_at as f32 * dt;
    let land = landed_at.unwrap_or(240) as f32 * dt;
    println!("launch      {speed:.2} m/s");
    println!("apex        {:.3} m", highest - resting);
    println!("rise        {rise:.3} s");
    println!("fall        {:.3} s", land - rise);
    println!("airborne    {land:.3} s");
    println!();
    // 🔴 A parabola is symmetric: a fall slower than the rise means something is holding the
    // character up.
    let free = (2.0 * (highest - resting) / 9.81).sqrt();
    println!("free fall from that apex would be {free:.3} s");
    println!(
        "the descent takes {:.3} s more than gravity asks for",
        (land - rise) - free
    );
    println!();
    println!("For reference: a platformer jump is usually 0.4-0.7 s in total.");
}
