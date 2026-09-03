//! #94's acceptance list, against a real solver.
//!
//! Unit tests can say the spring arithmetic is right. Only this can say
//! a character *stands* — the maths being correct and the impulse
//! reaching a body that then holds its height are different claims, and
//! the second is the one that has ever been wrong.

use glam::Vec3;

use kooch_character::plugin::hold_characters;
use kooch_character::{CharacterController, Facing, Grounded, Walk};
use kooch_core::resource::Resources;
use kooch_core::run_state::Playing;
use kooch_core::time::Time;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::entity::Entity;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::transform::Transform;
use kooch_gravity::{GlobalGravity, PointGravity, plugin};
use kooch_physics::components::{Collider, KIND_STATIC, PhysicsBody, SHAPE_CUBOID};
use kooch_physics::plugin::{
    PhysicsWorld, physics_step_system, physics_sync_system, physics_writeback_system,
};
use kooch_physics::rapier_backend::RapierBackend;

fn world() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r.insert(Commands::new());
    r.insert(DynamicComponents::new());
    r.insert(Time::new());
    r.insert(PhysicsWorld::new(Box::new(RapierBackend::new())));

    let registry = r.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<Transform>();
    registry.register_cpu_reflected::<PhysicsBody>();
    registry.register_cpu_reflected::<Collider>();
    registry.register_cpu::<kooch_physics::plugin::SolverBody>();
    registry.register_cpu_reflected::<GlobalGravity>();
    registry.register_cpu_reflected::<PointGravity>();
    registry.register_cpu_reflected::<CharacterController>();
    registry.register_cpu_reflected::<Facing>();
    registry.register_cpu_reflected::<Grounded>();
    registry.register_cpu_reflected::<Walk>();
    r
}

fn spawn(resources: &mut Resources) -> Entity {
    let mut commands = resources.remove::<Commands>().unwrap();
    let entity = commands.spawn(resources).id();
    commands.apply(resources);
    resources.insert(commands);
    entity
}

fn insert<T: Component>(resources: &mut Resources, entity: Entity, value: T) {
    use std::any::TypeId;
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<T>()
    {
        storage.insert(entity, value);
    }
    let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() else {
        return;
    };
    let current = match archetypes.entity_archetype(entity) {
        Some(current) => current,
        None => {
            let empty = archetypes.get_or_create(Default::default());
            archetypes.register_entity(entity, empty);
            empty
        }
    };
    let next = archetypes.archetype_after_add_dynamic(current, TypeId::of::<T>());
    archetypes.register_entity(entity, next);
}

fn body_at(resources: &mut Resources, position: Vec3) -> Entity {
    let entity = spawn(resources);
    insert(resources, entity, Transform::from_position(position));
    insert(resources, entity, PhysicsBody::default());
    insert(resources, entity, Collider::default());
    entity
}

fn source_at<T: Component>(resources: &mut Resources, transform: Transform, source: T) -> Entity {
    let entity = spawn(resources);
    insert(resources, entity, transform);
    insert(resources, entity, source);
    entity
}

fn position(resources: &Resources, entity: Entity) -> Vec3 {
    resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Transform>())
        .and_then(|s| s.get(entity))
        .map(|t| t.position)
        .expect("no transform")
}

/// The frame the gravity and character plugins schedule between them.
///
/// The character system runs *after* gravity and before the step, which
/// is the order the plugins register: the spring fights this step's
/// gravity, and fighting last step's is a character that sinks whenever
/// the field changes.
fn simulate(resources: &mut Resources, steps: u32) {
    for _ in 0..steps {
        plugin::reconcile_world_gravity_for_test(resources);
        physics_sync_system(resources);
        if Playing::is_playing(resources) {
            plugin::apply_gravity_sources(resources);
            hold_characters(resources);
            physics_step_system(resources);
            physics_writeback_system(resources);
        }
    }
}

/// A capsule that holds itself up, at `position`.
fn character(resources: &mut Resources, position: Vec3) -> Entity {
    let entity = spawn(resources);
    insert(resources, entity, Transform::from_position(position));
    insert(resources, entity, PhysicsBody::default());
    insert(
        resources,
        entity,
        Collider {
            shape: kooch_physics::components::SHAPE_CAPSULE,
            radius: 0.4,
            half_height: 0.5,
            ..Default::default()
        },
    );
    insert(resources, entity, CharacterController::default());
    insert(resources, entity, Grounded::default());
    entity
}

