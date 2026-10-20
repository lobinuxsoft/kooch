//! Deriving a parametric block's geometry from its [`BlockShape`] (#1106, #1567).
//!
//! 🔴 Here and not in the editor. The mesh is a pure function of the parameters, so the side that
//! **owns the world** has to compute it: an editor that derived it into its own mirror sent only
//! the parameters over the wire, and the project's next snapshot handed the old geometry back.

use std::collections::HashMap;

use kooch_core::resource::Resources;
use kooch_ecs::Entity;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::query::Query;

use crate::{Block, BlockShape};

/// The parameters each shaped block was last built from.
#[derive(Debug, Default)]
pub struct BuiltShapes(HashMap<Entity, BlockShape>);

/// Rebuilds every block whose shape parameters changed since they were last seen.
///
/// A first sighting only records: the scene already holds that geometry, and rebuilding it on load
/// would dirty every block in the level. A block edited by hand keeps its work by losing
/// `BlockShape` altogether, which is what the editor's `bake` is for.
pub fn sync_shapes(resources: &mut Resources) {
    let mut shaped = Vec::new();
    Query::<(&BlockShape, &Block)>::new(resources)
        .for_each_entity(|entity, (shape, _)| shaped.push((entity, *shape)));

    let mut built = resources.remove::<BuiltShapes>().unwrap_or_default();
    built
        .0
        .retain(|entity, _| shaped.iter().any(|(seen, ..)| seen == entity));
    let mut rebuilt = Vec::new();
    for (entity, shape) in shaped {
        if built
            .0
            .insert(entity, shape)
            .is_some_and(|last| last != shape)
            && set_mesh(resources, entity, &shape)
        {
            rebuilt.push(entity);
        }
    }
    resources.insert(built);
    for entity in rebuilt {
        touch(resources, entity);
    }
}

/// Writes the built geometry onto the block. `false` when the entity has no `Block` to write.
fn set_mesh(resources: &mut Resources, entity: Entity, shape: &BlockShape) -> bool {
    let mesh = shape.build();
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return false;
    };
    let Some(block) = registry
        .get_cpu_mut::<Block>()
        .and_then(|storage| storage.get_mut(entity))
    else {
        return false;
    };
    block.mesh = mesh;
    true
}

/// Marks the rebuilt block's scene unsaved. Only ever reached when the parameters changed, which is
/// an authored edit — a scene that loads and derives nothing stays clean.
fn touch(resources: &mut Resources, entity: Entity) {
    let Some(scene) = resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<kooch_ecs::SceneMember>()?.get(entity))
        .map(|member| member.scene)
    else {
        return;
    };
    if let Some(manager) = resources.get_mut::<kooch_ecs::SceneManager>() {
        manager.mark_scene_dirty(scene);
    }
}

#[cfg(test)]
mod tests;
