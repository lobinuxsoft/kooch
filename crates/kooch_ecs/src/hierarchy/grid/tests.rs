use glam::{DVec3, IVec3, Vec3};
use kooch_core::resource::Resources;
use kooch_core::run_state::Playing;

use super::*;
use crate::allocator::EntityAllocator;
use crate::archetype_registry::ArchetypeRegistry;
use crate::commands::Commands;
use crate::hierarchy::{Children, Parent, hierarchy_sync_system, transform_propagation_system};
use crate::query::AccessTracker;

fn world() -> Resources {
    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());
    resources.insert(Commands::new());
    resources.insert(Playing(true));
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<Transform>();
    registry.register_cpu_reflected::<Parent>();
    registry.register_cpu_reflected::<Children>();
    registry.register_cpu_reflected::<GlobalTransform>();
    registry.register_cpu_reflected::<GridCell>();
    registry.register_cpu_reflected::<FloatingOrigin>();
    resources
}

fn spawn(resources: &mut Resources, at: DVec3) -> Entity {
    let mut commands = resources.remove::<Commands>().unwrap();
    let entity = commands.spawn(resources).id();
    commands.apply(resources);
    resources.insert(commands);
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    let transform = Transform::from_position(at);
    registry
        .get_cpu_mut::<Transform>()
        .unwrap()
        .insert(entity, transform);
    entity
}

fn global(resources: &Resources, entity: Entity) -> Vec3 {
    let registry = resources.get::<ComponentRegistry>().unwrap();
    let globals = registry.get_cpu::<GlobalTransform>().unwrap();
    globals.get(entity).unwrap().matrix.w_axis.truncate()
}

/// Earth radius away, with a box 5.25 m beside the player: f32 at that radius cannot hold the .25.
#[test]
fn a_rebase_keeps_exact_offsets() {
    let mut resources = world();
    let player = spawn(&mut resources, DVec3::new(0.0, 6_371_007.0, 0.0));
    let beside = spawn(&mut resources, DVec3::new(5.25, 6_371_007.125, 0.0));
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    let origin = FloatingOrigin::default();
    registry
        .get_cpu_mut::<FloatingOrigin>()
        .unwrap()
        .insert(player, origin);
    hierarchy_sync_system(&mut resources);
    transform_propagation_system(&mut resources);

    rebase_origin(&mut resources);

    let cell = resources.get::<WorldOrigin>().unwrap().cell;
    assert_eq!(cell, IVec3::new(0, 3111, 0));
    assert_eq!(global(&resources, player), Vec3::new(0.0, -321.0, 0.0));
    assert_eq!(global(&resources, beside), Vec3::new(5.25, -320.875, 0.0));
    let registry = resources.get::<ComponentRegistry>().unwrap();
    let transforms = registry.get_cpu::<Transform>().unwrap();
    assert_eq!(transforms.get(player).unwrap().position.y, 6_371_007.0);
}

/// Scenes saved before #1972 hold `position` as an f32 `Vec3`; a rejected write would leave every
/// entity at the origin without a word.
#[test]
fn an_f32_position_still_loads() {
    use crate::reflect::{Reflect, ReflectValue};
    let mut transform = Transform::default();
    transform
        .reflect_set("position", ReflectValue::Vec3(Vec3::new(1.5, -2.0, 3.25)))
        .expect("an f32 position was refused");
    assert_eq!(transform.position, DVec3::new(1.5, -2.0, 3.25));
}

fn published(resources: &Resources, entity: Entity) -> IVec3 {
    let registry = resources.get::<ComponentRegistry>().unwrap();
    registry
        .get_cpu::<FloatingOrigin>()
        .unwrap()
        .get(entity)
        .unwrap()
        .cell()
}

/// Play → Stop → Play: Stop's restore writes the authored cell back while the origin stays far.
#[test]
fn a_replay_republishes_the_cell() {
    let mut resources = world();
    let player = spawn(&mut resources, DVec3::new(0.0, 6_371_007.0, 0.0));
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    let origin = FloatingOrigin::default();
    registry
        .get_cpu_mut::<FloatingOrigin>()
        .unwrap()
        .insert(player, origin);
    hierarchy_sync_system(&mut resources);
    transform_propagation_system(&mut resources);
    rebase_origin(&mut resources);
    let far = IVec3::new(0, 3111, 0);

    // Stop: the snapshot restores the authored cell.
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry
        .get_cpu_mut::<FloatingOrigin>()
        .unwrap()
        .insert(player, FloatingOrigin::default());
    rebase_origin(&mut resources);
    assert_eq!(resources.get::<WorldOrigin>().unwrap().cell, far);
    assert_eq!(published(&resources, player), far);
}

/// With nothing to follow (game stopped, or a scene without a FloatingOrigin) the origin goes home.
#[test]
fn no_anchor_returns_home() {
    let mut resources = world();
    resources.insert(WorldOrigin {
        cell: IVec3::new(0, 3111, 0),
    });
    resources.insert(Playing(false));
    rebase_origin(&mut resources);
    assert_eq!(resources.get::<WorldOrigin>().unwrap().cell, IVec3::ZERO);
}