/// A slab to stand on, centred at `at`.
fn floor(resources: &mut Resources, at: Vec3, half: Vec3) -> Entity {
    slab(resources, at, half, glam::Quat::IDENTITY)
}

/// The same, turned — a wall is a floor stood on its edge.
fn slab(resources: &mut Resources, at: Vec3, half: Vec3, rotation: glam::Quat) -> Entity {
    let entity = spawn(resources);
    insert(
        resources,
        entity,
        Transform {
            position: at,
            rotation,
            scale: Vec3::ONE,
        },
    );
    insert(
        resources,
        entity,
        PhysicsBody {
            kind: KIND_STATIC,
            ..Default::default()
        },
    );
    insert(
        resources,
        entity,
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: half,
            ..Default::default()
        },
    );
    entity
}

fn grounded(resources: &Resources, entity: Entity) -> Grounded {
    resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Grounded>())
        .and_then(|s| s.get(entity))
        .copied()
        .expect("no Grounded")
}

/// The claim the whole design rests on: the capsule holds a gap and does
/// not rest on the floor.
#[test]
fn a_character_floats_at_its_ride_height() {
    let mut resources = world();
    Playing::set(&mut resources, true);
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    floor(
        &mut resources,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(20.0, 0.5, 20.0),
    );
    let hero = character(&mut resources, Vec3::new(0.0, 3.0, 0.0));

    simulate(&mut resources, 240);

    let state = grounded(&resources, hero);
    assert!(state.standing, "should have found the floor");
    let wanted = CharacterController::default().ride_height;
    assert!(
        (state.distance - wanted).abs() < 0.06,
        "held {} above the ground, wanted {wanted}",
        state.distance,
    );
    assert!(state.normal.y > 0.9, "flat floor: {}", state.normal);
}

/// It settles instead of oscillating. The landing dips on purpose —
/// see `it_dips_when_it_lands` — and a spring damped too lightly to
/// come to rest passes the height check on the frame it crosses.
#[test]
fn a_landing_settles() {
    let mut resources = world();
    Playing::set(&mut resources, true);
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    floor(
        &mut resources,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(20.0, 0.5, 20.0),
    );
    let hero = character(&mut resources, Vec3::new(0.0, 4.0, 0.0));

    simulate(&mut resources, 300);
    let settled = position(&resources, hero).y;
    simulate(&mut resources, 60);
    let later = position(&resources, hero).y;

    assert!(
        (later - settled).abs() < 0.01,
        "still moving: {settled} then {later}",
    );
}

/// Ground runs out and the character falls. A spring that pulled from
/// nothing would hold it over the void.
#[test]
fn it_falls_when_the_ground_ends() {
    let mut resources = world();
    Playing::set(&mut resources, true);
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    floor(
        &mut resources,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(2.0, 0.5, 2.0),
    );
    let hero = character(&mut resources, Vec3::new(30.0, 0.0, 0.0));

    simulate(&mut resources, 120);

    assert!(!grounded(&resources, hero).standing, "nothing is under it");
    assert!(position(&resources, hero).y < -3.0, "it should be falling");
}

/// Acceptance: a 30° slope is walked on without special-casing it — the
/// spring holds the same gap it holds on the flat.
#[test]
fn a_gentle_slope_is_ground() {
    let state = on_a_ramp(30.0);
    assert!(state.standing, "30 degrees is walkable: {}", state.normal);
    let wanted = CharacterController::default().ride_height;
    assert!(
        (state.distance - wanted).abs() < 0.12,
        "held {} on the slope, wanted about {wanted}",
        state.distance,
    );
}

/// Past `max_slope` the sweep still finds the surface and the spring
/// still pushes off it — but it is not ground. Without the distinction a
/// character can jump off a cliff face forever.
#[test]
fn a_steep_slope_is_not_ground() {
    let state = on_a_ramp(70.0);
    assert!(!state.standing, "70 degrees is a wall: {}", state.normal,);
    assert!(
        state.normal.length() > 0.5,
        "and it was still found: {}",
        state.normal,
    );
}

/// Drops a character onto a ramp tilted by `degrees` and reports what it
/// decided it was standing on.
fn on_a_ramp(degrees: f32) -> Grounded {
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
        Vec3::new(30.0, 0.5, 30.0),
        glam::Quat::from_rotation_z(degrees.to_radians()),
    );
    let hero = character(&mut resources, Vec3::new(0.0, 2.0, 0.0));

    simulate(&mut resources, 120);
    grounded(&resources, hero)
}

