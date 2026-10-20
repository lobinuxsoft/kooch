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
/// would overwrite geometry the author had every right to leave diverged. A block edited by hand
/// keeps its work by losing `BlockShape` altogether, which is what the editor's `bake` is for.
///
/// 🔴 Marks nothing unsaved. A derive correcting its own geometry is not an author's edit — the
/// gesture that changed the parameters is, and that is the `SetField` that dirties the scene.
pub fn sync_shapes(resources: &mut Resources) {
    let mut shaped = Vec::new();
    Query::<(&BlockShape, &Block)>::new(resources)
        .for_each_entity(|entity, (shape, _)| shaped.push((entity, *shape)));

    let mut built = resources.remove::<BuiltShapes>().unwrap_or_default();
    built
        .0
        .retain(|entity, _| shaped.iter().any(|(seen, ..)| seen == entity));
    for (entity, shape) in shaped {
        if built
            .0
            .insert(entity, shape)
            .is_some_and(|last| last != shape)
        {
            set_mesh(resources, entity, &shape);
        }
    }
    resources.insert(built);
}

/// Writes the built geometry onto the block. Quiet when the entity has no `Block` to write.
fn set_mesh(resources: &mut Resources, entity: Entity, shape: &BlockShape) {
    let mesh = shape.build();
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(block) = registry
            .get_cpu_mut::<Block>()
            .and_then(|storage| storage.into_mut(entity))
    {
        block.mesh = mesh;
    }
}

#[cfg(test)]
mod tests;
