//! That the geometry follows the parameters, and that loading a scene does not touch it.

use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::query::AccessTracker;

use super::*;
use crate::{Block, BlockMesh, BlockShape, Shape};

/// A world holding one block whose geometry disagrees with its parameters — a scene saved before
/// the derive reached the side that owns the world.
fn world() -> (Resources, Entity) {
    let mut resources = Resources::new();
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());

    let entity = Entity::new(1, 0);
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
        archetypes.register_entity(entity, kooch_ecs::archetype::ArchetypeId::EMPTY);
    }
    add(&mut resources, entity, Block::default());
    add(&mut resources, entity, cube(1.0));
    (resources, entity)
}

fn add<T: kooch_ecs::component::Component>(resources: &mut Resources, entity: Entity, value: T) {
    if let Some(components) = resources.get_mut::<ComponentRegistry>() {
        components.register_cpu::<T>();
        if let Some(storage) = components.get_cpu_mut::<T>() {
            storage.insert(entity, value);
        }
    }
    kooch_ecs::move_entity::gained(resources, entity, std::any::TypeId::of::<T>());
}

fn cube(side: f32) -> BlockShape {
    Shape::Cube {
        size: glam::Vec3::splat(side),
    }
    .into()
}

fn corners(resources: &Resources, entity: Entity) -> usize {
    resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<Block>()?.get(entity))
        .map(|block| block.mesh.positions().len())
        .unwrap_or_default()
}

fn mesh_of(resources: &Resources, entity: Entity) -> BlockMesh {
    resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<Block>()?.get(entity))
        .map(|block| block.mesh.clone())
        .unwrap_or_default()
}

/// 🔴 The step the editor used to take on its own. Red against a world that derives nothing: the
/// parameters move and the geometry stays where the scene left it.
#[test]
fn a_changed_shape_rebuilds() {
    let (mut resources, entity) = world();
    sync_shapes(&mut resources);
    let loaded = mesh_of(&resources, entity);

    if let Some(storage) = resources
        .get_mut::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu_mut::<BlockShape>())
        && let Some(shape) = storage.get_mut(entity)
    {
        *shape = cube(4.0);
    }
    sync_shapes(&mut resources);

    let rebuilt = mesh_of(&resources, entity);
    assert_ne!(
        rebuilt.positions(),
        loaded.positions(),
        "the parameters moved and the geometry did not",
    );
    assert_eq!(rebuilt.positions(), cube(4.0).build().positions());
}

/// A first sighting only records. Rebuilding on load would dirty every block in a level and
/// overwrite geometry the author had every right to leave diverged.
#[test]
fn a_loaded_shape_is_kept() {
    let (mut resources, entity) = world();
    sync_shapes(&mut resources);
    assert_eq!(
        corners(&resources, entity),
        0,
        "a load rebuilt the geometry"
    );
}