/// Acceptance: upright the whole way round a planet, poles included.
#[test]
fn it_stays_upright_around_a_planet() {
    let mut resources = world();
    Playing::set(&mut resources, true);
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        PointGravity {
            strength: 20.0,
            radius: 20.0,
            range: 40.0,
            inverse_square: true,
        },
    );

    // Four characters spread around a sphere, including one under it.
    for at in [
        Vec3::new(0.0, 9.0, 0.0),
        Vec3::new(9.0, 0.0, 0.0),
        Vec3::new(0.0, -9.0, 0.0),
        Vec3::new(0.0, 0.0, -9.0),
    ] {
        let planet = spawn(&mut resources);
        insert(&mut resources, planet, Transform::from_position(Vec3::ZERO));
        insert(
            &mut resources,
            planet,
            PhysicsBody {
                kind: KIND_STATIC,
                ..Default::default()
            },
        );
        insert(
            &mut resources,
            planet,
            Collider {
                radius: 7.0,
                ..Default::default()
            },
        );
        let hero = character(&mut resources, at);

        simulate(&mut resources, 400);

        let up = at.normalize();
        let state = grounded(&resources, hero);
        assert!(
            state.standing,
            "should stand at {at}, normal {}",
            state.normal
        );

        let facing = resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<Transform>())
            .and_then(|s| s.get(hero))
            .map(|t| t.rotation * Vec3::Y)
            .expect("no transform");
        assert!(
            facing.dot(up) > 0.9,
            "leaning at {at}: facing {facing}, up {up}",
        );
    }
}

/// Acceptance: zero gravity is a defined case, not a normalise of a zero
/// vector. Nothing spins, nothing becomes NaN.
#[test]
fn zero_gravity_does_not_spin_it() {
    let mut resources = world();
    Playing::set(&mut resources, true);
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity {
            acceleration: Vec3::ZERO,
        },
    );
    let hero = character(&mut resources, Vec3::new(0.0, 3.0, 0.0));

    simulate(&mut resources, 180);

    let at = position(&resources, hero);
    assert!(at.is_finite(), "{at}");
    assert!(
        (at - Vec3::new(0.0, 3.0, 0.0)).length() < 0.1,
        "drifted to {at}"
    );
    assert!(!grounded(&resources, hero).standing);
}

/// Acceptance: it stays part of the world. A kinematic controller moves
/// *through* the scene; this one pushes and is pushed.
#[test]
fn it_pushes_a_crate_and_is_pushed_back() {
    let mut resources = world();
    Playing::set(&mut resources, true);
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    floor(
        &mut resources,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(30.0, 0.5, 30.0),
    );

    let hero = character(&mut resources, Vec3::new(0.0, 0.5, 0.0));
    let crate_entity = body_at(&mut resources, Vec3::new(1.2, 0.5, 0.0));

    // Let both settle before anything is pushed, so the movement below
    // is the only thing that could have moved the crate.
    simulate(&mut resources, 180);
    let crate_start = position(&resources, crate_entity).x;

    // Walking pace, straight at it.
    for _ in 0..120 {
        let body = resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<kooch_physics::plugin::SolverBody>())
            .and_then(|s| s.get(hero))
            .copied();
        if let Some(body) = body
            && let Some(world) = resources.get_mut::<kooch_physics::plugin::PhysicsWorld>()
        {
            world.apply_impulse(body, Vec3::X * 0.15);
        }
        simulate(&mut resources, 1);
    }

    let moved = position(&resources, crate_entity).x - crate_start;
    assert!(moved > 0.3, "the crate should have been shoved: {moved}");
    assert!(
        grounded(&resources, hero).standing,
        "and the character should still be on the floor",
    );
}

/// Walks the character at `entity` along +X for `steps` frames, at a
/// walking pace, and reports the highest it ever got.
fn walk(resources: &mut Resources, entity: Entity, steps: u32) -> f32 {
    let mut highest = f32::MIN;
    for _ in 0..steps {
        let body = resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<kooch_physics::plugin::SolverBody>())
            .and_then(|s| s.get(entity))
            .copied();
        if let Some(body) = body
            && let Some(world) = resources.get_mut::<PhysicsWorld>()
        {
            let speed = world.linear_velocity(body).unwrap_or(Vec3::ZERO).x;
            if speed < 3.0 {
                world.apply_impulse(body, Vec3::X * 0.05);
            }
        }
        simulate(resources, 1);
        highest = highest.max(position(resources, entity).y);
    }
    highest
}

