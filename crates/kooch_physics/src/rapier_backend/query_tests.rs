//! #562's acceptance list, against a real pipeline.

use glam::{Quat, Vec3};

use crate::backend::{BodyDesc, CollisionShape, PhysicsBackend, QueryFilter, ShapeAt};

use super::backend::RapierBackend;

fn wall(backend: &mut RapierBackend, at: Vec3, half: Vec3) -> crate::backend::BodyHandle {
    backend.add_body(BodyDesc::static_at(
        CollisionShape::Cuboid { half_extents: half },
        at,
    ))
}

/// The query a ray cannot answer: a capsule is not a line, and where it
/// stops is where its *side* touches, not where its centre would.
#[test]
fn a_capsule_stops_at_the_wall() {
    let mut backend = RapierBackend::new();
    let body = wall(
        &mut backend,
        Vec3::new(0.0, 0.0, 5.0),
        Vec3::new(4.0, 4.0, 0.5),
    );

    let capsule = CollisionShape::Capsule {
        radius: 0.5,
        half_height: 1.0,
    };
    let hit = backend
        .query_sweep(
            ShapeAt::new(&capsule, Vec3::ZERO),
            Vec3::Z,
            100.0,
            QueryFilter::ALL,
        )
        .expect("the sweep should reach the wall");

    assert_eq!(hit.body, body);
    // Wall face at z = 4.5, capsule radius 0.5, so contact at t = 4.
    assert!((hit.t - 4.0).abs() < 0.05, "stopped at {}", hit.t);
    // The wall faces back along -Z, towards where the capsule came from.
    // Not the swept shape's normal, which points the other way and reads
    // just as plausible.
    assert!(hit.normal.z < -0.9, "normal was {}", hit.normal);
    // And the contact is on the wall's face, in world space — not in
    // some shape's local frame.
    assert!((hit.point.z - 4.5).abs() < 0.05, "contact at {}", hit.point);
    assert!(!hit.penetrating);
}

/// A ray of zero width slips through a gap a body could never fit in.
/// This is the whole reason the sweep exists.
#[test]
fn a_ray_fits_where_a_capsule_does_not() {
    let mut backend = RapierBackend::new();
    // Two slabs with a 0.4 m gap on the axis the query travels down.
    wall(
        &mut backend,
        Vec3::new(-1.2, 0.0, 5.0),
        Vec3::new(1.0, 4.0, 0.5),
    );
    wall(
        &mut backend,
        Vec3::new(1.2, 0.0, 5.0),
        Vec3::new(1.0, 4.0, 0.5),
    );

    assert!(
        backend
            .query_ray(Vec3::ZERO, Vec3::Z, 100.0, QueryFilter::ALL)
            .is_none(),
        "a line of zero width goes through the gap",
    );

    let ball = CollisionShape::Sphere { radius: 0.5 };
    assert!(
        backend
            .query_sweep(
                ShapeAt::new(&ball, Vec3::ZERO),
                Vec3::Z,
                100.0,
                QueryFilter::ALL
            )
            .is_some(),
        "a half-metre ball does not fit through a gap of 0.4",
    );
}

/// Sweeping is what stops something fast from ending the step on the far
/// side of a wall it never touched.
#[test]
fn a_fast_shape_does_not_tunnel() {
    let mut backend = RapierBackend::new();
    wall(
        &mut backend,
        Vec3::new(0.0, 0.0, 50.0),
        Vec3::new(4.0, 4.0, 0.02),
    );

    let ball = CollisionShape::Sphere { radius: 0.1 };
    // A step's worth of travel at 6 km/h... per frame: 100 m in one go,
    // straight through a four-centimetre wall.
    let hit = backend.query_sweep(
        ShapeAt::new(&ball, Vec3::ZERO),
        Vec3::Z,
        100.0,
        QueryFilter::ALL,
    );
    let hit = hit.expect("the sweep should catch the thin wall");
    assert!((hit.t - 49.88).abs() < 0.05, "stopped at {}", hit.t);
}

