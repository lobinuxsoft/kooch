//! #94's acceptance list, against a real solver.
//!
//! Unit tests can say the spring arithmetic is right. Only this can say
//! a character *stands* — the maths being correct and the impulse
//! reaching a body that then holds its height are different claims, and
//! the second is the one that has ever been wrong.

use glam::Vec3;

use kooch_character::plugin::{cling_and_leap, hold_characters};
use kooch_character::{
    CharacterController, Facing, Grounded, Jump, Sprint, Touching, Walk, WallJump, WallRun,
    WallSlide,
};
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
    registry.register_cpu_reflected::<Jump>();
    registry.register_cpu_reflected::<Sprint>();
    registry.register_cpu_reflected::<Touching>();
    registry.register_cpu_reflected::<WallJump>();
    registry.register_cpu_reflected::<WallRun>();
    registry.register_cpu_reflected::<WallSlide>();
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
            cling_and_leap(resources);
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

/// It stays upright against the field, on a ramp as anywhere else.
///
/// Standing perpendicular to every surface swings the body as the
/// ground changes and tips it sideways on a slope it is only crossing.
#[test]
fn a_ramp_does_not_tip_it() {
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
    assert!(
        standing.y > 0.999,
        "should stand straight up on a ramp: {standing}",
    );
}

/// Acceptance: walking up a ramp is not leaving the ground.
///
/// The rise test used to read the speed along the field, where climbing
/// a 25 degree ramp at 6 m/s reads as 2.5 — five times the threshold.
/// The character looked like it was already jumping, so the spring let
/// go and `standing` went false: you could not jump on a slope.
#[test]
fn a_climb_is_still_standing() {
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
        Vec3::new(16.0, 0.5, 12.0),
        glam::Quat::from_rotation_z(25f32.to_radians()),
    );
    let hero = character(&mut resources, Vec3::new(-4.0, 4.0, 0.0));
    insert(&mut resources, hero, Walk::default());
    simulate(&mut resources, 240);

    // Uphill, at walking pace, for long enough that a frame is not luck.
    let mut refused = 0;
    insert(&mut resources, hero, Facing { direction: Vec3::X });
    for _ in 0..180 {
        simulate(&mut resources, 1);
        if !grounded(&resources, hero).standing {
            refused += 1;
        }
    }
    assert!(refused < 10, "lost the ground {refused} frames out of 180");
}

/// Acceptance: a slope too steep to walk takes the character back down.
///
/// The spring cancels gravity, so holding the body up against a surface
/// it has already refused to walk carried it straight to the top —
/// climbing a cliff by standing on it. A step's riser gives the *same*
/// contact normal, which is why this is decided by looking for a ledge
/// rather than by the normal alone.
#[test]
fn a_steep_slope_slides() {
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
        Vec3::new(16.0, 0.5, 12.0),
        glam::Quat::from_rotation_z(65f32.to_radians()),
    );
    let hero = character(&mut resources, Vec3::new(-2.0, 6.0, 0.0));
    insert(&mut resources, hero, Walk::default());
    simulate(&mut resources, 60);

    // Walking straight up it, as hard as it can.
    let start = position(&resources, hero).y;
    insert(&mut resources, hero, Facing { direction: Vec3::X });
    simulate(&mut resources, 180);
    let ended = position(&resources, hero).y;

    assert!(
        ended < start,
        "should have slid down a 65 degree slope, went from {start} to {ended}",
    );
    assert!(
        !grounded(&resources, hero).standing,
        "and never counted as standing on it",
    );
}

/// Acceptance: the wall a character is pressed against has a name.
///
/// Without it a wall slide, a wall jump and a shoulder animation each
/// cast their own probe — three chances to disagree about whether there
/// is a wall, which is the mistake `Grounded` was made to stop.
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

/// Acceptance: shoving a wall does not tip the character over.
///
/// The lean used to be drawn from the force applied. A body pressed
/// against a wall is given the whole `max_force` and goes nowhere, so it
/// leaned `atan(max_force / g) * lean` — 29 degrees — and stayed there
/// as long as the stick was held.
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