/// How far up a riser of `height` the character gets, in metres.
fn onto_a_step(height: f32) -> f32 {
    let mut resources = world();
    Playing::set(&mut resources, true);
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    floor(
        &mut resources,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(4.0, 0.5, 6.0),
    );
    // Long enough that reaching the top is the only way past it.
    floor(
        &mut resources,
        Vec3::new(10.0, height - 2.5, 0.0),
        Vec3::new(6.0, 2.0, 6.0),
    );
    let hero = character(&mut resources, Vec3::new(0.0, 0.5, 0.0));
    simulate(&mut resources, 180);
    let start = position(&resources, hero).y;
    walk(&mut resources, hero, 300) - start
}

/// The headline claim: a step is climbed by the spring alone, with
/// nothing in the code that knows what a step is.
#[test]
fn a_low_step_is_climbed() {
    let rose = onto_a_step(0.6);
    assert!(rose > 0.55, "should have got up a 0.6 m step, rose {rose}");
}

/// And the other half — the same mechanism has to refuse a wall, or
/// "climbs steps" means "walks through the level".
#[test]
fn a_tall_step_is_not() {
    let rose = onto_a_step(1.0);
    assert!(
        rose < 0.2,
        "should have been stopped by a 1 m wall, rose {rose}"
    );
}

/// A flat world with a character standing on it, settled.
fn on_the_floor(resources: &mut Resources) -> Entity {
    Playing::set(resources, true);
    source_at(
        resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    floor(
        resources,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(20.0, 0.5, 20.0),
    );
    let hero = character(resources, Vec3::new(0.0, 0.5, 0.0));
    simulate(resources, 180);
    hero
}

/// Acceptance: it points where it is steered. Without a `Facing` the
/// controller only ever stood the body up, and a character that walks
/// sideways for ever is what that looks like.
#[test]
fn it_faces_where_it_walks() {
    let mut resources = world();
    let hero = on_the_floor(&mut resources);

    let steered = Vec3::new(1.0, 0.0, 1.0).normalize();
    insert(&mut resources, hero, Facing { direction: steered });
    simulate(&mut resources, 120);

    let looking = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Transform>())
        .and_then(|s| s.get(hero))
        .map(|t| t.rotation * Vec3::NEG_Z)
        .expect("no transform");
    assert!(
        looking.dot(steered) > 0.99,
        "should look along {steered}, looked along {looking}",
    );
}

/// Acceptance: a jump leaves the floor. The spring's own damping fights
/// the launch the frame after it starts — 18 damping against 5 m/s is
/// 90 m/s² of "come back" — so without letting go while rising, the
/// character never gets off the ground.
#[test]
fn a_jump_leaves_the_ground() {
    let mut resources = world();
    let hero = on_the_floor(&mut resources);
    let resting = position(&resources, hero).y;

    let body = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<kooch_physics::plugin::SolverBody>())
        .and_then(|s| s.get(hero))
        .copied()
        .expect("no body");
    if let Some(world) = resources.get_mut::<PhysicsWorld>() {
        let mass = world.mass(body).unwrap_or(1.0);
        world.apply_impulse(body, Vec3::Y * 5.0 * mass);
    }

    let mut highest = resting;
    for _ in 0..120 {
        simulate(&mut resources, 1);
        highest = highest.max(position(&resources, hero).y);
    }
    // 5 m/s against 9.81 is 1.27 m of arc. Anything under half of that
    // is the spring winning.
    assert!(
        highest - resting > 0.6,
        "jumped {} m from {resting}",
        highest - resting,
    );
}

/// A landing that dips and comes back. At critical damping the body
/// arrives dead — bottomed and settled agree to seven decimals — which
/// is correct and reads as a character with no weight.
#[test]
fn it_dips_when_it_lands() {
    let mut resources = world();
    Playing::set(&mut resources, true);
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    floor(
        &mut resources,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(20.0, 0.5, 20.0),
    );
    let hero = character(&mut resources, Vec3::new(0.0, 5.0, 0.0));

    let mut lowest = f32::MAX;
    for _ in 0..240 {
        simulate(&mut resources, 1);
        lowest = lowest.min(position(&resources, hero).y);
    }
    let settled = position(&resources, hero).y;
    assert!(
        settled - lowest > 0.08,
        "should have dipped and recovered: bottomed at {lowest}, settled at {settled}",
    );
}