#[test]
fn a_point_projects_onto_the_nearest_surface() {
    let mut backend = RapierBackend::new();
    let body = wall(&mut backend, Vec3::ZERO, Vec3::splat(1.0));

    let outside = backend
        .query_point(Vec3::new(3.0, 0.0, 0.0), 100.0, QueryFilter::ALL)
        .expect("something is within 100 m");
    assert_eq!(outside.body, body);
    assert!(!outside.inside);
    assert!((outside.point - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-3);

    let inside = backend
        .query_point(Vec3::new(0.5, 0.0, 0.0), 100.0, QueryFilter::ALL)
        .expect("the point is in the box");
    assert!(inside.inside, "a point inside should say so");
}

#[test]
fn an_overlap_finds_every_body_in_reach() {
    let mut backend = RapierBackend::new();
    let near = wall(&mut backend, Vec3::new(1.0, 0.0, 0.0), Vec3::splat(0.4));
    let also = wall(&mut backend, Vec3::new(-1.0, 0.0, 0.0), Vec3::splat(0.4));
    let far = wall(&mut backend, Vec3::new(40.0, 0.0, 0.0), Vec3::splat(0.4));

    let blast = CollisionShape::Sphere { radius: 2.0 };
    let mut caught = Vec::new();
    backend.query_overlaps(
        ShapeAt::new(&blast, Vec3::ZERO),
        QueryFilter::ALL,
        &mut |body| {
            caught.push(body);
            true
        },
    );

    assert!(
        caught.contains(&near) && caught.contains(&also),
        "{caught:?}"
    );
    assert!(!caught.contains(&far), "the far one is out of the blast");
    assert_eq!(caught.len(), 2);
}

/// Without this a character's ground probe finds the character. Dropping
/// only the nearest hit is not a fix: a body with two colliders answers
/// twice.
#[test]
fn a_filter_hides_the_asking_body() {
    let mut backend = RapierBackend::new();
    let asking = backend.add_body(BodyDesc::static_at(
        CollisionShape::Sphere { radius: 1.0 },
        Vec3::ZERO,
    ));
    let floor = wall(
        &mut backend,
        Vec3::new(0.0, -5.0, 0.0),
        Vec3::new(9.0, 0.5, 9.0),
    );

    let found = backend
        .query_ray(Vec3::ZERO, Vec3::NEG_Y, 100.0, QueryFilter::ALL)
        .expect("something is below");
    assert_eq!(found.body, asking, "unfiltered, a body finds itself");

    let found = backend
        .query_ray(
            Vec3::ZERO,
            Vec3::NEG_Y,
            100.0,
            QueryFilter::excluding(asking),
        )
        .expect("the floor is still there");
    assert_eq!(found.body, floor);
}

/// The editor's case. Since 0.34 the pipeline is a view over the
/// broad-phase BVH, which only fills while stepping — so a body that has
/// never been simulated has to publish its AABB or queries miss it.
#[test]
fn a_query_works_without_a_step() {
    let mut backend = RapierBackend::new();
    let body = wall(&mut backend, Vec3::new(0.0, 0.0, 5.0), Vec3::splat(1.0));

    let ball = CollisionShape::Sphere { radius: 0.5 };
    let hit = backend.query_sweep(
        ShapeAt::new(&ball, Vec3::ZERO),
        Vec3::Z,
        100.0,
        QueryFilter::ALL,
    );
    assert_eq!(hit.map(|hit| hit.body), Some(body), "no step has run");
}

/// A turned shape sweeps as it is turned, not axis-aligned.
#[test]
fn a_turned_shape_sweeps_turned() {
    let mut backend = RapierBackend::new();
    wall(
        &mut backend,
        Vec3::new(0.0, 0.0, 6.0),
        Vec3::new(9.0, 9.0, 0.5),
    );

    // A long thin box, lying along Y. Upright it reaches the wall with
    // its narrow end; laid along Z it reaches four metres sooner.
    let rod = CollisionShape::Cuboid {
        half_extents: Vec3::new(0.1, 4.0, 0.1),
    };
    let upright = backend
        .query_sweep(
            ShapeAt::new(&rod, Vec3::ZERO),
            Vec3::Z,
            100.0,
            QueryFilter::ALL,
        )
        .expect("upright hit");
    let lying = backend
        .query_sweep(
            ShapeAt::new(&rod, Vec3::ZERO)
                .turned(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
            Vec3::Z,
            100.0,
            QueryFilter::ALL,
        )
        .expect("lying hit");

    assert!(
        lying.t < upright.t - 3.0,
        "turning it should matter: {} vs {}",
        lying.t,
        upright.t,
    );
}

/// Multi-hit walks past the first thing, which is the point.
#[test]
fn a_piercing_ray_finds_them_all() {
    let mut backend = RapierBackend::new();
    for z in [3.0, 6.0, 9.0] {
        wall(
            &mut backend,
            Vec3::new(0.0, 0.0, z),
            Vec3::new(2.0, 2.0, 0.2),
        );
    }

    let mut count = 0;
    backend.query_ray_all(Vec3::ZERO, Vec3::Z, 100.0, QueryFilter::ALL, &mut |_| {
        count += 1;
        true
    });
    assert_eq!(count, 3, "all three walls");

    let mut seen = 0;
    backend.query_ray_all(Vec3::ZERO, Vec3::Z, 100.0, QueryFilter::ALL, &mut |_| {
        seen += 1;
        false
    });
    assert_eq!(seen, 1, "returning false stops the walk");
}

/// A cast that starts inside something says so, instead of reading as
/// open space. A controller that misses this pushes itself further into
/// whatever it is stuck in.
#[test]
fn a_cast_that_starts_stuck_says_so() {
    let mut backend = RapierBackend::new();
    wall(&mut backend, Vec3::ZERO, Vec3::splat(2.0));

    let ball = CollisionShape::Sphere { radius: 0.5 };
    let hit = backend
        .query_sweep(
            ShapeAt::new(&ball, Vec3::ZERO),
            Vec3::Z,
            10.0,
            QueryFilter::ALL,
        )
        .expect("it is inside the box");
    assert!(hit.penetrating);
    assert_eq!(hit.t, 0.0);
}

/// A sensor is not a floor and not a wall.
#[test]
fn sensors_are_skipped_unless_asked_for() {
    let mut backend = RapierBackend::new();
    let mut desc = BodyDesc::static_at(
        CollisionShape::Cuboid {
            half_extents: Vec3::new(4.0, 4.0, 0.5),
        },
        Vec3::new(0.0, 0.0, 3.0),
    );
    desc.interaction.sensor = true;
    let trigger = backend.add_body(desc);
    let solid = wall(
        &mut backend,
        Vec3::new(0.0, 0.0, 8.0),
        Vec3::new(4.0, 4.0, 0.5),
    );

    let hit = backend
        .query_ray(Vec3::ZERO, Vec3::Z, 100.0, QueryFilter::ALL)
        .expect("the solid wall is behind it");
    assert_eq!(hit.body, solid, "a trigger volume is not a wall");

    let hit = backend
        .query_ray(Vec3::ZERO, Vec3::Z, 100.0, QueryFilter::ALL.with_sensors())
        .expect("asked for triggers this time");
    assert_eq!(hit.body, trigger);
}
