use super::*;
use kooch_ecs::entity::Entity;

fn vcam(index: u32) -> Entity {
    Entity::new(index, 0)
}

fn started(duration: f32) -> CameraBlend {
    let mut b = CameraBlend::default();
    b.begin(vcam(1), (Vec3::ZERO, glam::Quat::IDENTITY), duration);
    b
}

#[test]
fn a_zero_duration_handover_is_a_cut() {
    assert!(!started(0.0).running(), "zero seconds must not blend");
}

#[test]
fn a_handover_runs_for_its_duration_and_then_stops() {
    let mut b = started(0.5);
    assert!(b.running());
    b.elapsed = 0.49;
    assert!(b.running());
    b.elapsed = 0.5;
    assert!(!b.running(), "it should be done at exactly its duration");
}

/// The interruption case. Taking over mid-blend has to continue from
/// the pose on screen, not from the outgoing vcam — which is behind
/// the camera by however far the blend had got.
#[test]
fn interrupting_a_handover_starts_from_where_the_camera_is() {
    let mut b = started(1.0);
    b.elapsed = 0.5;
    let on_screen = Vec3::new(3.0, 1.0, -2.0);
    b.begin(vcam(2), (on_screen, glam::Quat::IDENTITY), 1.0);

    assert_eq!(b.active, Some(vcam(2)));
    assert_eq!(b.from_pos, on_screen);
    assert_eq!(b.elapsed, 0.0, "the new handover starts at the beginning");
}

/// `q` and `-q` are the same rotation. Without picking the shorter
/// arc a small handover can roll almost all the way round.
#[test]
fn the_slerp_takes_the_short_way() {
    let from = glam::Quat::IDENTITY;
    let to = -glam::Quat::from_rotation_y(0.2);
    let quarter = short_slerp(from, to, 0.25);
    assert!(
        quarter.angle_between(from) < 0.1,
        "went the long way: {} rad at t=0.25",
        quarter.angle_between(from),
    );
}

#[test]
fn the_slerp_reaches_its_destination() {
    let from = glam::Quat::IDENTITY;
    let to = glam::Quat::from_rotation_z(1.0);
    assert!(short_slerp(from, to, 1.0).angle_between(to) < 1e-4);
}

// ---- target resolution by tag -------------------------------------

use kooch_ecs::EntityAllocator;
use kooch_ecs::component::ComponentRegistry;

/// A registry with `CameraTarget` registered and nothing in it.
fn target_registry() -> (ComponentRegistry, EntityAllocator) {
    let mut registry = ComponentRegistry::new();
    registry.register_cpu::<CameraTarget>();
    (registry, EntityAllocator::new())
}

fn tag(
    registry: &mut ComponentRegistry,
    allocator: &mut EntityAllocator,
    group: u32,
    weight: f32,
) -> Entity {
    let entity = allocator.spawn();
    registry
        .get_cpu_mut::<CameraTarget>()
        .expect("registered above")
        .insert(entity, CameraTarget { group, weight });
    entity
}

/// Positions handed out by entity index, so a test can say where a
/// tagged entity is without a transform storage.
fn poses(placed: Vec<(Entity, Vec3)>) -> impl Fn(Entity) -> Option<(Vec3, glam::Quat)> {
    move |entity| {
        placed
            .iter()
            .find(|(candidate, _)| *candidate == entity)
            .map(|(_, position)| (*position, glam::Quat::IDENTITY))
    }
}

#[test]
fn a_vcam_follows_the_entity_tagged_with_its_group() {
    let (mut registry, mut allocator) = target_registry();
    let subject = tag(&mut registry, &mut allocator, 0, 1.0);
    let at = Vec3::new(1.0, 2.0, 3.0);

    let pose = target_pose(
        registry.get_cpu::<CameraTarget>(),
        0,
        &poses(vec![(subject, at)]),
    );

    assert_eq!(pose.map(|(position, _)| position), Some(at));
}

#[test]
fn a_vcam_ignores_targets_of_another_group() {
    let (mut registry, mut allocator) = target_registry();
    let other = tag(&mut registry, &mut allocator, 7, 1.0);

    let pose = target_pose(
        registry.get_cpu::<CameraTarget>(),
        0,
        &poses(vec![(other, Vec3::ONE)]),
    );

    assert!(pose.is_none(), "group 0 followed a member of group 7");
}

#[test]
fn nothing_tagged_means_nothing_to_follow() {
    let (registry, _) = target_registry();
    let pose = target_pose(registry.get_cpu::<CameraTarget>(), 0, &poses(vec![]));
    assert!(pose.is_none());
}

/// The case the tag exists for: several members are a group.
#[test]
fn two_members_of_a_group_are_followed_at_their_centre() {
    let (mut registry, mut allocator) = target_registry();
    let a = tag(&mut registry, &mut allocator, 0, 1.0);
    let b = tag(&mut registry, &mut allocator, 0, 1.0);

    let pose = target_pose(
        registry.get_cpu::<CameraTarget>(),
        0,
        &poses(vec![(a, Vec3::ZERO), (b, Vec3::new(10.0, 0.0, 0.0))]),
    );

    assert_eq!(
        pose.map(|(position, _)| position),
        Some(Vec3::new(5.0, 0.0, 0.0))
    );
}

/// Orientation comes from one member, not from an average — averaging
/// quaternions across a group produces an up-vector nobody asked for.
#[test]
fn the_heaviest_member_owns_the_orientation() {
    let (mut registry, mut allocator) = target_registry();
    let light = allocator.spawn();
    let heavy = allocator.spawn();
    {
        let storage = registry.get_cpu_mut::<CameraTarget>().unwrap();
        storage.insert(
            light,
            CameraTarget {
                group: 0,
                weight: 1.0,
            },
        );
        storage.insert(
            heavy,
            CameraTarget {
                group: 0,
                weight: 5.0,
            },
        );
    }
    let turned = glam::Quat::from_rotation_y(1.0);
    let pose_of = move |entity: Entity| -> Option<(Vec3, glam::Quat)> {
        if entity == heavy {
            Some((Vec3::ZERO, turned))
        } else {
            Some((Vec3::ZERO, glam::Quat::IDENTITY))
        }
    };

    let (_, rotation) =
        target_pose(registry.get_cpu::<CameraTarget>(), 0, &pose_of).expect("two members");

    assert!(
        rotation.angle_between(turned) < 1e-5,
        "the light member's orientation won"
    );
}

/// A tagged entity with no transform yet must not void the group.
#[test]
fn a_member_with_no_pose_is_skipped_rather_than_fatal() {
    let (mut registry, mut allocator) = target_registry();
    let placed = tag(&mut registry, &mut allocator, 0, 1.0);
    let unplaced = tag(&mut registry, &mut allocator, 0, 1.0);
    let _ = unplaced;
    let at = Vec3::new(4.0, 0.0, 0.0);

    let pose = target_pose(
        registry.get_cpu::<CameraTarget>(),
        0,
        &poses(vec![(placed, at)]),
    );

    assert_eq!(
        pose.map(|(position, _)| position),
        Some(at),
        "the member without a pose should be skipped, not void the group"
    );
}
