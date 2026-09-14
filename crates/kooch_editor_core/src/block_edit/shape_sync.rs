//! Keeping a parametric block's geometry in step with its [`BlockShape`] (#1106).

use std::collections::HashMap;

use kooch_blockmesh::{Block, BlockShape};
use kooch_core::resource::Resources;
use kooch_ecs::component::{ComponentNames, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::query::Query;

use crate::actions::EditorAction;

/// The parameters each shaped block was last built from.
#[derive(Default)]
pub(crate) struct BuiltShapes(HashMap<Entity, BlockShape>);

/// Rebuilds every block whose shape parameters changed since they were last seen. A first sighting
/// only records: the file already holds that shape, and rewriting it on load would dirty it.
pub(crate) fn sync_block_shapes(resources: &mut Resources) {
    let mut shaped = Vec::new();
    Query::<(&BlockShape, &Block)>::new(resources).for_each_entity(|entity, (shape, block)| {
        if let Some(source) = block.source {
            shaped.push((entity, *shape, source));
        }
    });

    let mut built = resources.remove::<BuiltShapes>().unwrap_or_default();
    built
        .0
        .retain(|entity, _| shaped.iter().any(|(seen, ..)| seen == entity));
    for (entity, shape, source) in shaped {
        if built
            .0
            .insert(entity, shape)
            .is_some_and(|last| last != shape)
        {
            let mesh = shape.build();
            if super::set_shape(resources, source, &mesh) {
                super::announce(resources, source);
                super::save(resources, entity);
            }
        }
    }
    resources.insert(built);
}

/// Removes the parameters from a block edited by hand, so the next parameter change cannot
/// overwrite the edit. `None` when the block has none.
pub(crate) fn bake(resources: &Resources, entity: Entity) -> Option<EditorAction> {
    resources
        .get::<ComponentRegistry>()?
        .get_cpu::<BlockShape>()?
        .get(entity)?;
    let component = resources
        .get::<ComponentNames>()?
        .id(std::any::type_name::<BlockShape>())?;
    Some(EditorAction::RemoveComponent { entity, component })
}
