//! Walls: reporting, tipping, sliding down, gripping, jumping off, and sprinting past.

use super::*;

/// Acceptance: the wall a character is pressed against has one shared answer.
#[test]
fn a_wall_is_reported() {
    let mut resources = world();
    let hero = on_the_floor(&mut resources);
    insert(&mut resources, hero, Walk::default());
    insert(&mut resources, hero, Touching::default());
    // Its face at x = 1, well inside the character's reach.
    floor(
        &mut resources,
        Vec3::new(3.0, 2.0, 0.0),
        Vec3::new(2.0, 3.0, 6.0),
    );

    let facing = |resources: &Resources| {
        resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<Touching>())
            .and_then(|s| s.get(hero))
            .copied()
            .expect("no Touching")
    };

    insert(
        &mut resources,
        hero,
        Facing {
            direction: Vec3::NEG_X,
        },
    );
    simulate(&mut resources, 60);
    assert!(!facing(&resources).wall, "nothing behind it");

    insert(&mut resources, hero, Facing { direction: Vec3::X });
    simulate(&mut resources, 120);
    let found = facing(&resources);
    assert!(found.wall, "should have found the wall");
    assert!(
        found.normal.x < -0.9,
        "and it faces back at the character: {}",
        found.normal,
    );
}

#[test]
#[ignore = "measurement, not an assertion"]
fn slide_profile() {
    for degrees in [30f32, 45.0, 49.0, 51.0, 60.0, 65.0, 75.0] {
        let mut resources = world();
        Playing::set(&mut resources, true);
        source_at(
            &mut resources,
            Transform::from_position(Vec3::ZERO),
            GlobalGravity::default(),
        );
        slab(
            &mut resources,
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(20.0, 0.5, 12.0),
            glam::Quat::from_rotation_z(degrees.to_radians()),
        );
        let hero = character(&mut resources, Vec3::new(0.0, 8.0, 0.0));
        insert(&mut resources, hero, Walk::default());
        simulate(&mut resources, 90);
        let settled = position(&resources, hero);
        simulate(&mut resources, 120);
        let after = position(&resources, hero);
        let fell = settled.y - after.y;
        let along = (after - settled).length();
        println!(
            "{degrees:>4.0} deg  standing {:<5}  fell {fell:>7.3}  moved {along:>7.3}  ({:.2} m/s)",
            grounded(&resources, hero).standing,
            along / 2.0,
        );
    }
}

/// Acceptance: shoving a wall does not tip the character over — a lean from applied force held it
/// at 29°.
#[test]
fn a_wall_does_not_tip_it() {
    let mut resources = world();
    let hero = on_the_floor(&mut resources);
    insert(&mut resources, hero, Walk::default());
    floor(
        &mut resources,
        Vec3::new(3.0, 2.0, 0.0),
        Vec3::new(2.0, 3.0, 6.0),
    );

    insert(&mut resources, hero, Facing { direction: Vec3::X });
    simulate(&mut resources, 240);

    let standing = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Transform>())
        .and_then(|s| s.get(hero))
        .map(|t| t.rotation * Vec3::Y)
        .expect("no transform");
    assert!(
        standing.y > 0.99,
        "should be upright against the wall: {standing}",
    );
}

fn falling(resources: &Resources, hero: Entity) -> f32 {
    let body = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<kooch_physics::plugin::SolverBody>())
        .and_then(|s| s.get(hero))
        .copied()
        .expect("no body");
    resources
        .get::<PhysicsWorld>()
        .and_then(|world| world.linear_velocity(body))
        .map(|velocity| velocity.y)
        .unwrap_or(0.0)
}

/// Acceptance: a wall is somewhere to stop, not somewhere to fall past.
#[test]
fn a_wall_slows_the_fall() {
    let mut resources = world();
    let hero = against_a_wall(&mut resources);
    simulate(&mut resources, 120);
    let free = falling(&resources, hero);

    let mut clinging = world();
    let held = against_a_wall(&mut clinging);
    insert(&mut clinging, held, WallSlide::default());
    simulate(&mut clinging, 120);
    let slowed = falling(&clinging, held);

    assert!(free < -8.0, "should be falling freely: {free}");
    assert!(
        slowed > -2.5,
        "should be held to the slide speed: {slowed} against {free}",
    );
}

/// And only while it is being held on to. A character running past a
/// wall must not be slowed by brushing it.
#[test]
fn a_wall_beside_it_does_not_grip() {
    let mut resources = world();
    let hero = against_a_wall(&mut resources);
    insert(&mut resources, hero, WallSlide::default());
    // Steered along the wall rather than into it.
    insert(&mut resources, hero, Facing { direction: Vec3::Z });
    simulate(&mut resources, 120);
    assert!(
        falling(&resources, hero) < -5.0,
        "should have fallen past it: {}",
        falling(&resources, hero),
    );
}