/// Steers a character for `steps` frames and returns its ground speed.
fn walked(resources: &mut Resources, hero: Entity, steered: Vec3, steps: u32) -> f32 {
    insert(resources, hero, Facing { direction: steered });
    simulate(resources, steps);
    let velocity = resources
        .get::<PhysicsWorld>()
        .and_then(|world| {
            let body = resources
                .get::<ComponentRegistry>()
                .and_then(|r| r.get_cpu::<kooch_physics::plugin::SolverBody>())
                .and_then(|s| s.get(hero))
                .copied()?;
            world.linear_velocity(body)
        })
        .unwrap_or(Vec3::ZERO);
    Vec3::new(velocity.x, 0.0, velocity.z).length()
}

/// Acceptance: it stops when you let go.
///
/// A floating capsule never touches the floor, so it has no friction at
/// all. Pushing in a direction and stopping at top speed leaves nothing
/// to slow it down — the character coasts for ever. Asking for a
/// velocity makes stopping the same term as starting.
#[test]
fn it_stops_when_you_let_go() {
    let mut resources = world();
    let hero = on_the_floor(&mut resources);
    insert(&mut resources, hero, Walk::default());

    let cruising = walked(&mut resources, hero, Vec3::X, 180);
    assert!(cruising > 4.0, "should be walking: {cruising}");

    let stopped = walked(&mut resources, hero, Vec3::ZERO, 60);
    assert!(stopped < 0.3, "should have stopped: {stopped} m/s");
}

/// And the top speed is the goal's, not a clamp applied afterwards.
#[test]
fn it_holds_its_top_speed() {
    let mut resources = world();
    let hero = on_the_floor(&mut resources);
    let steps = Walk::default();
    insert(&mut resources, hero, steps);

    let reached = walked(&mut resources, hero, Vec3::X, 240);
    assert!(
        (reached - steps.max_speed).abs() < 0.4,
        "wanted {}, reached {reached}",
        steps.max_speed,
    );
}

/// Nobody steering is nobody moving. The spring and the lean both act
/// along the local up, and a stationary character that drifts means one
/// of them is leaking sideways.
#[test]
fn a_standing_character_does_not_drift() {
    let mut resources = world();
    let hero = on_the_floor(&mut resources);
    insert(&mut resources, hero, Walk::default());

    insert(
        &mut resources,
        hero,
        Facing {
            direction: Vec3::ZERO,
        },
    );
    let start = position(&resources, hero);
    simulate(&mut resources, 240);
    let drift = position(&resources, hero) - start;
    let across = Vec3::new(drift.x, 0.0, drift.z).length();
    assert!(across < 0.1, "it drifted {across} m");
}

/// Acceptance: letting go mid-jump keeps the momentum.
///
/// The goal-velocity chase brakes towards zero, so on the ground
/// releasing the stick stops the character — which is the point. In the
/// air it stopped it dead in mid-flight, which is not a jump.
#[test]
fn a_jump_keeps_its_momentum() {
    let mut resources = world();
    let hero = on_the_floor(&mut resources);
    insert(&mut resources, hero, Walk::default());

    let running = walked(&mut resources, hero, Vec3::X, 180);
    assert!(running > 4.0, "should be walking: {running}");

    let body = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<kooch_physics::plugin::SolverBody>())
        .and_then(|s| s.get(hero))
        .copied()
        .expect("no body");
    if let Some(world) = resources.get_mut::<PhysicsWorld>() {
        let mass = world.mass(body).unwrap_or(1.0);
        world.apply_impulse(body, Vec3::Y * 6.0 * mass);
    }

    // Off the ground, and the stick released at the top of the arc.
    simulate(&mut resources, 20);
    let coasting = walked(&mut resources, hero, Vec3::ZERO, 25);
    assert!(
        coasting > running * 0.8,
        "it stopped in mid-air: {running} became {coasting}",
    );
}

/// Acceptance: it stands on the ramp, not on the field.
///
/// A character that walks up a slope bolt upright reads as a sprite
/// being slid along it.
#[test]
fn it_stands_on_a_ramp() {
    let mut resources = world();
    Playing::set(&mut resources, true);
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    let tilt = 25f32.to_radians();
    slab(
        &mut resources,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(12.0, 0.5, 12.0),
        glam::Quat::from_rotation_z(tilt),
    );
    let hero = character(&mut resources, Vec3::new(0.0, 2.0, 0.0));
    simulate(&mut resources, 300);

    let standing = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Transform>())
        .and_then(|s| s.get(hero))
        .map(|t| t.rotation * Vec3::Y)
        .expect("no transform");
    let surface = grounded(&resources, hero).normal.normalize();
    assert!(
        standing.dot(surface) > 0.98,
        "should stand on the ramp: body up {standing}, surface {surface}",
    );
    assert!(
        standing.y < 0.995,
        "and that is not straight up: {standing}"
    );
}
