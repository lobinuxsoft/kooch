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
        archetypes.register_at(entity, kooch_ecs::archetype::ArchetypeId::EMPTY, None);
    }
    add(&mut resources, entity, Block::default());
    add(&mut resources, entity, cube(1.0));
    (resources, entity)
}

fn add<T: kooch_ecs::component::Component>(resources: &mut Resources, entity: Entity, value: T) {
    if let Some(components) = resources.get_mut::<ComponentRegistry>() {
        components.register_cpu::<T>();
        if let Some(mut storage) = components.get_cpu_mut::<T>() {
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

    reshape(&mut resources, entity, cube(4.0));
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

/// 🔴 A derive is not an edit. Marking the scene unsaved here put an asterisk on a level nobody
/// had touched, and told the author to save geometry that was already on disk.
#[test]
fn a_rebuild_marks_nothing() {
    let (mut resources, entity) = world();
    let mut manager = kooch_ecs::SceneManager::new();
    let scene = manager.new_scene();
    resources.insert(manager);
    add(&mut resources, entity, kooch_ecs::SceneMember::new(scene));

    sync_shapes(&mut resources);
    reshape(&mut resources, entity, cube(4.0));
    sync_shapes(&mut resources);

    let dirty = resources
        .get::<kooch_ecs::SceneManager>()
        .is_some_and(|manager| manager.any_dirty());
    assert!(!dirty, "the derive marked the scene unsaved");
}

fn reshape(resources: &mut Resources, entity: Entity, shape: BlockShape) {
    if let Some(mut storage) = resources
        .get_mut::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu_mut::<BlockShape>())
        && let Some(held) = storage.get_mut(entity)
    {
        *held = shape;
    }
}