/// A tall wall with its face at `x`, and a character beside it.
fn against_a_wall(resources: &mut Resources) -> Entity {
    Playing::set(resources, true);
    source_at(
        resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    // Far enough down that two seconds of falling never reaches it — a
    // character that lands mid-test measures the floor, not the wall.
    floor(
        resources,
        Vec3::new(0.0, -200.0, 0.0),
        Vec3::new(40.0, 0.5, 40.0),
    );
    // Face at x = 1, tall enough to fall down all of and long enough to
    // run along without reaching the end.
    let wall = spawn(resources);
    insert(
        resources,
        wall,
        Transform::from_position(Vec3::new(3.0, -60.0, 0.0)),
    );
    insert(
        resources,
        wall,
        PhysicsBody {
            kind: KIND_STATIC,
            ..Default::default()
        },
    );
    insert(
        resources,
        wall,
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(2.0, 80.0, 60.0),
            // Frictionless, so a wall test measures the mechanic rather
            // than rapier's Coulomb friction — which alone holds a
            // character pressed into a wall almost still.
            friction: 0.0,
            ..Default::default()
        },
    );

    let hero = character(resources, Vec3::new(0.2, 0.0, 0.0));
    // Frictionless on both sides: the combining rule takes the larger,
    // so a slick wall alone still leaves the character's own 0.4.
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<Collider>()
        && let Some(collider) = storage.get_mut(hero)
    {
        collider.friction = 0.0;
    }
    insert(resources, hero, Walk::default());
    insert(resources, hero, Touching::default());
    insert(resources, hero, Facing { direction: Vec3::X });
    hero
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

/// Acceptance: it stays on the wall after arriving at speed.
///
/// The solver pushes the capsule back out of whatever it hits, and with
/// the air push deliberately not aimed into the wall there is nothing to
/// bring it back — the character bounced off and drifted away mid-slide.
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

/// Thrown along the wall at `speed`, and left to run.
fn running(resources: &mut Resources, run: WallRun, speed: f32) -> Entity {
    thrown(resources, run, speed, Vec3::new(0.15, 0.0, 1.0))
}

/// The same, steering where you say — including nowhere, for a test
/// about the speed a character *arrives* with rather than the speed air
/// steering works it up to on the way in.
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
    // The side probes see the wall from further away than the character
    // starts, so the entry was judged on the spawn frame — at rest, and
    // refused before this test had given it any speed. A player arrives
    // already moving; this makes the harness do the same.
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

/// Arriving slowly is not a run. Without the entry speed a wall run is
/// a cling with extra steps.
///
/// Asks the run's own clock rather than measuring the fall: a body
/// pressed to a wall keeps some of rapier's friction whatever the
/// colliders say, and it happens to slow a fall by about as much as the
/// run does — so the drop cannot tell the two apart, and this claim is
/// about the entry speed.
#[test]
fn a_slow_arrival_does_not_run() {
    assert!(ran(9.0), "should have started a run at speed");
    assert!(!ran(0.5), "and not at a crawl");
}

/// The clock is what ends it, and the sag before that is what tells the
/// player it is going to.
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

/// Acceptance: it keeps hold of the wall it is running along.
///
/// The forward probe looks where the character is *going*, and once a
/// run is under way that is along the wall rather than at it — so the
/// wall dropped out every few frames and took the run with it.
#[test]
fn a_run_keeps_the_wall() {
    let mut resources = world();
    // Steered purely along the wall, which is what a player holds during
    // a wall run — and the case where a probe aimed where the character
    // is going stops pointing at the wall at all.
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

/// Measurement: the whole shape of a jump, in seconds and metres.
///
/// Reported as *"el salto parece que flota en el aire"*, which is a
/// feeling until it is a number. Prints time to the apex, the apex, the
/// fall, and the total — because "floaty" is about **time**, not height,
/// and the two are set by different knobs.
///
/// Run with `--ignored --nocapture`.
#[test]
#[ignore = "measurement"]
fn jump_profile() {
    let mut resources = world();
    let hero = on_the_floor(&mut resources);
    let resting = position(&resources, hero).y;
    // The harness never advances `Time`, so the step is the physics
    // fallback the systems actually used — `FALLBACK_DT` in
    // `kooch_physics::plugin::systems`.
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
    // 🔴 The comparison that names the cause. A parabola is symmetric:
    // falling back from the apex under the same gravity takes the time
    // it took to rise. Anything beyond that is something holding the
    // character up on the way down.
    let free = (2.0 * (highest - resting) / 9.81).sqrt();
    println!("free fall from that apex would be {free:.3} s");
    println!(
        "the descent takes {:.3} s more than gravity asks for",
        (land - rise) - free
    );
    println!();
    println!("For reference: a platformer jump is usually 0.4-0.7 s in total.");
}
