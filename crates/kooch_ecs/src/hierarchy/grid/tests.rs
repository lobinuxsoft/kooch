use glam::{IVec3, Vec3};
use kooch_core::resource::Resources;
use kooch_core::run_state::Playing;

use super::*;
use crate::allocator::EntityAllocator;
use crate::archetype_registry::ArchetypeRegistry;
use crate::commands::Commands;
use crate::hierarchy::{Children, hierarchy_sync_system, transform_propagation_system};
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

fn spawn(resources: &mut Resources, at: Vec3) -> Entity {
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

/// Earth radius away: the player, a box beside it, and a planet anchored at cell zero.
#[test]
fn a_rebase_keeps_relative_poses() {
    let mut resources = world();
    let player = spawn(&mut resources, Vec3::new(0.0, 6_371_007.0, 0.0));
    let beside = spawn(&mut resources, Vec3::new(5.0, 6_371_007.0, 0.0));
    let planet = spawn(&mut resources, Vec3::ZERO);
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    let origin = FloatingOrigin::default();
    registry
        .get_cpu_mut::<FloatingOrigin>()
        .unwrap()
        .insert(player, origin);
    let cell = GridCell::default();
    registry
        .get_cpu_mut::<GridCell>()
        .unwrap()
        .insert(planet, cell);
    hierarchy_sync_system(&mut resources);
    transform_propagation_system(&mut resources);

    rebase_origin(&mut resources);

    let registry = resources.get::<ComponentRegistry>().unwrap();
    assert_eq!(origin_cell(registry), IVec3::new(0, 3111, 0));
    assert_eq!(global(&resources, player), Vec3::new(0.0, -321.0, 0.0));
    assert_eq!(global(&resources, beside), Vec3::new(5.0, -321.0, 0.0));
    assert_eq!(
        global(&resources, planet),
        Vec3::new(0.0, -6_371_328.0, 0.0)
    );
    let shift = resources.get::<OriginShift>().unwrap().cells;
    assert_eq!(shift, IVec3::new(0, 3111, 0));
}