/// Acceptance: it pushes off the wall, away and up.
#[test]
fn it_jumps_off_a_wall() {
    let mut resources = world();
    let hero = against_a_wall(&mut resources);
    insert(&mut resources, hero, WallSlide::default());
    insert(&mut resources, hero, WallJump::default());
    insert(
        &mut resources,
        hero,
        Jump {
            air_jumps: 0,
            ..Default::default()
        },
    );
    simulate(&mut resources, 60);

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
        after.x < before.x - 1.0,
        "should have been pushed away from the wall: {} to {}",
        before.x,
        after.x,
    );
}

/// Acceptance: the second jump, which is what `air_jumps` is for.
#[test]
fn it_jumps_again_in_the_air() {
    let mut resources = world();
    let hero = on_the_floor(&mut resources);
    insert(&mut resources, hero, Walk::default());
    insert(
        &mut resources,
        hero,
        Jump {
            air_jumps: 1,
            coyote: 0.0,
            ..Default::default()
        },
    );

    let press = |resources: &mut Resources| {
        if let Some(registry) = resources.get_mut::<ComponentRegistry>()
            && let Some(storage) = registry.get_cpu_mut::<Jump>()
            && let Some(jump) = storage.get_mut(hero)
        {
            jump.wanted = true;
        }
    };

    let ground = position(&resources, hero).y;
    press(&mut resources);
    simulate(&mut resources, 40);
    let single = position(&resources, hero).y;

    press(&mut resources);
    let mut highest = single;
    for _ in 0..60 {
        simulate(&mut resources, 1);
        highest = highest.max(position(&resources, hero).y);
    }
    assert!(
        highest > single + 0.5,
        "the second jump should have gone higher: {ground} -> {single} -> {highest}",
    );
}

/// Acceptance: running is faster than walking, and it is the same
/// mechanism — the top speed the goal is built from.
#[test]
fn a_sprint_is_faster() {
    let mut resources = world();
    let hero = on_the_floor(&mut resources);
    insert(&mut resources, hero, Walk::default());
    let walking = walked(&mut resources, hero, Vec3::X, 240);

    insert(
        &mut resources,
        hero,
        Sprint {
            wanted: true,
            ..Default::default()
        },
    );
    let running = walked(&mut resources, hero, Vec3::X, 240);

    assert!(
        running > walking * 1.5,
        "running {running} should beat walking {walking}",
    );
}

#[test]
#[ignore = "measurement, not an assertion"]
fn wall_trace() {
    let mut resources = world();
    let hero = against_a_wall(&mut resources);
    if std::env::var("KOOCH_LOOSE").is_ok() {
        insert(
            &mut resources,
            hero,
            Facing {
                direction: Vec3::ZERO,
            },
        );
    }
    for step in 0..7 {
        simulate(&mut resources, 20);
        let found = resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<Touching>())
            .and_then(|s| s.get(hero))
            .copied()
            .unwrap_or_default();
        println!(
            "{:>3}  pos {:>8.3?}  vy {:>7.3}  wall {} {:.2}  standing {}",
            step * 20,
            position(&resources, hero),
            falling(&resources, hero),
            found.wall,
            found.distance,
            grounded(&resources, hero).standing,
        );
    }
}

/// Acceptance: it stays on the wall after arriving at speed, instead of bouncing off mid-slide.
#[test]
fn a_wall_holds_it() {
    let mut resources = world();
    let hero = against_a_wall(&mut resources);
    insert(&mut resources, hero, WallSlide::default());
    // One step so the solver has a body to throw.
    simulate(&mut resources, 1);
    // Thrown at the wall rather than settled against it.
    let body = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<kooch_physics::plugin::SolverBody>())
        .and_then(|s| s.get(hero))
        .copied()
        .expect("no body");
    if let Some(world) = resources.get_mut::<PhysicsWorld>() {
        world.set_linear_velocity(body, Vec3::X * 9.0);
    }

    simulate(&mut resources, 30);
    let arrived = position(&resources, hero).x;
    simulate(&mut resources, 120);
    let later = position(&resources, hero).x;

    assert!(
        arrived > 0.4,
        "should have reached the wall at x = 1: {arrived}",
    );
    assert!(
        later > arrived - 0.1,
        "should still be on it: {arrived} drifted to {later}",
    );
    let found = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Touching>())
        .and_then(|s| s.get(hero))
        .copied()
        .expect("no Touching");
    assert!(found.wall, "and still sees it");
}
